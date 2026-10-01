//! Integration tests covering the Flashblocks RPC surface area.
//!
//! These tests exercise the flashblocks-extended RPC endpoints (pending block,
//! pending balance, pending transaction receipt, `eth_call` with flashblock state, etc.)
//! by launching a full local Ethereum node with the flashblocks test extension.

use std::{str::FromStr, time::Duration};

use DoubleCounter::DoubleCounterInstance;
use alloy_consensus::constants::EMPTY_WITHDRAWALS;
use alloy_eips::{BlockId, BlockNumberOrTag, eip7685::EMPTY_REQUESTS_HASH};
use alloy_primitives::{
    Address, B256, Bytes, Log as PrimitiveLog, LogData, TxHash, U256, address, b256, bytes,
    keccak256, map::foldhash::HashMap,
};
use alloy_provider::{Provider, network::TransactionResponse};
use alloy_rpc_types::simulate::{SimBlock, SimulatePayload};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::{
    Filter, TransactionInput, TransactionRequest,
    error::EthRpcErrorCode,
    state::{AccountOverride, StateOverride},
};
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, BLOCK_TIME_SECONDS, DoubleCounter};
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use futures_util::{SinkExt, StreamExt};
use reth_ethereum_primitives::Receipt;
use serde_json::json;
use tokio_tungstenite::{connect_async, tungstenite::Message};

// Test constants
const TEST_ADDRESS: Address = address!("0x1234567890123456789012345678901234567890");
const PENDING_BALANCE: u64 = 4660;

// Test parent beacon block root for flashblock tests
const TEST_PARENT_BEACON_BLOCK_ROOT: B256 =
    b256!("0x1234567890abcdef1234567890abcdef1234567890abcdef1234567890abcdef");

// A pre-signed EIP-1559 transfer transaction (Alice -> 0xdead..beef, 50 ETH)
// Sender: Alice (0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266)
const TRANSFER_ETH_TX: Bytes = bytes!(
    "0x02f86b0180806482520894deadbeefdeadbeefdeadbeefdeadbeefdeadbeef8902b5e3af16b188000080c001a0c18767bf03c514933cfec05f2c9a354bf4e8eaafe2e4e7c86836bfc0fb62ad42a02b291b32c588337b7b45420076433157a440bb97afebb154988986527a6ef535"
);
const TRANSFER_ETH_HASH: TxHash =
    b256!("0x706bbbf402a4f55831d250c77be8f368e16d9b63df9d58561cea8d1f2b59030b");
/// Receives the 50 ETH of `TRANSFER_ETH_TX`
const TRANSFER_ETH_RECIPIENT: Address = address!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");
const PROBE_ADDRESS: Address = address!("0x0000000000000000000000000000000000000abc");

struct TestSetup {
    harness: FlashblocksHarness,
    txn_details: TransactionDetails,
}

struct TransactionDetails {
    counter_deployment_tx: Bytes,
    counter_address: Address,

    counter_increment_tx: Bytes,

    counter_increment2_tx: Bytes,

    alice_eth_transfer_tx: Bytes,
    alice_eth_transfer_hash: TxHash,

    // Balance transfer for balance test
    balance_transfer_tx: Bytes,
}

impl TestSetup {
    async fn new() -> Result<Self> {
        Self::with_harness(FlashblocksHarness::new().await?)
    }

    async fn manual_canonical() -> Result<Self> {
        Self::with_harness(FlashblocksHarness::manual_canonical().await?)
    }

    fn with_harness(harness: FlashblocksHarness) -> Result<Self> {
        let provider = harness.provider();
        let deployer = Account::Deployer;
        let alice = Account::Alice;
        let bob = Account::Bob;

        // DoubleCounter deployment at nonce 0
        let (counter_deployment_tx, counter_address, _) = deployer
            .create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)
            .expect("should be able to sign DoubleCounter deployment txn");
        let counter = DoubleCounterInstance::new(counter_address, provider);
        let (increment1_tx, _) = deployer
            .sign_txn_request(counter.increment().into_transaction_request().nonce(1))
            .expect("should be able to sign increment() txn");
        let (increment2_tx, _) = deployer
            .sign_txn_request(counter.increment2().into_transaction_request().nonce(2))
            .expect("should be able to sign increment2() txn");

        // Alice's ETH transfer at nonce 1 (TRANSFER_ETH_TX in the first flashblock already consumed
        // Alice's nonce 0; block 2 stacks on block 1's pending state). The value is not asserted,
        // so it is kept comfortably below Alice's remaining balance.
        let (eth_transfer_tx, eth_transfer_hash) = alice
            .sign_txn_request(
                TransactionRequest::default()
                    .to(bob.address())
                    .value(U256::from_str("1000000000000000000").unwrap())
                    .gas_limit(100_000)
                    .nonce(1),
            )
            .expect("should be able to sign eth transfer txn");

        // Balance transfer: alice sends PENDING_BALANCE wei to TEST_ADDRESS at nonce 2
        let (balance_transfer_tx, _) = alice
            .sign_txn_request(
                TransactionRequest::default()
                    .to(TEST_ADDRESS)
                    .value(U256::from(PENDING_BALANCE))
                    .gas_limit(21_000)
                    .nonce(2),
            )
            .expect("should be able to sign balance transfer txn");

        let txn_details = TransactionDetails {
            counter_deployment_tx,
            counter_address,
            counter_increment_tx: increment1_tx,
            counter_increment2_tx: increment2_tx,
            alice_eth_transfer_tx: eth_transfer_tx,
            alice_eth_transfer_hash: eth_transfer_hash,
            balance_transfer_tx,
        };

        Ok(Self { harness, txn_details })
    }

    fn create_first_payload(&self) -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 0,
            base: Some(ExecutionPayloadBaseV1 {
                parent_beacon_block_root: TEST_PARENT_BEACON_BLOCK_ROOT,
                parent_hash: B256::default(),
                fee_recipient: Address::ZERO,
                prev_randao: B256::default(),
                block_number: 1,
                gas_limit: 30_000_000,
                timestamp: 0,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::ZERO,
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                blob_gas_used: 0,
                transactions: vec![TRANSFER_ETH_TX],
                ..Default::default()
            },
            metadata: Metadata {
                block_number: 1,
                receipts: {
                    let mut receipts = HashMap::default();
                    receipts.insert(
                        TRANSFER_ETH_HASH,
                        Receipt {
                            tx_type: alloy_consensus::TxType::Eip1559,
                            success: true,
                            cumulative_gas_used: 21000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                new_account_balances: HashMap::default(),
                inclusion_fee: None,
            },
        }
    }

    fn create_second_payload(&self) -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 1,
            base: None,
            diff: ExecutionPayloadFlashblockDeltaV1 {
                state_root: B256::default(),
                receipts_root: B256::default(),
                gas_used: 0,
                block_hash: B256::default(),
                blob_gas_used: 0,
                transactions: vec![
                    self.txn_details.alice_eth_transfer_tx.clone(),
                    self.txn_details.counter_deployment_tx.clone(),
                    self.txn_details.counter_increment_tx.clone(),
                    self.txn_details.counter_increment2_tx.clone(),
                    self.txn_details.balance_transfer_tx.clone(),
                ],
                withdrawals: Vec::new(),
                logs_bloom: Default::default(),
                excess_blob_gas: 0,
            },
            metadata: Metadata {
                block_number: 1,
                // Our flashblock format carries receipts in metadata (unlike base, which rebuilds
                // them from execution). The processor requires a receipt per transaction, keyed by
                // the EIP-2718 tx hash (`keccak256` of the encoded tx for typed transactions).
                receipts: {
                    let mut receipts = HashMap::default();
                    let mut cumulative_gas_used = 0u64;
                    for tx in [
                        &self.txn_details.alice_eth_transfer_tx,
                        &self.txn_details.counter_deployment_tx,
                        &self.txn_details.counter_increment_tx,
                        &self.txn_details.counter_increment2_tx,
                        &self.txn_details.balance_transfer_tx,
                    ] {
                        cumulative_gas_used += 100_000;
                        receipts.insert(
                            alloy_primitives::keccak256(tx.as_ref()),
                            Receipt {
                                tx_type: alloy_consensus::TxType::Eip1559,
                                success: true,
                                cumulative_gas_used,
                                logs: vec![],
                            },
                        );
                    }
                    receipts
                },
                // `eth_getBalance(pending)` is served from `metadata.new_account_balances` (the
                // sequencer-provided balance deltas), so advertise the balance the balance-transfer
                // tx produces for TEST_ADDRESS.
                new_account_balances: {
                    let mut balances = HashMap::default();
                    balances.insert(TEST_ADDRESS, U256::from(PENDING_BALANCE));
                    balances
                },
                inclusion_fee: None,
            },
        }
    }

    fn count1(&self) -> TransactionRequest {
        let counter =
            DoubleCounterInstance::new(self.txn_details.counter_address, self.harness.provider());
        counter.count1().into_transaction_request()
    }

    fn count2(&self) -> TransactionRequest {
        let counter =
            DoubleCounterInstance::new(self.txn_details.counter_address, self.harness.provider());
        counter.count2().into_transaction_request()
    }

    async fn send_flashblock(&self, flashblock: FlashBlock) -> Result<()> {
        self.harness.send_flashblock(flashblock).await
    }

    async fn send_test_payloads(&self) -> Result<()> {
        let base_payload = self.create_first_payload();
        self.send_flashblock(base_payload).await?;

        let second_payload = self.create_second_payload();
        self.send_flashblock(second_payload).await?;

        Ok(())
    }

    /// Submit a raw transaction via `eth_sendRawTransactionSync`, which blocks until the
    /// transaction's receipt is available in (pending) flashblocks state or `timeout_ms` elapses.
    async fn send_raw_transaction_sync(
        &self,
        tx: Bytes,
        timeout_ms: Option<u64>,
    ) -> Result<alloy_rpc_types_eth::TransactionReceipt> {
        let client = self.harness.rpc_client()?;
        let receipt = client
            .request::<_, alloy_rpc_types_eth::TransactionReceipt>(
                "eth_sendRawTransactionSync",
                (tx, timeout_ms),
            )
            .await?;
        Ok(receipt)
    }
}

#[tokio::test]
async fn test_get_pending_block() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();

    let latest_block = provider
        .get_block_by_number(BlockNumberOrTag::Latest)
        .await?
        .expect("latest block expected");
    assert_eq!(latest_block.number(), 0);

    // Querying pending block when it does not exist yet
    let pending_block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("latest block expected");

    assert_eq!(pending_block.number(), latest_block.number());
    assert_eq!(pending_block.hash(), latest_block.hash());

    let base_payload = setup.create_first_payload();
    setup.send_flashblock(base_payload).await?;

    // Query pending block after sending the base payload with one transaction
    let pending_block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block expected");

    assert_eq!(pending_block.number(), 1);
    assert_eq!(pending_block.transactions.hashes().len(), 1); // The transfer transaction

    let second_payload = setup.create_second_payload();
    setup.send_flashblock(second_payload).await?;

    // Query pending block after sending the second payload with transactions
    let block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block expected");

    assert_eq!(block.number(), 1);
    // First flashblock: 1 transfer transaction
    // Second flashblock: 1 alice ETH transfer + 1 counter deploy + 1 counter increment
    // + 1 counter increment2 + 1 balance transfer
    // Total: 1 + 5 = 6 transactions
    assert_eq!(block.transactions.hashes().len(), 6);

    Ok(())
}

#[tokio::test]
async fn test_get_balance_pending() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();

    setup.send_test_payloads().await?;

    let balance = provider.get_balance(TEST_ADDRESS).await?;
    assert_eq!(balance, U256::ZERO);

    let pending_balance = provider.get_balance(TEST_ADDRESS).pending().await?;
    assert_eq!(pending_balance, U256::from(PENDING_BALANCE));
    Ok(())
}

#[tokio::test]
async fn test_get_transaction_by_hash_pending() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();

    assert!(
        provider
            .get_transaction_by_hash(setup.txn_details.alice_eth_transfer_hash)
            .await?
            .is_none()
    );

    setup.send_test_payloads().await?;

    let tx = provider
        .get_transaction_by_hash(setup.txn_details.alice_eth_transfer_hash)
        .await?
        .expect("tx expected");
    assert_eq!(tx.tx_hash(), setup.txn_details.alice_eth_transfer_hash);
    assert_eq!(tx.from(), Account::Alice.address());

    Ok(())
}

#[tokio::test]
async fn test_get_transaction_receipt_pending() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();

    let receipt =
        provider.get_transaction_receipt(setup.txn_details.alice_eth_transfer_hash).await?;
    assert!(receipt.is_none());

    setup.send_test_payloads().await?;

    // The transfer from the first flashblock should have a receipt
    let receipt = provider.get_transaction_receipt(TRANSFER_ETH_HASH).await?;
    assert!(receipt.is_some(), "receipt expected for first flashblock tx");

    Ok(())
}

#[tokio::test]
async fn test_get_transaction_count() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();

    let alice = Account::Alice;

    // Initially Alice's nonce should be 0
    let count = provider.get_transaction_count(alice.address()).await?;
    assert_eq!(count, 0);

    setup.send_test_payloads().await?;

    // After sending payloads with transactions from Alice, pending nonce should increase
    let pending_count = provider.get_transaction_count(alice.address()).pending().await?;
    assert!(pending_count > 0, "pending nonce should be > 0 after flashblock transactions");

    Ok(())
}

#[tokio::test]
async fn test_eth_call() -> Result<()> {
    let setup = TestSetup::new().await?;

    setup.send_test_payloads().await?;

    // DoubleCounter initializes count1/count2 to 1; increment()/increment2() each add 1, so after
    // one call each both read back as 2.
    let count1_result: Bytes =
        setup.harness.rpc_client()?.request("eth_call", (setup.count1(), "pending")).await?;
    let count1 = U256::from_be_slice(&count1_result);
    assert_eq!(count1, U256::from(2));

    let count2_result: Bytes =
        setup.harness.rpc_client()?.request("eth_call", (setup.count2(), "pending")).await?;
    let count2 = U256::from_be_slice(&count2_result);
    assert_eq!(count2, U256::from(2));

    Ok(())
}

#[tokio::test]
async fn test_eth_estimate_gas() -> Result<()> {
    let setup = TestSetup::new().await?;

    setup.send_test_payloads().await?;

    // estimate_gas for a simple call to the deployed counter
    let gas: U256 =
        setup.harness.rpc_client()?.request("eth_estimateGas", (setup.count1(), "pending")).await?;
    assert!(gas > U256::ZERO, "estimated gas should be > 0");

    Ok(())
}

#[tokio::test]
async fn test_get_block_transaction_count_by_number_pending() -> Result<()> {
    let setup = TestSetup::new().await?;

    setup.send_test_payloads().await?;

    let count: Option<U256> = setup
        .harness
        .rpc_client()?
        .request("eth_getBlockTransactionCountByNumber", ("pending",))
        .await?;
    assert!(count.is_some(), "pending block should have transaction count");
    assert!(count.unwrap() > U256::ZERO, "pending block transaction count should be > 0");

    Ok(())
}

// ============================ eth_subscribe (pubsub) ============================

/// Subscribes to a kind over a raw WebSocket and returns the subscription id.
async fn ws_subscribe(
    ws: &mut tokio_tungstenite::WebSocketStream<
        tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>,
    >,
    id: u64,
    params: serde_json::Value,
) -> Result<String> {
    ws.send(Message::Text(
        json!({"jsonrpc": "2.0", "id": id, "method": "eth_subscribe", "params": params})
            .to_string()
            .into(),
    ))
    .await?;
    let response = ws.next().await.unwrap()?;
    let sub: serde_json::Value = serde_json::from_str(response.to_text()?)?;
    assert_eq!(sub["jsonrpc"], "2.0");
    assert_eq!(sub["id"], id);
    Ok(sub["result"].as_str().expect("subscription id expected").to_string())
}

#[tokio::test]
async fn test_eth_subscribe_new_flashblocks() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let subscription_id = ws_subscribe(&mut ws_stream, 1, json!(["newFlashblocks"])).await?;

    setup.send_flashblock(setup.create_first_payload()).await?;

    let notification = ws_stream.next().await.unwrap()?;
    let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
    assert_eq!(notif["method"], "eth_subscription");
    assert_eq!(notif["params"]["subscription"], subscription_id);

    let block = &notif["params"]["result"];
    assert_eq!(block["number"], "0x1");
    assert!(block["hash"].is_string());
    assert!(block["parentHash"].is_string());
    assert!(block["transactions"].is_array());
    assert_eq!(block["transactions"].as_array().unwrap().len(), 1);

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_multiple_flashblocks() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let subscription_id = ws_subscribe(&mut ws_stream, 1, json!(["newFlashblocks"])).await?;

    setup.send_flashblock(setup.create_first_payload()).await?;
    let notif1 = ws_stream.next().await.unwrap()?;
    let notif1: serde_json::Value = serde_json::from_str(notif1.to_text()?)?;
    assert_eq!(notif1["params"]["subscription"], subscription_id);
    let block1 = &notif1["params"]["result"];
    assert_eq!(block1["number"], "0x1");
    assert_eq!(block1["transactions"].as_array().unwrap().len(), 1);

    setup.send_flashblock(setup.create_second_payload()).await?;
    let notif2 = ws_stream.next().await.unwrap()?;
    let notif2: serde_json::Value = serde_json::from_str(notif2.to_text()?)?;
    assert_eq!(notif2["params"]["subscription"], subscription_id);
    let block2 = &notif2["params"]["result"];
    // Same block, incremental updates: 1 from first flashblock + 5 from the second.
    assert_eq!(block1["number"], block2["number"]);
    assert_eq!(block2["transactions"].as_array().unwrap().len(), 6);

    Ok(())
}

#[tokio::test]
async fn test_eth_unsubscribe() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let subscription_id = ws_subscribe(&mut ws_stream, 1, json!(["newFlashblocks"])).await?;

    ws_stream
        .send(Message::Text(
            json!({"jsonrpc": "2.0", "id": 2, "method": "eth_unsubscribe", "params": [subscription_id]})
                .to_string()
                .into(),
        ))
        .await?;

    let unsub = ws_stream.next().await.unwrap()?;
    let unsub: serde_json::Value = serde_json::from_str(unsub.to_text()?)?;
    assert_eq!(unsub["id"], 2);
    assert_eq!(unsub["result"], true);

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_multiple_clients() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws1, _) = connect_async(&ws_url).await?;
    let (mut ws2, _) = connect_async(&ws_url).await?;

    ws_subscribe(&mut ws1, 1, json!(["newFlashblocks"])).await?;
    ws_subscribe(&mut ws2, 1, json!(["newFlashblocks"])).await?;

    setup.send_flashblock(setup.create_first_payload()).await?;

    let notif1 = ws1.next().await.unwrap()?;
    let notif1: serde_json::Value = serde_json::from_str(notif1.to_text()?)?;
    let notif2 = ws2.next().await.unwrap()?;
    let notif2: serde_json::Value = serde_json::from_str(notif2.to_text()?)?;
    assert_eq!(notif1["method"], "eth_subscription");
    assert_eq!(notif2["method"], "eth_subscription");

    let block1 = &notif1["params"]["result"];
    let block2 = &notif2["params"]["result"];
    assert_eq!(block1["number"], "0x1");
    assert_eq!(block1["number"], block2["number"]);
    assert_eq!(block1["hash"], block2["hash"]);

    Ok(())
}

/// Verifies that standard subscription kinds (newHeads) are proxied to reth's implementation via
/// `ExtendedSubscriptionKind`.
#[tokio::test]
async fn test_eth_subscribe_new_heads() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let id = ws_subscribe(&mut ws_stream, 1, json!(["newHeads"])).await?;
    assert!(!id.is_empty(), "expected a subscription id for newHeads");

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_new_flashblock_transactions_hashes() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let subscription_id =
        ws_subscribe(&mut ws_stream, 1, json!(["newFlashblockTransactions"])).await?;

    // First flashblock: 1 transaction -> 1 hash message.
    setup.send_flashblock(setup.create_first_payload()).await?;
    let notification = ws_stream.next().await.unwrap()?;
    let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
    assert_eq!(notif["params"]["subscription"], subscription_id);
    assert!(notif["params"]["result"].is_string(), "expected a single hash string");

    // Second flashblock delta: 5 transactions -> 5 separate hash messages.
    setup.send_flashblock(setup.create_second_payload()).await?;
    let mut received = Vec::new();
    for _ in 0..5 {
        let notification = ws_stream.next().await.unwrap()?;
        let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
        assert_eq!(notif["params"]["subscription"], subscription_id);
        received.push(notif["params"]["result"].as_str().expect("hash string").to_string());
    }
    assert_eq!(received.len(), 5);

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_new_flashblock_transactions_full() -> Result<()> {
    let setup = TestSetup::new().await?;
    let ws_url = setup.harness.ws_url();
    let (mut ws_stream, _) = connect_async(&ws_url).await?;

    let subscription_id =
        ws_subscribe(&mut ws_stream, 1, json!(["newFlashblockTransactions", true])).await?;

    setup.send_flashblock(setup.create_first_payload()).await?;
    let notification = ws_stream.next().await.unwrap()?;
    let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
    assert_eq!(notif["params"]["subscription"], subscription_id);

    let tx = &notif["params"]["result"];
    assert!(tx.is_object(), "expected a full transaction object, got: {tx:?}");
    assert!(tx["hash"].is_string(), "expected flattened tx hash");
    assert!(tx["blockNumber"].is_string(), "expected flattened tx blockNumber");
    assert!(tx["logs"].is_array(), "expected logs array");
    let gas_used = tx["gasUsed"].as_str().expect("gasUsed should be a hex quantity string");
    assert!(gas_used.starts_with("0x"), "gasUsed should be a hex quantity, got: {gas_used}");
    assert_eq!(tx["status"], "0x1", "expected a receipt-shaped status");
    assert!(tx["cumulativeGasUsed"].as_str().is_some_and(|v| v.starts_with("0x")));
    // An absent key reads as null, so check the key rather than the value.
    let contract_address =
        tx.get("contractAddress").expect("contractAddress should be present on the wire");
    assert!(contract_address.is_null() || contract_address.is_string());
    assert!(tx["logsBloom"].is_string());

    // Second flashblock delta: 5 transactions -> 5 separate full-tx messages.
    setup.send_flashblock(setup.create_second_payload()).await?;
    for _ in 0..5 {
        let notification = ws_stream.next().await.unwrap()?;
        let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
        assert_eq!(notif["params"]["subscription"], subscription_id);
        let tx = &notif["params"]["result"];
        assert!(tx["hash"].is_string() && tx["blockNumber"].is_string());
        assert!(tx["logs"].is_array());

        let gas_used = tx["gasUsed"].as_str().expect("gasUsed should be a hex quantity string");
        assert!(gas_used.starts_with("0x"), "gasUsed should be a hex quantity, got: {gas_used}");
        assert_eq!(tx["status"], "0x1");
        assert!(tx["cumulativeGasUsed"].as_str().is_some_and(|v| v.starts_with("0x")));
        assert!(tx.get("contractAddress").is_some());
        assert!(tx["logsBloom"].is_string());
    }

    Ok(())
}

// ================================ eth_getLogs (pending) ================================
//
// Our `get_pending_logs` serves logs from the flashblock's `metadata.receipts` (the production
// path). These tests attach logs to a receipt and exercise the `eth_getLogs` override + filtering.

const LOG_EMITTER_A: Address = address!("0x000000000000000000000000000000000000a001");
const LOG_EMITTER_B: Address = address!("0x000000000000000000000000000000000000b002");
const TEST_LOG_TOPIC_0: B256 =
    b256!("0xaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
const TEST_LOG_TOPIC_1: B256 =
    b256!("0xbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb");

const fn make_log(address: Address, topics: Vec<B256>) -> PrimitiveLog {
    PrimitiveLog { address, data: LogData::new_unchecked(topics, Bytes::new()) }
}

/// A single-flashblock payload (block 1) whose lone transaction's receipt carries `logs`.
fn logs_payload(logs: Vec<PrimitiveLog>) -> FlashBlock {
    let mut receipts = HashMap::default();
    receipts.insert(
        TRANSFER_ETH_HASH,
        Receipt {
            tx_type: alloy_consensus::TxType::Eip1559,
            success: true,
            cumulative_gas_used: 21000,
            logs,
        },
    );
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index: 0,
        base: Some(ExecutionPayloadBaseV1 {
            parent_beacon_block_root: TEST_PARENT_BEACON_BLOCK_ROOT,
            parent_hash: B256::default(),
            fee_recipient: Address::ZERO,
            prev_randao: B256::default(),
            block_number: 1,
            gas_limit: 30_000_000,
            timestamp: 0,
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::ZERO,
        }),
        diff: ExecutionPayloadFlashblockDeltaV1 {
            blob_gas_used: 0,
            transactions: vec![TRANSFER_ETH_TX],
            ..Default::default()
        },
        metadata: Metadata {
            block_number: 1,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

// ============================ pending state (the overlay) ============================
//
// Methods this node does not override answer `pending` through `EthgasEthApi::local_pending_state`.

#[tokio::test]
async fn test_pending_code_and_storage_come_from_flashblocks() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let counter = setup.txn_details.counter_address;

    assert!(provider.get_code_at(counter).await?.is_empty());
    assert!(provider.get_code_at(counter).pending().await?.is_empty());

    setup.send_flashblock(setup.create_first_payload()).await?;
    assert!(
        provider.get_code_at(counter).pending().await?.is_empty(),
        "the first flashblock deploys nothing"
    );

    setup.send_flashblock(setup.create_second_payload()).await?;

    let pending_code = provider.get_code_at(counter).pending().await?;
    assert_eq!(
        pending_code,
        DoubleCounter::DEPLOYED_BYTECODE,
        "pending must see a contract deployed inside a flashblock"
    );

    // `count1` is slot 0. It initialises to 1 and the same flashblock increments it once.
    let pending_slot0 = provider.get_storage_at(counter, U256::ZERO).pending().await?;
    assert_eq!(pending_slot0, U256::from(2), "pending must see storage written by a flashblock");

    assert!(
        provider.get_code_at(counter).await?.is_empty(),
        "the overlay must not leak into the canonical tag"
    );
    assert_eq!(provider.get_storage_at(counter, U256::ZERO).await?, U256::ZERO);

    Ok(())
}

#[tokio::test]
async fn test_get_proof_refuses_pending() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let err = provider
        .get_proof(setup.txn_details.counter_address, vec![])
        .pending()
        .await
        .expect_err("eth_getProof must refuse the pending tag");
    assert!(err.to_string().contains("state root"), "the refusal should say why, got: {err}");
    assert!(
        err.to_string().contains("-32602"),
        "the refusal must be an invalid-params error: {err}"
    );

    let proof = provider.get_proof(setup.txn_details.counter_address, vec![]).await?;
    assert_eq!(proof.address, setup.txn_details.counter_address);

    Ok(())
}

/// While the engine holds an executed block at least as new as the snapshot, `pending` is that
/// block, which has a state root, so a proof at `pending` answers from it.
#[tokio::test]
async fn test_get_proof_at_pending_answers_from_the_engine_block() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let (transfer_tx, _) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(999)).nonce(0),
    )?;
    setup.harness.submit_block_from_transactions(vec![transfer_tx]).await?;
    assert_eq!(provider.get_block_number().await?, 0, "the block must not be canonical");

    let proof = provider.get_proof(PROBE_ADDRESS, vec![]).pending().await?;
    assert_eq!(proof.balance, U256::from(999), "the proof must describe the engine block");

    Ok(())
}

#[tokio::test]
async fn test_pending_balance_miss_path_comes_from_flashblocks() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    assert_eq!(provider.get_balance(TRANSFER_ETH_RECIPIENT).await?, U256::ZERO);

    assert_eq!(
        provider.get_balance(TRANSFER_ETH_RECIPIENT).pending().await?,
        U256::from(50_000_000_000_000_000_000_u128),
        "a balance the builder did not report must come from the flashblocks bundle"
    );

    Ok(())
}

#[tokio::test]
async fn test_pending_state_is_anchored_to_the_block_the_bundle_was_built_on() -> Result<()> {
    let setup = TestSetup::manual_canonical().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    // A canonical block the bundle was not executed on.
    let (transfer_tx, _) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(777)).nonce(0),
    )?;
    setup.harness.build_block_from_transactions(vec![transfer_tx]).await?;
    assert_eq!(provider.get_block_number().await?, 1);
    assert_eq!(provider.get_balance(PROBE_ADDRESS).await?, U256::from(777));

    assert_eq!(
        provider.get_balance(PROBE_ADDRESS).pending().await?,
        U256::ZERO,
        "pending must be anchored to the block the bundle was built on"
    );

    // The snapshot itself still answers.
    assert_eq!(
        provider.get_code_at(setup.txn_details.counter_address).pending().await?,
        DoubleCounter::DEPLOYED_BYTECODE
    );

    Ok(())
}

#[tokio::test]
async fn test_pending_state_defers_to_the_engine_pending_block() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    // Executed by the engine, not yet canonical.
    let (transfer_tx, _) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(999)).nonce(0),
    )?;
    setup.harness.submit_block_from_transactions(vec![transfer_tx]).await?;
    assert_eq!(provider.get_block_number().await?, 0, "the block must not be canonical");
    assert_eq!(provider.get_balance(PROBE_ADDRESS).await?, U256::ZERO);

    assert_eq!(
        provider.get_balance(PROBE_ADDRESS).pending().await?,
        U256::from(999),
        "an executed engine block at least as new as the snapshot must outrank the overlay"
    );

    Ok(())
}

#[tokio::test]
async fn test_get_logs_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    // No pending flashblock yet -> no pending logs.
    let logs = provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
    assert_eq!(logs.len(), 0);

    harness
        .send_flashblock(logs_payload(vec![
            make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0]),
            make_log(LOG_EMITTER_B, vec![TEST_LOG_TOPIC_0]),
        ]))
        .await?;

    let logs = provider
        .get_logs(
            &Filter::default()
                .from_block(BlockNumberOrTag::Pending)
                .to_block(BlockNumberOrTag::Pending),
        )
        .await?;
    assert_eq!(logs.len(), 2);
    assert_eq!(logs[0].address(), LOG_EMITTER_A);
    assert_eq!(logs[0].topics()[0], TEST_LOG_TOPIC_0);
    assert_eq!(logs[0].transaction_hash, Some(TRANSFER_ETH_HASH));
    assert_eq!(logs[1].address(), LOG_EMITTER_B);

    Ok(())
}

#[tokio::test]
async fn test_pending_block_and_transaction_report_no_block_hash() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    harness
        .send_flashblock(logs_payload(vec![make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0])]))
        .await?;

    let pending_block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block expected");
    assert_eq!(pending_block.hash(), B256::ZERO);

    let tx = provider
        .get_transaction_by_hash(TRANSFER_ETH_HASH)
        .await?
        .expect("pending transaction expected");
    assert_eq!(tx.block_hash(), None);

    let receipt = provider
        .get_transaction_receipt(TRANSFER_ETH_HASH)
        .await?
        .expect("pending receipt expected");
    assert_eq!(receipt.block_hash, Some(B256::ZERO));

    let logs = provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].block_hash, Some(B256::ZERO));

    Ok(())
}

#[tokio::test]
async fn test_get_logs_filter_by_address() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    harness
        .send_flashblock(logs_payload(vec![
            make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0]),
            make_log(LOG_EMITTER_B, vec![TEST_LOG_TOPIC_0]),
        ]))
        .await?;

    let logs = provider
        .get_logs(
            &Filter::default()
                .address(LOG_EMITTER_A)
                .from_block(BlockNumberOrTag::Pending)
                .to_block(BlockNumberOrTag::Pending),
        )
        .await?;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].address(), LOG_EMITTER_A);

    Ok(())
}

#[tokio::test]
async fn test_get_logs_topic_filtering() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    harness
        .send_flashblock(logs_payload(vec![
            make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0]),
            make_log(LOG_EMITTER_B, vec![TEST_LOG_TOPIC_1]),
        ]))
        .await?;

    // topic0 == TEST_LOG_TOPIC_1 matches only the second log.
    let logs = provider
        .get_logs(
            &Filter::default()
                .event_signature(TEST_LOG_TOPIC_1)
                .from_block(BlockNumberOrTag::Pending)
                .to_block(BlockNumberOrTag::Pending),
        )
        .await?;
    assert_eq!(logs.len(), 1);
    assert_eq!(logs[0].address(), LOG_EMITTER_B);
    assert_eq!(logs[0].topics()[0], TEST_LOG_TOPIC_1);

    Ok(())
}

// ============================== eth_simulateV1 / sync / header ==============================

#[tokio::test]
async fn test_eth_simulate_v1() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    // After the test payloads, `count1` is 2. Simulate: read count1, increment(), read count1.
    let simulate_call = SimulatePayload {
        block_state_calls: vec![SimBlock {
            calls: vec![
                setup.count1().gas_limit(100_000),
                TransactionRequest::default()
                    .from(Account::Alice.address())
                    .to(setup.txn_details.counter_address)
                    .gas_limit(200_000)
                    .input(TransactionInput::new(bytes!("0xd09de08a"))),
                setup.count1().gas_limit(100_000),
            ],
            block_overrides: None,
            state_overrides: None,
        }],
        trace_transfers: false,
        // Pending balances are advertised via `metadata.new_account_balances`, which the test
        // fixtures populate only for `TEST_ADDRESS`; `validation: true` would reject the
        // increment sender for lack of (sequencer-advertised) funds. We only need to verify that
        // the simulation observes pending flashblock state (the deployed counter) and applies the
        // simulated mutation, so disable sender validation.
        validation: false,
        return_full_transactions: true,
    };

    let block =
        provider.simulate(&simulate_call).block_id(BlockNumberOrTag::Pending.into()).await?;
    assert_eq!(block.len(), 1);
    assert_eq!(block[0].calls.len(), 3);
    // count1 == 2 before the simulated increment, == 3 after.
    assert_eq!(
        block[0].calls[0].return_data,
        bytes!("0x0000000000000000000000000000000000000000000000000000000000000002")
    );
    assert_eq!(block[0].calls[1].return_data, bytes!("0x"));
    assert_eq!(
        block[0].calls[2].return_data,
        bytes!("0x0000000000000000000000000000000000000000000000000000000000000003")
    );

    Ok(())
}

#[tokio::test]
async fn test_send_raw_transaction_sync() -> Result<()> {
    let setup = TestSetup::new().await?;

    setup.send_flashblock(setup.create_first_payload()).await?;

    // Run the sync request and deliver the payload that contains the tx in parallel.
    let second_payload = setup.create_second_payload();
    let (receipt_result, payload_result) = tokio::join!(
        setup.send_raw_transaction_sync(setup.txn_details.alice_eth_transfer_tx.clone(), None),
        async {
            tokio::time::sleep(std::time::Duration::from_millis(100)).await;
            setup.send_flashblock(second_payload).await
        }
    );

    payload_result?;
    let receipt = receipt_result?;
    assert_eq!(receipt.transaction_hash, setup.txn_details.alice_eth_transfer_hash);

    Ok(())
}

#[tokio::test]
async fn test_send_raw_transaction_sync_timeout() {
    let setup = TestSetup::new().await.unwrap();

    // A 0ms timeout fails the request immediately (the tx is never delivered).
    let receipt_result = setup
        .send_raw_transaction_sync(setup.txn_details.alice_eth_transfer_tx.clone(), Some(0))
        .await;

    let error_code = EthRpcErrorCode::TransactionConfirmationTimeout.code();
    assert!(
        receipt_result.err().unwrap().to_string().contains(format!("{error_code}").as_str()),
        "expected a transaction-confirmation-timeout error"
    );
}

#[tokio::test]
async fn test_pending_block_header_fields() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let pending_block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("pending block expected");

    // withdrawals should be an empty array, not null.
    assert_eq!(pending_block.withdrawals, Some(vec![].into()));
    assert_eq!(pending_block.header.parent_beacon_block_root, Some(TEST_PARENT_BEACON_BLOCK_ROOT));
    assert_eq!(pending_block.header.withdrawals_root, Some(EMPTY_WITHDRAWALS));
    assert_eq!(pending_block.header.requests_hash, Some(EMPTY_REQUESTS_HASH));

    Ok(())
}

// ============================ every other path to the pending tag ============================
//
// What the overrides answer without a snapshot and at every other tag, how the builder's balance
// map and the overlay relate, and how the methods this node does not override answer `pending`.

/// An address only the builder's balance map knows about.
const MAP_ONLY_ADDRESS: Address = address!("0x00000000000000000000000000000000000ba1a2");
const MAP_ONLY_BALANCE: u64 = 1234;

type WsStream =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// A delta flashblock (index 1) for block 1 carrying only `tx`, whose receipt carries `logs`.
fn delta_payload_with_logs(
    tx: Bytes,
    cumulative_gas_used: u64,
    logs: Vec<PrimitiveLog>,
) -> FlashBlock {
    let mut receipts = HashMap::default();
    receipts.insert(
        keccak256(&tx),
        Receipt {
            tx_type: alloy_consensus::TxType::Eip1559,
            success: true,
            cumulative_gas_used,
            logs,
        },
    );
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index: 1,
        base: None,
        diff: ExecutionPayloadFlashblockDeltaV1 { transactions: vec![tx], ..Default::default() },
        metadata: Metadata {
            block_number: 1,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

/// The `result` of the next notification on subscription `id`.
async fn next_notification(ws: &mut WsStream, id: &str) -> Result<serde_json::Value> {
    let message = ws.next().await.expect("the socket is open")?;
    let notification: serde_json::Value = serde_json::from_str(message.to_text()?)?;
    assert_eq!(notification["method"], "eth_subscription");
    assert_eq!(notification["params"]["subscription"], id);
    Ok(notification["params"]["result"].clone())
}

/// Nothing arrives within 300 ms.
async fn assert_no_notification(ws: &mut WsStream) {
    let quiet = tokio::time::timeout(Duration::from_millis(300), ws.next()).await;
    assert!(quiet.is_err(), "unexpected notification: {quiet:?}");
}

fn as_b256(value: &serde_json::Value) -> B256 {
    value.as_str().expect("a hex string").parse().expect("a 32-byte hex string")
}

fn as_address(value: &serde_json::Value) -> Address {
    value.as_str().expect("a hex string").parse().expect("a 20-byte hex string")
}

/// Without a snapshot, every override answers `pending` from canonical state. A block built from
/// the pool would show the deployment, count the nonce, and list the receipt.
#[tokio::test]
async fn test_pending_without_a_snapshot_answers_from_canonical_state_not_the_pool() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    let deployer = Account::Deployer.address();

    let latest_balance = provider.get_balance(deployer).await?;
    let _pool_only =
        provider.send_raw_transaction(&setup.txn_details.counter_deployment_tx).await?;

    assert_eq!(provider.get_transaction_count(deployer).pending().await?, 0);
    assert_eq!(provider.get_balance(deployer).pending().await?, latest_balance);
    let count1: Bytes = client.request("eth_call", (setup.count1(), "pending")).await?;
    assert!(count1.is_empty(), "a contract that exists only in the pool must not answer");
    let pending_gas: U256 = client.request("eth_estimateGas", (setup.count1(), "pending")).await?;
    let latest_gas: U256 = client.request("eth_estimateGas", (setup.count1(), "latest")).await?;
    assert_eq!(pending_gas, latest_gas);
    let count: Option<U256> =
        client.request("eth_getBlockTransactionCountByNumber", ("pending",)).await?;
    assert_eq!(count, Some(U256::ZERO));
    let block = provider.get_block_by_number(BlockNumberOrTag::Pending).await?.expect("latest");
    assert_eq!(block.number(), 0);
    let logs = provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
    assert!(logs.is_empty());

    // The methods this node does not override resolve `pending` through reth, which must reach
    // the canonical tip and never a pool-built block.
    assert_eq!(provider.get_block_receipts(BlockId::pending()).await?, Some(vec![]));
    assert!(
        provider
            .get_transaction_by_block_number_and_index(BlockNumberOrTag::Pending, 0)
            .await?
            .is_none()
    );

    Ok(())
}

#[tokio::test]
async fn test_get_transaction_count_pending_adds_flashblock_transactions_to_the_canonical_nonce()
-> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let alice = Account::Alice.address();
    setup.send_test_payloads().await?;

    // TRANSFER_ETH_TX, the ETH transfer and the balance transfer: nonces 0, 1 and 2.
    assert_eq!(provider.get_transaction_count(alice).pending().await?, 3);
    assert_eq!(provider.get_transaction_count(alice).await?, 0);

    let info = provider.get_account_info(alice).pending().await?;
    assert_eq!(info.nonce, 3, "eth_getTransactionCount and eth_getAccountInfo must agree");

    Ok(())
}

/// A flashblock that repeats a transaction the block already holds is rejected, so the pending
/// block lists the transaction once, where it first appeared.
///
/// The repeat is the block's last transaction: receipts are keyed by hash, so both copies share
/// one cumulative gas, which stays monotonic only when nothing lies between them.
#[tokio::test]
async fn test_a_flashblock_that_repeats_a_transaction_is_rejected() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let last = setup.txn_details.balance_transfer_tx.clone();
    let hash = keccak256(&last);
    let mut repeat = setup.create_second_payload();
    repeat.index = 2;
    repeat.diff.transactions = vec![last];
    setup.send_flashblock(repeat).await?;

    let block = provider
        .get_block_by_number(BlockNumberOrTag::Pending)
        .await?
        .expect("a pending block is published");
    assert_eq!(block.transactions.hashes().filter(|listed| *listed == hash).count(), 1);
    assert_eq!(block.transactions.len(), 6, "the rejected flashblock adds nothing");
    let transaction =
        provider.get_transaction_by_hash(hash).await?.expect("the transfer is pending");
    assert_eq!(transaction.transaction_index, Some(5));

    Ok(())
}

/// A block that turns canonical before the processor clears the snapshot holding its transactions
/// does not raise the pending nonce twice.
#[tokio::test]
async fn test_pending_nonce_counts_a_transaction_once_while_its_block_turns_canonical() -> Result<()>
{
    let setup = TestSetup::manual_canonical().await?;
    let provider = setup.harness.provider();
    let charlie = Account::Charlie.address();

    let (transfer, transfer_hash) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(1)).nonce(0),
    )?;
    let mut payload = setup.create_first_payload();
    payload.diff.transactions = vec![transfer.clone()];
    payload.metadata.receipts = HashMap::from_iter([(
        transfer_hash,
        Receipt {
            tx_type: alloy_consensus::TxType::Eip1559,
            success: true,
            cumulative_gas_used: 21_000,
            logs: vec![],
        },
    )]);
    setup.send_flashblock(payload).await?;
    assert_eq!(provider.get_transaction_count(charlie).pending().await?, 1);

    // The processor never hears of this block, so the snapshot keeps the transfer.
    setup.harness.build_block_from_transactions(vec![transfer]).await?;
    assert_eq!(provider.get_transaction_count(charlie).await?, 1, "latest holds the transfer");
    assert_eq!(
        provider.get_transaction_count(charlie).pending().await?,
        1,
        "the snapshot must not count the transfer again"
    );

    Ok(())
}

#[tokio::test]
async fn test_eth_estimate_gas_pending_executes_against_flashblock_state() -> Result<()> {
    let setup = TestSetup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.send_test_payloads().await?;

    let counter =
        DoubleCounterInstance::new(setup.txn_details.counter_address, setup.harness.provider());
    let increment = counter.increment().into_transaction_request();
    let pending: U256 = client.request("eth_estimateGas", (increment.clone(), "pending")).await?;
    // At `latest` the counter does not exist, so the call is a transfer with calldata.
    let latest: U256 = client.request("eth_estimateGas", (increment, "latest")).await?;
    assert!(
        pending > latest,
        "pending ({pending}) must pay for the SSTORE the flashblock-deployed counter executes, \
         latest is {latest}"
    );

    Ok(())
}

#[tokio::test]
async fn test_send_raw_transaction_sync_rejects_a_timeout_above_the_cap() -> Result<()> {
    let setup = TestSetup::new().await?;

    let err = setup
        .send_raw_transaction_sync(setup.txn_details.alice_eth_transfer_tx.clone(), Some(6_001))
        .await
        .expect_err("6001 ms must be rejected, not clamped");
    let message = err.to_string();
    assert!(message.contains("-32602"), "expected an invalid-params error, got: {message}");
    assert!(message.contains("time out too long"), "got: {message}");

    // The rejection comes before submission, so the pool never saw the transaction.
    assert!(
        setup
            .harness
            .provider()
            .get_transaction_by_hash(setup.txn_details.alice_eth_transfer_hash)
            .await?
            .is_none()
    );

    Ok(())
}

#[tokio::test]
async fn test_get_balance_pending_prefers_the_builder_balance_map() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_flashblock(setup.create_first_payload()).await?;
    let mut second = setup.create_second_payload();
    // No transaction touches this address, so only the map knows the balance.
    second.metadata.new_account_balances.insert(MAP_ONLY_ADDRESS, U256::from(MAP_ONLY_BALANCE));
    setup.send_flashblock(second).await?;

    assert_eq!(
        provider.get_balance(MAP_ONLY_ADDRESS).pending().await?,
        U256::from(MAP_ONLY_BALANCE)
    );
    assert_eq!(provider.get_balance(MAP_ONLY_ADDRESS).await?, U256::ZERO);
    // The overlay is built from execution, so the map is invisible to it.
    assert_eq!(provider.get_account_info(MAP_ONLY_ADDRESS).pending().await?.balance, U256::ZERO);

    // For an address a flashblock transaction did touch, the two sources agree.
    assert_eq!(provider.get_balance(TEST_ADDRESS).pending().await?, U256::from(PENDING_BALANCE));
    assert_eq!(
        provider.get_account_info(TEST_ADDRESS).pending().await?.balance,
        U256::from(PENDING_BALANCE)
    );

    Ok(())
}

#[tokio::test]
async fn test_pending_account_info_and_storage_values_come_from_flashblocks() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    let counter = setup.txn_details.counter_address;
    setup.send_test_payloads().await?;

    let info = provider.get_account_info(counter).pending().await?;
    assert_eq!(info.code, DoubleCounter::DEPLOYED_BYTECODE);
    assert_eq!(info.nonce, 1, "a created contract starts at nonce 1");
    assert!(provider.get_account_info(counter).await?.code.is_empty());

    let mut slots = HashMap::default();
    slots.insert(counter, vec![U256::ZERO, U256::from(1)]);
    let pending: HashMap<Address, Vec<B256>> =
        client.request("eth_getStorageValues", (slots.clone(), "pending")).await?;
    // count1 and count2 start at 1, and the flashblock increments each once.
    assert_eq!(pending[&counter], vec![B256::with_last_byte(2), B256::with_last_byte(2)]);
    let latest: HashMap<Address, Vec<B256>> =
        client.request("eth_getStorageValues", (slots, "latest")).await?;
    assert_eq!(latest[&counter], vec![B256::ZERO, B256::ZERO]);

    Ok(())
}

#[tokio::test]
async fn test_get_account_and_get_multi_proof_refuse_pending() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    let alice = Account::Alice.address();
    setup.send_test_payloads().await?;

    let err = provider.get_account(alice).pending().await.expect_err("eth_getAccount");
    assert!(err.to_string().contains("-32602"), "not an invalid-params error: {err}");
    assert!(err.to_string().contains("state root"), "the refusal should say why: {err}");

    let targets = vec![(alice, Vec::<B256>::new())];
    let err = client
        .request::<_, serde_json::Value>("eth_getMultiProof", (targets.clone(), "pending"))
        .await
        .expect_err("eth_getMultiProof");
    assert!(err.to_string().contains("-32602"), "not an invalid-params error: {err}");
    assert!(err.to_string().contains("state root"), "the refusal should say why: {err}");

    // Both answer at `latest`.
    assert_eq!(provider.get_account(alice).await?.nonce, 0);
    let proofs: Vec<serde_json::Value> =
        client.request("eth_getMultiProof", (targets, "latest")).await?;
    assert_eq!(proofs.len(), 1);

    Ok(())
}

/// `eth_getBlockReceipts` and `eth_getTransactionByBlockNumberAndIndex` at `pending` describe the
/// flashblock, as `eth_getBlockByNumber` does, even while the canonical tip is a different block of
/// the same number. `eth_feeHistory` is not overridden and describes the canonical tip.
#[tokio::test]
async fn test_block_receipts_and_transaction_by_index_at_pending_describe_the_flashblock()
-> Result<()> {
    let setup = TestSetup::manual_canonical().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let (charlie_tx, charlie_hash) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(777)).nonce(0),
    )?;
    setup.harness.build_block_from_transactions(vec![charlie_tx]).await?;
    assert_eq!(provider.get_block_number().await?, 1);

    let pending_block =
        provider.get_block_by_number(BlockNumberOrTag::Pending).await?.expect("pending block");
    let hashes: Vec<TxHash> = pending_block.transactions.hashes().collect();
    assert_eq!(hashes.len(), 6);

    let pending_receipts = provider.get_block_receipts(BlockId::pending()).await?.expect("block");
    let receipt_hashes: Vec<TxHash> =
        pending_receipts.iter().map(|receipt| receipt.transaction_hash).collect();
    assert_eq!(receipt_hashes, hashes, "the flashblock's receipts, in block order");
    let latest_receipts = provider.get_block_receipts(BlockId::latest()).await?.expect("block");
    assert_eq!(latest_receipts.len(), 1);
    assert_eq!(latest_receipts[0].transaction_hash, charlie_hash);

    let last = provider
        .get_transaction_by_block_number_and_index(BlockNumberOrTag::Pending, 5)
        .await?
        .expect("the flashblock holds six transactions");
    assert_eq!(last.tx_hash(), hashes[5]);
    assert!(
        provider
            .get_transaction_by_block_number_and_index(BlockNumberOrTag::Pending, 6)
            .await?
            .is_none()
    );
    let canonical = provider
        .get_transaction_by_block_number_and_index(BlockNumberOrTag::Latest, 0)
        .await?
        .expect("block 1 holds one transaction");
    assert_eq!(canonical.tx_hash(), charlie_hash);

    let pending_history = provider.get_fee_history(1, BlockNumberOrTag::Pending, &[]).await?;
    let latest_history = provider.get_fee_history(1, BlockNumberOrTag::Latest, &[]).await?;
    assert_eq!(pending_history.oldest_block, 1);
    assert_eq!(pending_history.oldest_block, latest_history.oldest_block);
    assert_eq!(pending_history.base_fee_per_gas, latest_history.base_fee_per_gas);

    Ok(())
}

/// A snapshot and pending logs are present the whole time and must leak into no other tag.
#[tokio::test]
async fn test_overrides_delegate_every_other_tag_to_reth() -> Result<()> {
    let setup = TestSetup::manual_canonical().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    setup
        .send_flashblock(logs_payload(vec![make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0])]))
        .await?;

    // Block 1: Charlie funds PROBE_ADDRESS and the Deployer deploys the counter on chain.
    let (charlie_tx, _) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(PROBE_ADDRESS).value(U256::from(777)).nonce(0),
    )?;
    setup
        .harness
        .build_block_from_transactions(vec![
            charlie_tx,
            setup.txn_details.counter_deployment_tx.clone(),
        ])
        .await?;
    // Block 2 is empty, and moves `safe` and `finalized` to block 1.
    setup.harness.advance_chain(1).await?;
    assert_eq!(provider.get_block_number().await?, 2);
    let block_1 = provider.get_block_by_number(1.into()).await?.expect("block 1");
    assert_eq!(block_1.transactions.len(), 2);

    for (tag, number) in [
        (BlockNumberOrTag::Latest, 2),
        (BlockNumberOrTag::Number(1), 1),
        (BlockNumberOrTag::Earliest, 0),
        (BlockNumberOrTag::Safe, 1),
        (BlockNumberOrTag::Finalized, 1),
    ] {
        let block = provider
            .get_block_by_number(tag)
            .await?
            .unwrap_or_else(|| panic!("eth_getBlockByNumber({tag}) must resolve"));
        assert_eq!(block.number(), number, "eth_getBlockByNumber({tag})");
        assert_ne!(block.hash(), B256::ZERO, "eth_getBlockByNumber({tag}) is a sealed block");
        let count: Option<U256> =
            client.request("eth_getBlockTransactionCountByNumber", (tag,)).await?;
        assert_eq!(count, Some(U256::from(block.transactions.len())), "count at {tag}");
    }
    assert!(provider.get_block_by_number(99.into()).await?.is_none());
    let count: Option<U256> = client
        .request("eth_getBlockTransactionCountByNumber", (BlockNumberOrTag::Number(99),))
        .await?;
    assert!(count.is_none());

    assert_eq!(provider.get_balance(PROBE_ADDRESS).await?, U256::from(777));
    assert_eq!(provider.get_balance(PROBE_ADDRESS).block_id(0.into()).await?, U256::ZERO);
    assert_eq!(
        provider.get_balance(PROBE_ADDRESS).block_id(BlockId::finalized()).await?,
        U256::from(777)
    );
    assert_eq!(
        provider.get_balance(PROBE_ADDRESS).block_id(BlockId::hash(block_1.hash())).await?,
        U256::from(777)
    );
    assert_eq!(
        provider.get_balance(TRANSFER_ETH_RECIPIENT).await?,
        U256::ZERO,
        "the snapshot must not leak into latest"
    );
    assert!(provider.get_balance(PROBE_ADDRESS).block_id(99.into()).await.is_err());

    assert_eq!(provider.get_transaction_count(Account::Charlie.address()).await?, 1);
    assert_eq!(
        provider.get_transaction_count(Account::Charlie.address()).block_id(0.into()).await?,
        0
    );
    assert_eq!(
        provider.get_transaction_count(Account::Alice.address()).await?,
        0,
        "the flashblock transaction must not count at latest"
    );

    let latest: Bytes = client.request("eth_call", (setup.count1(), "latest")).await?;
    assert_eq!(U256::from_be_slice(&latest), U256::from(1), "deployed on chain in block 1");
    let by_hash: Bytes =
        client.request("eth_call", (setup.count1(), BlockId::hash(block_1.hash()))).await?;
    assert_eq!(by_hash, latest);
    let earliest: Bytes = client.request("eth_call", (setup.count1(), "earliest")).await?;
    assert!(earliest.is_empty(), "no contract at genesis");
    let counter =
        DoubleCounterInstance::new(setup.txn_details.counter_address, setup.harness.provider());
    let increment = counter.increment().into_transaction_request();
    let gas_latest: U256 = client.request("eth_estimateGas", (increment.clone(), "latest")).await?;
    let gas_earliest: U256 = client.request("eth_estimateGas", (increment, "earliest")).await?;
    assert!(gas_latest > gas_earliest, "latest ({gas_latest}) executes the counter");

    let emitter = Filter::new().address(LOG_EMITTER_A);
    let historical = provider
        .get_logs(&emitter.clone().from_block(BlockNumberOrTag::Earliest).to_block(2))
        .await?;
    assert!(historical.is_empty(), "pending logs must not leak into a canonical range");
    assert!(provider.get_logs(&emitter.clone().at_block_hash(block_1.hash())).await?.is_empty());
    assert_eq!(provider.get_logs(&emitter.select(BlockNumberOrTag::Pending)).await?.len(), 1);

    Ok(())
}

#[tokio::test]
async fn test_get_logs_reaches_flashblocks_only_when_to_block_is_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();
    harness
        .send_flashblock(logs_payload(vec![make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0])]))
        .await?;
    let genesis = provider.get_block_by_number(BlockNumberOrTag::Earliest).await?.expect("genesis");

    let to_pending = provider
        .get_logs(
            &Filter::new()
                .from_block(BlockNumberOrTag::Earliest)
                .to_block(BlockNumberOrTag::Pending),
        )
        .await?;
    assert_eq!(to_pending.len(), 1);
    let from_pending = provider
        .get_logs(
            &Filter::new().from_block(BlockNumberOrTag::Pending).to_block(BlockNumberOrTag::Latest),
        )
        .await?;
    assert!(from_pending.is_empty(), "fromBlock alone must not reach flashblocks");
    assert!(provider.get_logs(&Filter::new().at_block_hash(genesis.hash())).await?.is_empty());

    Ok(())
}

/// The block, its transaction count, each transaction by hash and by index, each receipt alone and
/// in the block's list, the logs and the `newFlashblocks` stream all describe the same flashblock
/// state.
#[tokio::test]
async fn test_one_snapshot_tells_one_story() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    let (mut ws, _) = connect_async(&setup.harness.ws_url()).await?;
    let id = ws_subscribe(&mut ws, 1, json!(["newFlashblocks"])).await?;

    setup.send_flashblock(setup.create_first_payload()).await?;
    next_notification(&mut ws, &id).await?;
    setup.send_flashblock(setup.create_second_payload()).await?;
    let streamed = next_notification(&mut ws, &id).await?;
    let streamed: Vec<TxHash> = streamed["transactions"]
        .as_array()
        .expect("full transactions")
        .iter()
        .map(|tx| as_b256(&tx["hash"]))
        .collect();

    let block =
        provider.get_block_by_number(BlockNumberOrTag::Pending).full().await?.expect("block");
    let hashes: Vec<TxHash> = block.transactions.hashes().collect();
    assert_eq!(hashes.len(), 6);
    assert_eq!(streamed, hashes, "newFlashblocks streams the block eth_getBlockByNumber serves");
    let count: Option<U256> =
        client.request("eth_getBlockTransactionCountByNumber", ("pending",)).await?;
    assert_eq!(count, Some(U256::from(hashes.len())));

    let mut cumulative_gas_used = 0;
    let mut receipts = Vec::new();
    for (index, (hash, in_block)) in hashes.iter().zip(block.transactions.txns()).enumerate() {
        let tx = provider.get_transaction_by_hash(*hash).await?.expect("pending transaction");
        assert_eq!(&tx, in_block, "eth_getTransactionByHash and the block disagree at {index}");
        let by_index = provider
            .get_transaction_by_block_number_and_index(BlockNumberOrTag::Pending, index)
            .await?
            .expect("indexed transaction");
        assert_eq!(&by_index, in_block, "the index and the block disagree at {index}");
        assert_eq!(tx.block_number, Some(1));
        assert_eq!(tx.transaction_index, Some(index as u64));
        assert_eq!(tx.block_hash, None);

        let receipt = provider.get_transaction_receipt(*hash).await?.expect("pending receipt");
        assert_eq!(receipt.transaction_hash, *hash);
        assert_eq!(receipt.block_number, Some(1));
        assert_eq!(receipt.transaction_index, Some(index as u64));
        assert_eq!(receipt.block_hash, Some(B256::ZERO));
        assert!(receipt.inner.cumulative_gas_used() > cumulative_gas_used);
        cumulative_gas_used = receipt.inner.cumulative_gas_used();
        receipts.push(receipt);
    }
    let block_receipts = provider.get_block_receipts(BlockId::pending()).await?.expect("receipts");
    assert_eq!(block_receipts, receipts, "eth_getBlockReceipts and the receipts disagree");

    let logs = provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
    assert!(logs.iter().all(|log| hashes.contains(&log.transaction_hash.expect("hash"))));

    Ok(())
}

// ============================ the remaining subscription forms ============================

#[tokio::test]
async fn test_eth_subscribe_pending_logs_filters_and_emits_only_the_latest_flashblock() -> Result<()>
{
    let setup = TestSetup::new().await?;
    let (mut ws, _) = connect_async(&setup.harness.ws_url()).await?;
    let id = ws_subscribe(&mut ws, 1, json!(["pendingLogs", {"address": LOG_EMITTER_A}])).await?;

    setup
        .send_flashblock(logs_payload(vec![
            make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0]),
            make_log(LOG_EMITTER_B, vec![TEST_LOG_TOPIC_0]),
        ]))
        .await?;
    let log = next_notification(&mut ws, &id).await?;
    assert_eq!(as_address(&log["address"]), LOG_EMITTER_A);
    assert_eq!(as_b256(&log["transactionHash"]), TRANSFER_ETH_HASH);
    assert_eq!(as_b256(&log["blockHash"]), B256::ZERO);
    assert_no_notification(&mut ws).await;

    // The next flashblock emits its own log only, never the first flashblock's again.
    setup
        .send_flashblock(delta_payload_with_logs(
            setup.txn_details.alice_eth_transfer_tx.clone(),
            121_000,
            vec![make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_1])],
        ))
        .await?;
    let log = next_notification(&mut ws, &id).await?;
    assert_eq!(as_b256(&log["transactionHash"]), setup.txn_details.alice_eth_transfer_hash);
    assert_eq!(as_b256(&log["topics"][0]), TEST_LOG_TOPIC_1);
    assert_no_notification(&mut ws).await;

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_new_flashblock_transactions_with_a_log_filter() -> Result<()> {
    let setup = TestSetup::new().await?;
    let (mut ws, _) = connect_async(&setup.harness.ws_url()).await?;
    let id =
        ws_subscribe(&mut ws, 1, json!(["newFlashblockTransactions", {"address": LOG_EMITTER_A}]))
            .await?;

    setup
        .send_flashblock(logs_payload(vec![
            make_log(LOG_EMITTER_A, vec![TEST_LOG_TOPIC_0]),
            make_log(LOG_EMITTER_B, vec![TEST_LOG_TOPIC_0]),
        ]))
        .await?;
    let tx = next_notification(&mut ws, &id).await?;
    assert_eq!(as_b256(&tx["hash"]), TRANSFER_ETH_HASH);
    assert_eq!(tx["logs"].as_array().map(Vec::len), Some(2), "a match carries all of its logs");
    assert_eq!(tx["status"], "0x1");
    assert!(tx["blockHash"].is_null());

    // Five transactions without a matching log emit nothing.
    setup.send_flashblock(setup.create_second_payload()).await?;
    assert_no_notification(&mut ws).await;

    Ok(())
}

#[tokio::test]
async fn test_eth_subscribe_standard_kinds_pass_through_to_reth() -> Result<()> {
    let setup = TestSetup::new().await?;
    let (mut ws, _) = connect_async(&setup.harness.ws_url()).await?;
    ws_subscribe(&mut ws, 1, json!(["logs", {"address": LOG_EMITTER_A}])).await?;
    let id = ws_subscribe(&mut ws, 2, json!(["newPendingTransactions"])).await?;

    let deployment = setup.txn_details.counter_deployment_tx.clone();
    let _pending = setup.harness.provider().send_raw_transaction(&deployment).await?;
    let hash = next_notification(&mut ws, &id).await?;
    assert_eq!(as_b256(&hash), keccak256(&deployment), "reth's pool subscription must fire");

    let id = ws_subscribe(&mut ws, 3, json!(["syncing"])).await?;
    assert!(!id.is_empty());

    Ok(())
}

// ============================ calls at pending run on the overlay ============================

/// `TIMESTAMP PUSH0 MSTORE PUSH1 32 PUSH0 RETURN`: returns the block timestamp.
const RETURN_TIMESTAMP: Bytes = bytes!("0x425f5260205ff3");
/// `TIMESTAMP PUSH4 PROBE_TIMESTAMP EQ PUSH1 13 JUMPI PUSH0 PUSH0 REVERT JUMPDEST STOP`: reverts
/// unless the block timestamp is `PROBE_TIMESTAMP`.
const REQUIRE_PROBE_TIMESTAMP: Bytes = bytes!("0x42636553f10014600d575f5ffd5b00");
const PROBE_TIMESTAMP: u64 = 1_700_000_000;

/// The first payload with a timestamp a call can tell from the genesis timestamp.
fn timestamped_first_payload(setup: &TestSetup) -> FlashBlock {
    let mut payload = setup.create_first_payload();
    payload.base.as_mut().expect("a base flashblock carries a base").timestamp = PROBE_TIMESTAMP;
    payload
}

fn code_override(code: Bytes) -> StateOverride {
    let mut overrides = StateOverride::default();
    overrides.insert(PROBE_ADDRESS, AccountOverride { code: Some(code), ..Default::default() });
    overrides
}

/// `PUSH20 address BALANCE PUSH0 MSTORE PUSH1 32 PUSH0 RETURN`: returns the balance of `address`.
fn return_balance_of(address: Address) -> Bytes {
    let mut code = vec![0x73];
    code.extend_from_slice(address.as_slice());
    code.extend_from_slice(&[0x31, 0x5f, 0x52, 0x60, 0x20, 0x5f, 0xf3]);
    code.into()
}

/// `eth_call` and `eth_estimateGas` at `pending` run in the flashblock's block environment, not
/// in the environment of the block it builds on.
#[tokio::test]
async fn test_calls_at_pending_run_in_the_flashblocks_block_environment() -> Result<()> {
    let setup = TestSetup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.send_flashblock(timestamped_first_payload(&setup)).await?;
    let call = TransactionRequest::default().to(PROBE_ADDRESS);

    let pending: Bytes = client
        .request("eth_call", (call.clone(), "pending", code_override(RETURN_TIMESTAMP)))
        .await?;
    assert_eq!(U256::from_be_slice(&pending), U256::from(PROBE_TIMESTAMP));
    let latest: Bytes = client
        .request("eth_call", (call.clone(), "latest", code_override(RETURN_TIMESTAMP)))
        .await?;
    assert_eq!(U256::from_be_slice(&latest), U256::from(setup.harness.latest_block().timestamp));

    let gas: U256 = client
        .request(
            "eth_estimateGas",
            (call.clone(), "pending", code_override(REQUIRE_PROBE_TIMESTAMP)),
        )
        .await?;
    assert!(gas > U256::from(21_000));
    let reverted = client
        .request::<_, U256>(
            "eth_estimateGas",
            (call, "latest", code_override(REQUIRE_PROBE_TIMESTAMP)),
        )
        .await;
    assert!(reverted.is_err(), "at latest the timestamp check must revert");

    Ok(())
}

/// While the engine holds an executed block at least as new as the snapshot, a call at `pending`
/// runs in that block's environment and on its state, never in the flashblock's.
#[tokio::test]
async fn test_calls_at_pending_run_in_the_engine_blocks_environment_while_it_is_as_new()
-> Result<()> {
    let setup = TestSetup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.send_flashblock(timestamped_first_payload(&setup)).await?;

    let engine_timestamp = setup.harness.latest_block().timestamp + BLOCK_TIME_SECONDS;
    let (transfer_tx, _) = Account::Charlie.sign_txn_request(
        TransactionRequest::default().to(TEST_ADDRESS).value(U256::from(999)).nonce(0),
    )?;
    setup.harness.submit_block_from_transactions(vec![transfer_tx]).await?;
    let call = TransactionRequest::default().to(PROBE_ADDRESS);

    let timestamp: Bytes = client
        .request("eth_call", (call.clone(), "pending", code_override(RETURN_TIMESTAMP)))
        .await?;
    assert_eq!(U256::from_be_slice(&timestamp), U256::from(engine_timestamp));

    let engine_only: Bytes = client
        .request(
            "eth_call",
            (call.clone(), "pending", code_override(return_balance_of(TEST_ADDRESS))),
        )
        .await?;
    assert_eq!(U256::from_be_slice(&engine_only), U256::from(999), "the engine block's state");

    let flashblock_only: Bytes = client
        .request(
            "eth_call",
            (call, "pending", code_override(return_balance_of(TRANSFER_ETH_RECIPIENT))),
        )
        .await?;
    assert_eq!(U256::from_be_slice(&flashblock_only), U256::ZERO, "not the flashblock's state");

    Ok(())
}

/// A caller's override of one field leaves the flashblock's code and storage in place.
#[tokio::test]
async fn test_eth_call_pending_keeps_flashblock_state_under_a_partial_override() -> Result<()> {
    let setup = TestSetup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.send_test_payloads().await?;

    let mut overrides = StateOverride::default();
    overrides.insert(
        setup.txn_details.counter_address,
        AccountOverride { balance: Some(U256::from(1)), ..Default::default() },
    );
    let count1: Bytes = client.request("eth_call", (setup.count1(), "pending", overrides)).await?;
    assert_eq!(U256::from_be_slice(&count1), U256::from(2));

    Ok(())
}

/// The second simulated block sees the first one's writes on top of the flashblock's state.
#[tokio::test]
async fn test_eth_simulate_v1_pending_carries_state_from_block_to_block() -> Result<()> {
    let setup = TestSetup::new().await?;
    let provider = setup.harness.provider();
    setup.send_test_payloads().await?;

    let increment = TransactionRequest::default()
        .from(Account::Alice.address())
        .to(setup.txn_details.counter_address)
        .gas_limit(200_000)
        .input(TransactionInput::new(bytes!("0xd09de08a")));
    let simulation = SimulatePayload {
        block_state_calls: vec![
            SimBlock { calls: vec![increment], block_overrides: None, state_overrides: None },
            SimBlock {
                calls: vec![setup.count1().gas_limit(100_000)],
                block_overrides: None,
                state_overrides: None,
            },
        ],
        trace_transfers: false,
        validation: false,
        return_full_transactions: false,
    };

    let blocks = provider.simulate(&simulation).block_id(BlockNumberOrTag::Pending.into()).await?;
    assert_eq!(blocks.len(), 2);
    assert_eq!(blocks[0].calls[0].return_data, bytes!("0x"));
    assert_eq!(
        blocks[1].calls[0].return_data,
        bytes!("0x0000000000000000000000000000000000000000000000000000000000000003"),
        "block 2 must see block 1's increment on the flashblock's count of 2"
    );

    Ok(())
}
