//! Integration tests covering the Flashblocks RPC surface area.
//!
//! These tests exercise the flashblocks-extended RPC endpoints (pending block,
//! pending balance, pending transaction receipt, eth_call with flashblock state, etc.)
//! by launching a full local Ethereum node with the flashblocks test extension.

use std::str::FromStr;

use DoubleCounter::DoubleCounterInstance;
use alloy_eips::BlockNumberOrTag;
use alloy_provider::network::TransactionResponse;
use alloy_primitives::{Address, B256, Bytes, TxHash, U256, address, b256, bytes};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::TransactionRequest;
use ethgas_node_runner::test_utils::{Account, DoubleCounter};
use ethgas_reth_flashblocks::{
    payload::{
        ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
    },
    test_harness::FlashblocksHarness,
};
use eyre::Result;
use alloy_primitives::map::foldhash::HashMap;
use reth_ethereum_primitives::Receipt;

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

        // Alice's ETH transfer at nonce 0
        let (eth_transfer_tx, eth_transfer_hash) = alice
            .sign_txn_request(
                TransactionRequest::default()
                    .to(bob.address())
                    .value(U256::from_str("999999999000000000000000").unwrap())
                    .gas_limit(100_000)
                    .nonce(0),
            )
            .expect("should be able to sign eth transfer txn");

        // Balance transfer: alice sends PENDING_BALANCE wei to TEST_ADDRESS at nonce 1
        let (balance_transfer_tx, _) = alice
            .sign_txn_request(
                TransactionRequest::default()
                    .to(TEST_ADDRESS)
                    .value(U256::from(PENDING_BALANCE))
                    .gas_limit(21_000)
                    .nonce(1),
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
                receipts: HashMap::default(),
                new_account_balances: HashMap::default(),
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

    // eth_call count1() should return 1 (the counter was incremented once)
    let count1_result: Bytes = setup
        .harness
        .rpc_client()?
        .request("eth_call", (setup.count1(), "pending"))
        .await?;
    let count1 = U256::from_be_slice(&count1_result);
    assert_eq!(count1, U256::from(1));

    // eth_call count2() should also return 1
    let count2_result: Bytes = setup
        .harness
        .rpc_client()?
        .request("eth_call", (setup.count2(), "pending"))
        .await?;
    let count2 = U256::from_be_slice(&count2_result);
    assert_eq!(count2, U256::from(1));

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
