//! Integration tests for the builder's inclusion fee surface: the `ethgas` namespace,
//! `eth_maxPriorityFeePerGas` and `eth_gasPrice`.

use alloy_eips::eip1559::BaseFeeParams;
use alloy_primitives::U256;
use alloy_provider::Provider;
use ethgas_flashblocks_node::test_harness::{FlashblockBuilder, FlashblocksBuilderTestHarness};
use ethgas_node_runner::test_utils::Account;
use ethgas_reth_flashblocks::{
    fee::{FeeSource, InclusionPriorityFee},
    payload::InclusionFee,
};
use eyre::Result;

/// What reth's oracle suggests on a fresh chain.
const ORACLE_SUGGESTION: u64 = 1_000_000_000;

/// Distinct from [`ORACLE_SUGGESTION`].
const INCLUSION_FEE: u64 = 7_777_777_777;

fn inclusion_fee() -> InclusionFee {
    InclusionFee { priority_fee: U256::from(INCLUSION_FEE) }
}

async fn inclusion_priority_fee(
    harness: &FlashblocksBuilderTestHarness,
) -> Result<InclusionPriorityFee> {
    Ok(harness.node.rpc_client()?.request("ethgas_inclusionPriorityFee", ()).await?)
}

#[tokio::test]
async fn serves_the_inclusion_fee_from_the_latest_flashblock() -> Result<()> {
    let harness = FlashblocksBuilderTestHarness::new().await;
    let flashblock =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();

    harness.send_flashblock(flashblock).await;

    let served = inclusion_priority_fee(&harness).await?;
    assert_eq!(served.source, FeeSource::Builder);
    assert_eq!(served.max_priority_fee_per_gas, U256::from(INCLUSION_FEE));
    assert_eq!(served.block_number, Some(1));
    assert_eq!(served.flashblock_index, Some(0));
    assert!(served.age_ms.is_some());
    Ok(())
}

#[tokio::test]
async fn falls_back_to_the_oracle_when_the_builder_publishes_none() -> Result<()> {
    let harness = FlashblocksBuilderTestHarness::new().await;
    let with_fee =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();
    harness.send_flashblock(with_fee).await;

    harness.send_flashblock(FlashblockBuilder::new(&harness, 1).build()).await;

    let served = inclusion_priority_fee(&harness).await?;
    assert_eq!(served.source, FeeSource::Fallback);
    assert_eq!(served.max_priority_fee_per_gas, U256::from(ORACLE_SUGGESTION));
    assert_eq!(served.age_ms, None);
    assert_eq!(served.block_number, None);
    Ok(())
}

#[tokio::test]
async fn max_priority_fee_serves_the_inclusion_fee_while_held() -> Result<()> {
    let harness = FlashblocksBuilderTestHarness::new().await;
    let flashblock =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();
    harness.send_flashblock(flashblock).await;

    let held = harness.node.provider().get_max_priority_fee_per_gas().await?;
    assert_eq!(held, u128::from(INCLUSION_FEE));

    harness.send_flashblock(FlashblockBuilder::new(&harness, 1).build()).await;

    let released = harness.node.provider().get_max_priority_fee_per_gas().await?;
    assert_eq!(released, u128::from(ORACLE_SUGGESTION));
    Ok(())
}

#[tokio::test]
async fn gas_price_adds_the_inclusion_fee_to_the_next_base_fee_while_held() -> Result<()> {
    let harness = FlashblocksBuilderTestHarness::new().await;
    let latest = harness.node.latest_block();
    let base_fee = u128::from(latest.base_fee_per_gas.expect("london is active"));
    let next_base_fee = u128::from(
        latest.header().next_block_base_fee(BaseFeeParams::ethereum()).expect("london is active"),
    );
    let flashblock =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();
    harness.send_flashblock(flashblock).await;

    let held = harness.node.provider().get_gas_price().await?;
    assert_eq!(held, next_base_fee + u128::from(INCLUSION_FEE));

    harness.send_flashblock(FlashblockBuilder::new(&harness, 1).build()).await;

    let released = harness.node.provider().get_gas_price().await?;
    assert_eq!(released, base_fee + u128::from(ORACLE_SUGGESTION));
    Ok(())
}

/// The builder's fee prices the block it is building. Once that block is sealed the node serves its
/// own suggestion, however fresh the fee is.
#[tokio::test]
async fn the_inclusion_fee_expires_with_its_block() -> Result<()> {
    let mut harness = FlashblocksBuilderTestHarness::new().await;
    let flashblock =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();
    harness.send_flashblock(flashblock).await;
    assert_eq!(inclusion_priority_fee(&harness).await?.source, FeeSource::Builder);

    harness.new_canonical_block(vec![]).await;

    let served = inclusion_priority_fee(&harness).await?;
    assert_eq!(served.source, FeeSource::Fallback);
    assert_eq!(served.block_number, None);
    Ok(())
}

/// `eth_fillTransaction` gives a request that sets no fee the builder's inclusion fee as its
/// priority fee, the value `eth_maxPriorityFeePerGas` serves.
#[tokio::test]
async fn fill_transaction_takes_the_inclusion_fee_while_held() -> Result<()> {
    let harness = FlashblocksBuilderTestHarness::new().await;
    let flashblock =
        FlashblockBuilder::new_base(&harness).with_inclusion_fee(inclusion_fee()).build();
    harness.send_flashblock(flashblock).await;

    let request = serde_json::json!({
        "from": Account::Alice.address(),
        "to": Account::Bob.address(),
        "value": "0x1",
    });
    let filled: serde_json::Value =
        harness.node.rpc_client()?.request("eth_fillTransaction", (request,)).await?;

    assert_eq!(filled["tx"]["maxPriorityFeePerGas"], format!("{INCLUSION_FEE:#x}"), "{filled}");
    Ok(())
}
