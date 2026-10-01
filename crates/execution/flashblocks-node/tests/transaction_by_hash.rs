//! The three sources of `eth_getTransactionByHash`, in the order the node asks them: the sealed
//! chain, this node's mempool, the flashblocks snapshot.

use alloy_consensus::TxType;
use alloy_primitives::{Address, B256, Bytes, U256, keccak256, map::foldhash::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, DoubleCounter};
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use reth_ethereum_primitives::Receipt;

/// The base flashblock of block 1, carrying `tx` alone.
fn base_payload(tx: Bytes) -> FlashBlock {
    let mut receipts = HashMap::default();
    receipts.insert(
        keccak256(&tx),
        Receipt {
            tx_type: TxType::Eip1559,
            success: true,
            cumulative_gas_used: 100_000,
            logs: vec![],
        },
    );
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
        diff: ExecutionPayloadFlashblockDeltaV1 { transactions: vec![tx], ..Default::default() },
        metadata: Metadata {
            block_number: 1,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

/// A transaction in this node's mempool is answered from the mempool until its block is sealed,
/// even after a flashblock includes it. Its receipt is the pre-confirmation signal.
#[tokio::test]
async fn a_pooled_transaction_reports_no_block_by_hash_but_its_receipt_is_pre_confirmed()
-> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();
    let (deployment, _, hash) =
        Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;

    let _pooled = provider.send_raw_transaction(&deployment).await?;
    assert!(provider.get_transaction_receipt(hash).await?.is_none());

    harness.send_flashblock(base_payload(deployment)).await?;

    let by_hash = provider.get_transaction_by_hash(hash).await?.expect("the pool knows it");
    assert_eq!(by_hash.block_hash, None);
    assert_eq!(by_hash.block_number, None, "the mempool copy answers until the block is sealed");
    assert_eq!(by_hash.transaction_index, None);

    let receipt = provider.get_transaction_receipt(hash).await?.expect("pre-confirmed receipt");
    assert_eq!(receipt.block_number, Some(1));
    assert_eq!(receipt.block_hash, Some(B256::ZERO));

    Ok(())
}

/// A transaction this node's mempool has never held is answered from the flashblock, with its
/// block number and index.
#[tokio::test]
async fn a_transaction_only_the_flashblock_knows_reports_its_block_number() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();
    let (deployment, _, hash) =
        Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;
    assert!(provider.get_transaction_by_hash(hash).await?.is_none());

    harness.send_flashblock(base_payload(deployment)).await?;

    let by_hash = provider.get_transaction_by_hash(hash).await?.expect("the snapshot knows it");
    assert_eq!(by_hash.block_hash, None);
    assert_eq!(by_hash.block_number, Some(1));
    assert_eq!(by_hash.transaction_index, Some(0));

    Ok(())
}

/// A mined transaction outranks a snapshot that a canonical commit has not yet cleared.
#[tokio::test]
async fn a_mined_transaction_is_answered_from_its_block_over_a_stale_snapshot() -> Result<()> {
    let harness = FlashblocksHarness::manual_canonical().await?;
    let provider = harness.provider();
    let (deployment, _, hash) =
        Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;

    harness.send_flashblock(base_payload(deployment.clone())).await?;
    let pending = provider.get_transaction_by_hash(hash).await?.expect("the snapshot knows it");
    assert_eq!(pending.block_hash, None);

    // The snapshot is not reconciled, so it still claims the transaction as pending.
    harness.build_block_from_transactions(vec![deployment]).await?;
    let block_1 = provider.get_block_by_number(1.into()).await?.expect("block 1");

    let mined = provider.get_transaction_by_hash(hash).await?.expect("the block knows it");
    assert_eq!(mined.block_hash, Some(block_1.header.hash));
    assert_eq!(mined.block_number, Some(1));

    Ok(())
}
