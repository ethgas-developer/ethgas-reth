//! Loads `ethgas-node` with RPC calls, on a `--dev` chain that it feeds with flashblocks itself.
//! `bench/README.md` has the before/after recipe.
//!
//! - `produce`: serves flashblocks over WebSocket for a `--dev` chain. Each block has 8
//!   flashblocks, 127 transactions in flashblock 0 and 25 in each later one: the shape of the
//!   `ETHGas` builder's feed at a one-second interval. Half the transactions move ether and half
//!   move an ERC-20 balance, each to a new address. The transactions never reach the pool, so
//!   canonical blocks stay empty and pending is dropped when each one lands. With `--stall`, the
//!   producer keeps building blocks on pending, for a node whose canonical tip does not move.
//! - `rpc`: sends one JSON-RPC method from `--workers` tasks for `--seconds` and prints latency
//!   quantiles. On a chain other than `produce`'s, `--token`, `--holder` and `--caller` name the
//!   ERC-20, an account that holds it and the `eth_call` sender.
//!
//! ```text
//! bench_driver produce --rpc http://127.0.0.1:28545 --listen 127.0.0.1:28600 [--interval-ms 250] [--stall] [--blocks N]
//! bench_driver rpc --rpc http://127.0.0.1:28545 --method call|balance|nonce|estimate|simulate|logs|block --tag pending|latest [--workers 16] [--seconds 20] [--token 0x.. --holder 0x.. --caller 0x..]
//! ```

use std::{
    borrow::Cow,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use alloy_consensus::{TxType, transaction::SignerRecoverable};
use alloy_eips::{BlockNumberOrTag, Encodable2718};
use alloy_primitives::{
    Address, B256, Bytes, Log, TxKind, U256, keccak256, map::foldhash::HashMap,
};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::Block;
use alloy_sol_types::{SolCall, SolConstructor, SolEvent};
use ethgas_node_runner::test_utils::MockERC20;
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::{Result, eyre};
use futures_util::{SinkExt, StreamExt};
use reth_ethereum_primitives::{Receipt, TransactionSigned};
use reth_transaction_pool::test_utils::TransactionBuilder;
use serde_json::{Value, json};
use tokio::{
    net::TcpListener,
    sync::broadcast::{self, error::RecvError},
    time::sleep,
};
use tokio_tungstenite::{accept_async, tungstenite::Message};

/// The `--dev` chain's 20 funded accounts (mnemonic `test ... junk`, as in reth's `dev.json`).
const DEV_KEYS: [&str; 20] = [
    "ac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80",
    "59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d",
    "5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a",
    "7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6",
    "47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a",
    "8b3a350cf5c34c9194ca85829a2df0ec3153be0318b5e2d3348e872092edffba",
    "92db14e403b83dfe3df233f83dfa3a0d7096f21ca9b0d6d6b8d88b2b4ec1564e",
    "4bbbf85ce3377467afe5d46f804f221813b2bb87f24d81f60f1fcdbf7cbf4356",
    "dbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97",
    "2a871d0798f97d79848a013d4936a73bf4cc922c825d33c1cf7073dff6d409c6",
    "f214f2b2cd398c806f84e317254e0f0b801d0643303237d97a22a48e01628897",
    "701b615bbdfb9de65240bc28bd21bbc0d996645a3dd57e7b12bc2bdf6f192c82",
    "a267530f49f8280200edf313ee7af6b827f2a8bce2897751d06a843f644967b1",
    "47c99abed3324a2707c28affff1267e45918ec8c3f20b8aa892e8b065d2942dd",
    "c526ee95bf44d8fc405a158bb884d9d1238d99f0612e9f33d006bb0789009aaa",
    "8166f546bab6da521a8369cab06c5d2b9e46670292d85c875ee9ec20e84ffb61",
    "ea6c44ac03bff858b476bba40716402b03e41b8e97e276d1baec7c37d42484a0",
    "689af8efa8c651a91ad287602527f3af2fe9f6501a7ac4b061667b5a93e037fd",
    "de9be858da4a475276426320d5e9262ecfc3ba460bfac56360bfa6c4c28b4ee0",
    "df57089febbacf7ba0bc227dafbffa9fc08a93fdc68e1e42411a14efcf23656e",
];
/// Accounts `0..9` send in even blocks and `9..18` in odd ones, so a block is always valid on a
/// pending block before it. Account 18 calls, account 19 deploys the token.
const SENDERS_PER_SET: usize = 9;
const CALLER: usize = 18;
const DEPLOYER: usize = 19;

const FLASHBLOCKS_PER_BLOCK: usize = 8;
const FIRST_FLASHBLOCK_TXS: usize = 127;
const LATER_FLASHBLOCK_TXS: usize = 25;
const SLOT_SECONDS: u64 = 12;
/// What the receipts claim. The node checks only the status against its own execution.
const ETH_TRANSFER_GAS: u64 = 21_000;
const TOKEN_TRANSFER_GAS: u64 = 60_000;
/// Enough for Amsterdam too, which charges a new account 120 state bytes at 1,530 gas each
/// (EIP-8037) and a new storage slot 64 of them.
const TRANSFER_GAS_LIMIT: u64 = 500_000;

fn arg(name: &str) -> Option<String> {
    let args: Vec<String> = std::env::args().collect();
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

fn flag(name: &str) -> bool {
    std::env::args().any(|a| a == name)
}

fn arg_or<T: std::str::FromStr>(name: &str, default: T) -> T {
    arg(name).and_then(|value| value.parse().ok()).unwrap_or(default)
}

fn address_arg(name: &str, default: Address) -> Result<Address> {
    arg(name).map_or(Ok(default), |value| value.parse().map_err(|err| eyre!("{name}: {err}")))
}

#[derive(Clone, Debug)]
struct DevAccount {
    key: B256,
    address: Address,
}

fn dev_accounts() -> Vec<DevAccount> {
    DEV_KEYS
        .iter()
        .map(|hex| {
            let key: B256 = hex.parse().expect("a key");
            let probe = TransactionBuilder::default().signer(key).chain_id(1).into_eip1559();
            DevAccount { key, address: probe.recover_signer().expect("a signer") }
        })
        .collect()
}

fn sign(account: &DevAccount, chain_id: u64, nonce: u64) -> TransactionBuilder {
    TransactionBuilder::default()
        .signer(account.key)
        .chain_id(chain_id)
        .nonce(nonce)
        .max_fee_per_gas(3_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000)
}

fn wire(tx: &TransactionSigned) -> Bytes {
    tx.encoded_2718().into()
}

#[tokio::main(flavor = "multi_thread")]
async fn main() -> Result<()> {
    match std::env::args().nth(1).as_deref() {
        Some("produce") => produce().await,
        Some("rpc") => rpc().await,
        _ => Err(eyre!("usage: bench_driver produce|rpc ... (see the file header)")),
    }
}

// ---------------------------------------------------------------------------------------------
// produce

/// One transaction as a flashblock carries it.
struct WireTx {
    bytes: Bytes,
    hash: B256,
    gas: u64,
    logs: Vec<Log>,
}

struct Producer {
    chain_id: u64,
    token: Address,
    accounts: Vec<DevAccount>,
    /// Canonical nonces, read once: the producer's transactions never become canonical.
    canonical_nonces: Vec<u64>,
    next_nonces: Vec<u64>,
    issued: u64,
}

impl Producer {
    fn transaction(&mut self, sender: usize) -> WireTx {
        self.issued += 1;
        let recipient = Address::from_word(keccak256(self.issued.to_be_bytes()));
        let account = self.accounts[sender].clone();
        let builder = sign(&account, self.chain_id, self.next_nonces[sender]);
        self.next_nonces[sender] += 1;

        let (tx, gas, logs) = if self.issued.is_multiple_of(2) {
            let tx = builder
                .to(recipient)
                .value(1_000_000_000)
                .gas_limit(TRANSFER_GAS_LIMIT)
                .into_eip1559();
            (tx, ETH_TRANSFER_GAS, Vec::new())
        } else {
            let amount = U256::from(1_000u64);
            let input = MockERC20::transferCall { to: recipient, amount }.abi_encode();
            let log = Log::new_unchecked(
                self.token,
                vec![
                    MockERC20::Transfer::SIGNATURE_HASH,
                    account.address.into_word(),
                    recipient.into_word(),
                ],
                Bytes::from(amount.to_be_bytes::<32>().to_vec()),
            );
            let tx =
                builder.to(self.token).gas_limit(TRANSFER_GAS_LIMIT).input(input).into_eip1559();
            (tx, TOKEN_TRANSFER_GAS, vec![log])
        };
        WireTx { bytes: wire(&tx), hash: *tx.tx_hash(), gas, logs }
    }

    fn block_flashblocks(
        &mut self,
        number: u64,
        parent_hash: B256,
        timestamp: u64,
        gas_limit: u64,
    ) -> Vec<FlashBlock> {
        let first_sender = if number.is_multiple_of(2) { 0 } else { SENDERS_PER_SET };
        let mut turn = 0;
        let mut cumulative_gas_used = 0;
        (0..FLASHBLOCKS_PER_BLOCK)
            .map(|index| {
                let count = if index == 0 { FIRST_FLASHBLOCK_TXS } else { LATER_FLASHBLOCK_TXS };
                let mut receipts = HashMap::default();
                let mut transactions = Vec::with_capacity(count);
                for _ in 0..count {
                    let tx = self.transaction(first_sender + turn % SENDERS_PER_SET);
                    turn += 1;
                    cumulative_gas_used += tx.gas;
                    transactions.push(tx.bytes);
                    receipts.insert(
                        tx.hash,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true,
                            cumulative_gas_used,
                            logs: tx.logs,
                        },
                    );
                }
                FlashBlock {
                    payload_id: PayloadId::default(),
                    index: index as u64,
                    base: (index == 0).then(|| ExecutionPayloadBaseV1 {
                        parent_beacon_block_root: B256::ZERO,
                        parent_hash,
                        fee_recipient: Address::with_last_byte(0xfe),
                        prev_randao: B256::with_last_byte(1),
                        block_number: number,
                        gas_limit,
                        timestamp,
                        extra_data: Bytes::new(),
                        base_fee_per_gas: U256::from(1_000_000_000u64),
                        slot_number: Some(timestamp),
                    }),
                    diff: ExecutionPayloadFlashblockDeltaV1 {
                        gas_used: cumulative_gas_used,
                        transactions,
                        ..Default::default()
                    },
                    metadata: Metadata {
                        block_number: number,
                        receipts,
                        new_account_balances: HashMap::default(),
                        inclusion_fee: None,
                    },
                }
            })
            .collect()
    }
}

async fn nonce(provider: &RootProvider, address: Address) -> Result<u64> {
    Ok(provider.get_transaction_count(address).await?)
}

async fn head(provider: &RootProvider) -> Result<Block> {
    provider.get_block_by_number(BlockNumberOrTag::Latest).await?.ok_or_else(|| eyre!("no head"))
}

/// Deploys the token from the deployer account and mints to every sender, through the pool.
async fn deploy_token(
    provider: &RootProvider,
    accounts: &[DevAccount],
    chain_id: u64,
) -> Result<Address> {
    let deployer = &accounts[DEPLOYER];
    let start = nonce(provider, deployer.address).await?;
    let token = deployer.address.create(start);
    if !provider.get_code_at(token).await?.is_empty() {
        return Ok(token);
    }

    let constructor = MockERC20::constructorCall {
        _name: "Bench".to_string(),
        _symbol: "BNCH".to_string(),
        _decimals: 18,
    };
    let code = [MockERC20::BYTECODE.to_vec(), constructor.abi_encode()].concat();
    let mut deploy = sign(deployer, chain_id, start).gas_limit(5_000_000).input(code);
    deploy.to = TxKind::Create;
    let _ = provider.send_raw_transaction(&wire(&deploy.into_eip1559())).await?;

    for (i, holder) in accounts.iter().enumerate().take(2 * SENDERS_PER_SET) {
        let mint = MockERC20::mintCall {
            to: holder.address,
            value: U256::from(10u64).pow(U256::from(30)),
        }
        .abi_encode();
        let tx = sign(deployer, chain_id, start + 1 + i as u64)
            .to(token)
            .gas_limit(200_000)
            .input(mint)
            .into_eip1559();
        let _ = provider.send_raw_transaction(&wire(&tx)).await?;
    }

    let deadline = Instant::now() + Duration::from_secs(120);
    let done = start + 1 + 2 * SENDERS_PER_SET as u64;
    while nonce(provider, deployer.address).await? < done {
        if Instant::now() > deadline {
            return Err(eyre!("the token deployment was not mined in 120 s"));
        }
        sleep(Duration::from_millis(200)).await;
    }
    Ok(token)
}

async fn serve(listen: String, feed: broadcast::Sender<Arc<str>>) -> Result<()> {
    let listener = TcpListener::bind(&listen).await?;
    eprintln!("serving flashblocks on ws://{listen}");
    loop {
        let (stream, peer) = listener.accept().await?;
        let mut messages = feed.subscribe();
        tokio::spawn(async move {
            let Ok(ws) = accept_async(stream).await else { return };
            eprintln!("subscriber connected: {peer}");
            let (mut write, mut read) = ws.split();
            // Reading answers the subscriber's pings; the flush below sends the pongs.
            tokio::spawn(async move { while let Some(Ok(_)) = read.next().await {} });
            let mut flush = tokio::time::interval(Duration::from_millis(250));
            loop {
                tokio::select! {
                    message = messages.recv() => match message {
                        Ok(text) => {
                            if write.send(Message::text(text.to_string())).await.is_err() {
                                break;
                            }
                        }
                        Err(RecvError::Lagged(_)) => {}
                        Err(RecvError::Closed) => break,
                    },
                    _ = flush.tick() => {
                        if write.flush().await.is_err() {
                            break;
                        }
                    }
                }
            }
            eprintln!("subscriber gone: {peer}");
        });
    }
}

async fn produce() -> Result<()> {
    let rpc: String = arg_or("--rpc", "http://127.0.0.1:28545".to_string());
    let listen: String = arg_or("--listen", "127.0.0.1:28600".to_string());
    let interval = Duration::from_millis(arg_or("--interval-ms", 250));
    let stall = flag("--stall");
    let blocks: u64 = arg_or("--blocks", u64::MAX);

    let provider = RootProvider::new_http(rpc.parse()?);
    let chain_id = provider.get_chain_id().await?;
    let accounts = dev_accounts();
    let token = deploy_token(&provider, &accounts, chain_id).await?;
    eprintln!("token {token}");

    let mut canonical_nonces = Vec::with_capacity(accounts.len());
    for account in &accounts {
        canonical_nonces.push(nonce(&provider, account.address).await?);
    }
    let mut producer = Producer {
        chain_id,
        token,
        accounts,
        next_nonces: canonical_nonces.clone(),
        canonical_nonces,
        issued: 0,
    };

    let (feed, _) = broadcast::channel::<Arc<str>>(4096);
    tokio::spawn(serve(listen, feed.clone()));
    // A node that misses a stall's first block cannot anchor the blocks after it.
    while feed.receiver_count() == 0 {
        sleep(Duration::from_millis(100)).await;
    }

    let mut sent_blocks = 0;
    let mut last_head = 0;
    let first = head(&provider).await?;
    let (mut number, mut parent_hash, mut timestamp) =
        (first.header.number, first.header.hash, first.header.timestamp);
    let gas_limit = first.header.gas_limit;

    while sent_blocks < blocks {
        if !stall {
            let latest = head(&provider).await?;
            if latest.header.number == last_head {
                sleep(Duration::from_millis(10)).await;
                continue;
            }
            last_head = latest.header.number;
            (number, parent_hash, timestamp) =
                (latest.header.number, latest.header.hash, latest.header.timestamp);
            // Pending was dropped when this block landed, so nonces start from canonical again.
            producer.next_nonces.clone_from(&producer.canonical_nonces);
        }

        let block = number + 1;
        let started = Instant::now();
        let flashblocks =
            producer.block_flashblocks(block, parent_hash, timestamp + SLOT_SECONDS, gas_limit);
        let built = started.elapsed();
        for flashblock in flashblocks {
            let text: Arc<str> = serde_json::to_string(&flashblock)?.into();
            let _ = feed.send(text);
            sleep(interval).await;
        }
        eprintln!(
            "block {block}: {FLASHBLOCKS_PER_BLOCK} flashblocks, signed in {} ms",
            built.as_millis()
        );

        sent_blocks += 1;
        if stall {
            (number, parent_hash, timestamp) =
                (block, keccak256(block.to_be_bytes()), timestamp + SLOT_SECONDS);
        }
    }
    sleep(Duration::from_secs(2)).await;
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// rpc

fn quantile(sorted: &[u64], q: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[((sorted.len() - 1) as f64 * q).round() as usize]
}

async fn rpc() -> Result<()> {
    let url: String = arg_or("--rpc", "http://127.0.0.1:28545".to_string());
    let method: String = arg_or("--method", "call".to_string());
    let tag: String = arg_or("--tag", "pending".to_string());
    let workers: usize = arg_or("--workers", 16);
    let seconds: u64 = arg_or("--seconds", 20);

    let provider: RootProvider = RootProvider::new_http(url.parse()?);
    let accounts = dev_accounts();
    // By default the token is the deployer's first contract, which `produce` deploys on a fresh
    // chain.
    let token = address_arg("--token", accounts[DEPLOYER].address.create(0))?;
    let caller = address_arg("--caller", accounts[CALLER].address)?;
    let holder = address_arg("--holder", accounts[0].address)?;

    let (name, params): (&'static str, Value) = match method.as_str() {
        "call" => (
            "eth_call",
            json!([{
                "from": caller, "to": token,
                "data": Bytes::from(MockERC20::balanceOfCall(holder).abi_encode())
            }, tag]),
        ),
        "estimate" => (
            "eth_estimateGas",
            json!([{
                "from": holder, "to": token,
                "data": Bytes::from(MockERC20::transferCall { to: caller, amount: U256::from(1) }.abi_encode())
            }, tag]),
        ),
        "simulate" => (
            "eth_simulateV1",
            json!([{ "blockStateCalls": [{ "calls": [{
                "from": holder, "to": token,
                "data": Bytes::from(MockERC20::transferCall { to: caller, amount: U256::from(1) }.abi_encode())
            }]}]}, tag]),
        ),
        "balance" => ("eth_getBalance", json!([holder, tag])),
        "nonce" => ("eth_getTransactionCount", json!([holder, tag])),
        "logs" => ("eth_getLogs", json!([{ "fromBlock": tag, "toBlock": tag, "address": token }])),
        "block" => ("eth_getBlockByNumber", json!([tag, false])),
        other => return Err(eyre!("unknown method {other}")),
    };

    let first: Value = provider.raw_request(Cow::Borrowed(name), params.clone()).await?;
    let shown = first.to_string();
    // The logs, or the block's transaction hashes: what a `pending` and a `latest` answer hold.
    let items = first
        .as_array()
        .or_else(|| first.get("transactions").and_then(Value::as_array))
        .map_or(0, Vec::len);
    eprintln!(
        "{name} {tag} first answer ({} bytes, {items} items): {}",
        shown.len(),
        &shown[..shown.len().min(160)]
    );

    let deadline = Instant::now() + Duration::from_secs(seconds);
    let errors = Arc::new(AtomicU64::new(0));
    let mut tasks = Vec::with_capacity(workers);
    for _ in 0..workers {
        let provider = provider.clone();
        let params = params.clone();
        let errors = Arc::clone(&errors);
        tasks.push(tokio::spawn(async move {
            let mut latencies = Vec::new();
            while Instant::now() < deadline {
                let started = Instant::now();
                let answer: Result<Value, _> =
                    provider.raw_request(Cow::Borrowed(name), params.clone()).await;
                latencies.push(started.elapsed().as_micros() as u64);
                if answer.is_err() {
                    errors.fetch_add(1, Ordering::Relaxed);
                }
            }
            latencies
        }));
    }
    let mut latencies = Vec::new();
    for task in tasks {
        latencies.extend(task.await?);
    }
    latencies.sort_unstable();
    println!(
        "{name} {tag} workers={workers} seconds={seconds} requests={} rps={:.0} errors={} p50_us={} p90_us={} p99_us={} p999_us={} max_us={}",
        latencies.len(),
        latencies.len() as f64 / seconds as f64,
        errors.load(Ordering::Relaxed),
        quantile(&latencies, 0.5),
        quantile(&latencies, 0.9),
        quantile(&latencies, 0.99),
        quantile(&latencies, 0.999),
        latencies.last().copied().unwrap_or_default(),
    );
    Ok(())
}
