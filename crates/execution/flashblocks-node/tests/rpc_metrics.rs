//! The `rpc_*` counters count the flashblocks path of each override, and the fee counters count
//! which source answered.
//!
//! One test, on purpose. `Metrics::default()` binds every counter to whatever recorder is global
//! at its first call in the process and caches the result, so the recorder must be installed
//! before the harness builds the flashblocks state. reth's CLI installs it before it runs the
//! node command; this test does the same before it launches the node.

use std::time::Duration;

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_primitives::{Address, B256, Bytes, TxHash, U256, address, b256, bytes};
use alloy_provider::Provider;
use alloy_rpc_types::simulate::{SimBlock, SimulatePayload};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::{Filter, TransactionRequest};
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::Account;
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, InclusionFee, Metadata,
};
use eyre::Result;
use futures_util::{SinkExt, StreamExt};
use reth_ethereum_primitives::Receipt;
use reth_node_metrics::recorder::install_prometheus_recorder;
use serde_json::{Value, json};
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, connect_async, tungstenite::Message};

/// Alice sends 50 ETH to `TRANSFER_ETH_RECIPIENT` at nonce 0.
const TRANSFER_ETH_TX: Bytes = bytes!(
    "0x02f86b0180806482520894deadbeefdeadbeefdeadbeefdeadbeefdeadbeef8902b5e3af16b188000080c001a0c18767bf03c514933cfec05f2c9a354bf4e8eaafe2e4e7c86836bfc0fb62ad42a02b291b32c588337b7b45420076433157a440bb97afebb154988986527a6ef535"
);
const TRANSFER_ETH_HASH: TxHash =
    b256!("0x706bbbf402a4f55831d250c77be8f368e16d9b63df9d58561cea8d1f2b59030b");
const TRANSFER_ETH_RECIPIENT: Address = address!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

/// The value of the counter whose name ends with `suffix`, read from reth's Prometheus recorder.
/// reth installs that recorder at node launch; the call here returns the same handle.
fn counter_value(suffix: &str) -> u64 {
    let rendered = install_prometheus_recorder().handle().render();
    rendered
        .lines()
        .filter_map(|line| line.split_once(' '))
        .find(|(name, _)| name.ends_with(suffix))
        .and_then(|(_, value)| value.trim().parse::<f64>().ok())
        .map_or(0, |value| value as u64)
}

/// The base flashblock of block 1: the transfer, with a builder balance for the recipient.
fn base_payload(inclusion_fee: Option<InclusionFee>) -> FlashBlock {
    let mut receipts = alloy_primitives::map::foldhash::HashMap::default();
    receipts.insert(
        TRANSFER_ETH_HASH,
        Receipt {
            tx_type: alloy_consensus::TxType::Eip1559,
            success: true,
            cumulative_gas_used: 21_000,
            logs: vec![],
        },
    );
    let mut new_account_balances = alloy_primitives::map::foldhash::HashMap::default();
    new_account_balances
        .insert(TRANSFER_ETH_RECIPIENT, U256::from(50_000_000_000_000_000_000_u128));
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index: 0,
        base: Some(ExecutionPayloadBaseV1 {
            parent_beacon_block_root: B256::ZERO,
            parent_hash: B256::ZERO,
            fee_recipient: Address::ZERO,
            prev_randao: B256::ZERO,
            block_number: 1,
            gas_limit: 30_000_000,
            timestamp: 0,
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::ZERO,
        }),
        diff: ExecutionPayloadFlashblockDeltaV1 {
            transactions: vec![TRANSFER_ETH_TX],
            ..Default::default()
        },
        metadata: Metadata { block_number: 1, receipts, new_account_balances, inclusion_fee },
    }
}

/// Subscribes to a kind over a raw WebSocket and returns the subscription id.
async fn ws_subscribe(
    ws: &mut WebSocketStream<MaybeTlsStream<TcpStream>>,
    id: u64,
    params: Value,
) -> Result<String> {
    ws.send(Message::Text(
        json!({"jsonrpc": "2.0", "id": id, "method": "eth_subscribe", "params": params})
            .to_string()
            .into(),
    ))
    .await?;
    let response = ws.next().await.expect("a subscription response")?;
    let sub: Value = serde_json::from_str(response.to_text()?)?;
    assert_eq!(sub["id"], id);
    Ok(sub["result"].as_str().expect("subscription id expected").to_string())
}

/// Runs `call` and returns how much the `reth_flashblocks_*` counter grew. Other tests in this
/// process only add to a counter, so growth is a lower bound.
async fn growth<F>(counter: &str, call: F) -> u64
where
    F: Future<Output = Result<()>>,
{
    let suffix = format!("flashblocks_{counter}");
    let before = counter_value(&suffix);
    call.await.unwrap_or_else(|err| panic!("{counter}: {err}"));
    counter_value(&suffix).saturating_sub(before)
}

#[tokio::test]
async fn every_flashblocks_path_increments_its_counter() -> Result<()> {
    install_prometheus_recorder();
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();
    let client = harness.rpc_client()?;
    let alice = Account::Alice.address();
    harness.send_flashblock(base_payload(None)).await?;

    let transfer = TransactionRequest::default().from(alice).to(Account::Bob.address());

    assert_eq!(
        growth("rpc_get_block_by_number", async {
            provider.get_block_by_number(BlockNumberOrTag::Pending).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_block_by_number", async {
            provider.get_block_by_number(BlockNumberOrTag::Latest).await?;
            Ok(())
        })
        .await,
        0,
        "the counter counts the flashblocks path only"
    );
    assert_eq!(
        growth("rpc_get_block_transaction_count_by_number", async {
            client
                .request::<_, Option<U256>>("eth_getBlockTransactionCountByNumber", ("pending",))
                .await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_block_receipts", async {
            provider.get_block_receipts(BlockId::pending()).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_block_receipts", async {
            provider.get_block_receipts(BlockId::latest()).await?;
            Ok(())
        })
        .await,
        0
    );
    assert_eq!(
        growth("rpc_get_transaction_by_block_number_and_index", async {
            provider
                .get_transaction_by_block_number_and_index(BlockNumberOrTag::Pending, 0)
                .await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_transaction_by_hash", async {
            provider.get_transaction_by_hash(TRANSFER_ETH_HASH).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_transaction_receipt", async {
            provider.get_transaction_receipt(TRANSFER_ETH_HASH).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_balance", async {
            provider.get_balance(TRANSFER_ETH_RECIPIENT).pending().await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_balance", async {
            provider.get_balance(TRANSFER_ETH_RECIPIENT).await?;
            Ok(())
        })
        .await,
        0
    );
    assert_eq!(
        growth("rpc_get_transaction_count", async {
            provider.get_transaction_count(alice).pending().await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_call", async {
            client.request::<_, Bytes>("eth_call", (transfer.clone(), "pending")).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_estimate_gas", async {
            client.request::<_, U256>("eth_estimateGas", (transfer.clone(), "pending")).await?;
            Ok(())
        })
        .await,
        1
    );
    let simulation = SimulatePayload {
        block_state_calls: vec![SimBlock {
            calls: vec![transfer.clone()],
            block_overrides: None,
            state_overrides: None,
        }],
        trace_transfers: false,
        validation: false,
        return_full_transactions: false,
    };
    assert_eq!(
        growth("rpc_simulate_v1", async {
            provider.simulate(&simulation).block_id(BlockNumberOrTag::Pending.into()).await?;
            Ok(())
        })
        .await,
        1
    );
    assert_eq!(
        growth("rpc_get_logs", async {
            provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
            Ok(())
        })
        .await,
        1
    );

    // No fee is held, so the fallback counter grows.
    assert_eq!(
        growth("rpc_inclusion_fee_fallback", async {
            client.request::<_, Value>("ethgas_inclusionPriorityFee", ()).await?;
            Ok(())
        })
        .await,
        1
    );
    harness
        .send_flashblock(base_payload(Some(InclusionFee { priority_fee: U256::from(7) })))
        .await?;
    assert_eq!(
        growth("rpc_inclusion_fee_builder", async {
            provider.get_gas_price().await?;
            Ok(())
        })
        .await,
        1
    );

    // `eth_sendRawTransactionSync`: the transfer is in the snapshot already, so the flashblock
    // counter grows. A transaction no flashblock carries ends with the timeout.
    assert_eq!(
        growth("rpc_send_raw_transaction_sync_flashblock", async {
            client
                .request::<_, Value>("eth_sendRawTransactionSync", (TRANSFER_ETH_TX, 1_000))
                .await?;
            Ok(())
        })
        .await,
        1
    );
    let (unknown_tx, _) = Account::Alice
        .sign_txn_request(
            TransactionRequest::default()
                .to(Account::Bob.address())
                .value(U256::from(1))
                .gas_limit(21_000)
                .nonce(7),
        )
        .expect("should be able to sign the transfer");
    assert_eq!(
        growth("rpc_send_raw_transaction_sync_timeout", async {
            client
                .request::<_, Value>("eth_sendRawTransactionSync", (unknown_tx, 50))
                .await
                .expect_err("no flashblock carries the transaction");
            Ok(())
        })
        .await,
        1
    );

    // Subscriptions count once per kind when they open.
    let (mut ws, _) = connect_async(&harness.ws_url()).await?;
    for (id, params, counter) in [
        (1, json!(["newFlashblocks"]), "subscriptions_new_flashblocks"),
        (2, json!(["pendingLogs", {}]), "subscriptions_pending_logs"),
        (3, json!(["newFlashblockTransactions"]), "subscriptions_new_flashblock_transactions"),
    ] {
        assert_eq!(
            growth(counter, async {
                ws_subscribe(&mut ws, id, params).await?;
                Ok(())
            })
            .await,
            1,
            "{counter}"
        );
    }

    // A canonical block that carries the transaction ends the wait with the canonical receipt.
    // The call submits Bob's transfer to the pool; the block built from the pool carries it.
    let (bob_tx, _) = Account::Bob
        .sign_txn_request(
            TransactionRequest::default().to(alice).value(U256::from(1)).gas_limit(21_000).nonce(0),
        )
        .expect("should be able to sign the transfer");
    let genesis_hash = harness.latest_block().hash();
    let sync_client = client.clone();
    let sync_call = tokio::spawn(async move {
        sync_client.request::<_, Value>("eth_sendRawTransactionSync", (bob_tx, 10_000)).await
    });
    assert_eq!(
        growth("rpc_send_raw_transaction_sync_canonical", async {
            tokio::time::sleep(Duration::from_millis(300)).await;
            let block = harness.submit_block_from_transactions(vec![]).await?;
            harness.engine().update_forkchoice(genesis_hash, block, None).await?;
            sync_call.await??;
            Ok(())
        })
        .await,
        1
    );

    Ok(())
}
