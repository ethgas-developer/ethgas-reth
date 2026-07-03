//! Integration tests covering the Flashblocks RPC surface area.
//!
//! These tests exercise the flashblocks-extended RPC endpoints (pending block,
//! pending balance, pending transaction receipt, eth_call with flashblock state, etc.)
//! by launching a full local Ethereum node with the flashblocks test extension.

use std::str::FromStr;

use DoubleCounter::DoubleCounterInstance;
use alloy_eips::BlockNumberOrTag;
use alloy_provider::network::TransactionResponse;
use alloy_primitives::{
    Address, B256, Bytes, Log as PrimitiveLog, LogData, TxHash, U256, address, b256, bytes,
};
use alloy_provider::Provider;
use alloy_consensus::constants::EMPTY_WITHDRAWALS;
use alloy_eips::eip7685::EMPTY_REQUESTS_HASH;
use alloy_rpc_types::simulate::{SimBlock, SimulatePayload};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::{Filter, TransactionInput, TransactionRequest, error::EthRpcErrorCode};
use ethgas_node_runner::test_utils::{Account, DoubleCounter};
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use alloy_primitives::map::foldhash::HashMap;
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
        let harness = FlashblocksHarness::new().await?;

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
        // Alice's nonce 0; block 2 stacks on block 1's pending state). The value is not asserted, so
        // it is kept comfortably below Alice's remaining balance.
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

    let receipt = provider
        .get_transaction_receipt(setup.txn_details.alice_eth_transfer_hash)
        .await?;
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
    let count1_result: Bytes = setup
        .harness
        .rpc_client()?
        .request("eth_call", (setup.count1(), "pending"))
        .await?;
    let count1 = U256::from_be_slice(&count1_result);
    assert_eq!(count1, U256::from(2));

    let count2_result: Bytes = setup
        .harness
        .rpc_client()?
        .request("eth_call", (setup.count2(), "pending"))
        .await?;
    let count2 = U256::from_be_slice(&count2_result);
    assert_eq!(count2, U256::from(2));

    Ok(())
}

#[tokio::test]
async fn test_eth_estimate_gas() -> Result<()> {
    let setup = TestSetup::new().await?;

    setup.send_test_payloads().await?;

    // estimate_gas for a simple call to the deployed counter
    let gas: U256 = setup
        .harness
        .rpc_client()?
        .request("eth_estimateGas", (setup.count1(), "pending"))
        .await?;
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

    // Our `TransactionWithLogs` flattens the transaction and adds `logs` + `gas_used`.
    let tx = &notif["params"]["result"];
    assert!(tx.is_object(), "expected a full transaction object, got: {tx:?}");
    assert!(tx["hash"].is_string(), "expected flattened tx hash");
    assert!(tx["blockNumber"].is_string(), "expected flattened tx blockNumber");
    assert!(tx["logs"].is_array(), "expected logs array");
    // `TransactionWithLogs` is camelCase, so `gas_used` serializes as `gasUsed`. Emitted entries
    // always carry a known receipt, so it is populated.
    assert!(tx["gasUsed"].is_number(), "expected gasUsed to be populated");

    // Second flashblock delta: 5 transactions -> 5 separate full-tx messages.
    setup.send_flashblock(setup.create_second_payload()).await?;
    for _ in 0..5 {
        let notification = ws_stream.next().await.unwrap()?;
        let notif: serde_json::Value = serde_json::from_str(notification.to_text()?)?;
        assert_eq!(notif["params"]["subscription"], subscription_id);
        let tx = &notif["params"]["result"];
        assert!(tx["hash"].is_string() && tx["blockNumber"].is_string());
        assert!(tx["logs"].is_array());
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

fn make_log(address: Address, topics: Vec<B256>) -> PrimitiveLog {
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
        metadata: Metadata { block_number: 1, receipts, new_account_balances: HashMap::default() },
    }
}

#[tokio::test]
async fn test_get_logs_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    // No pending flashblock yet -> no pending logs.
    let logs =
        provider.get_logs(&Filter::default().select(BlockNumberOrTag::Pending)).await?;
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

    let block = provider
        .simulate(&simulate_call)
        .block_id(BlockNumberOrTag::Pending.into())
        .await?;
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
    assert_eq!(
        pending_block.header.parent_beacon_block_root,
        Some(TEST_PARENT_BEACON_BLOCK_ROOT)
    );
    assert_eq!(pending_block.header.withdrawals_root, Some(EMPTY_WITHDRAWALS));
    assert_eq!(pending_block.header.requests_hash, Some(EMPTY_REQUESTS_HASH));

    Ok(())
}
