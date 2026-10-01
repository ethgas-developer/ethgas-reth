//! The flashblocks subscriptions over IPC, which serves every enabled module.

use std::time::Duration;

use alloy_primitives::{Address, B256, Bytes, U256, map::foldhash::HashMap};
use alloy_rpc_types_engine::PayloadId;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use jsonrpsee::{core::client::SubscriptionClientT, rpc_params};
use reth_ipc::client::IpcClientBuilder;
use serde_json::{Value, json};

/// The base flashblock of block 1, with no transactions.
fn empty_base_payload() -> FlashBlock {
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
        diff: ExecutionPayloadFlashblockDeltaV1::default(),
        metadata: Metadata {
            block_number: 1,
            receipts: HashMap::default(),
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

#[tokio::test]
async fn new_flashblocks_subscription_over_ipc() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let path = harness.ipc_path().expect("the node serves IPC").to_owned();
    let client = IpcClientBuilder::default().build(&path).await?;
    let mut subscription = client
        .subscribe::<Value, _>("eth_subscribe", rpc_params!["newFlashblocks"], "eth_unsubscribe")
        .await?;

    harness.send_flashblock(empty_base_payload()).await?;

    let block = tokio::time::timeout(Duration::from_secs(5), subscription.next())
        .await?
        .expect("the subscription is open")?;
    assert_eq!(block["number"], json!("0x1"));

    Ok(())
}
