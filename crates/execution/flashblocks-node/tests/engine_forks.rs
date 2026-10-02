//! The harness's engine client builds canonical blocks on every chain shape the suite uses:
//! Prague only, Osaka, Amsterdam at genesis, and Amsterdam from a timestamp after genesis. Each
//! fork takes its own `engine_getPayload` and `engine_newPayload` versions, and an Amsterdam
//! block takes a slot number in its payload attributes.

use std::sync::Arc;

use alloy_consensus::Header;
use alloy_genesis::Genesis;
use ethgas_node_runner::test_utils::{
    TestHarness, build_prague_test_genesis, build_test_genesis_with_amsterdam_at, slot_number_at,
};
use eyre::{Result, eyre};
use reth_chainspec::ChainSpec;
use reth_provider::HeaderProvider;

/// Builds three blocks on `genesis` and returns their headers. Genesis is at timestamp 1 and
/// blocks follow 2 s apart, so blocks 1, 2 and 3 are at 3, 5 and 7.
async fn three_blocks(genesis: Genesis) -> Result<Vec<Header>> {
    let harness =
        TestHarness::builder().with_chain_spec(Arc::new(ChainSpec::from(genesis))).build().await?;
    harness.advance_chain(3).await?;
    let provider = harness.blockchain_provider();
    (1..=3)
        .map(|number| {
            provider.header_by_number(number)?.ok_or_else(|| eyre!("block {number} is missing"))
        })
        .collect()
}

fn assert_before_amsterdam(header: &Header) {
    assert_eq!(header.slot_number, None, "block {}", header.number);
    assert_eq!(header.block_access_list_hash, None, "block {}", header.number);
}

fn assert_amsterdam(header: &Header) {
    assert_eq!(
        header.slot_number,
        Some(slot_number_at(header.timestamp)),
        "block {}",
        header.number
    );
    assert!(header.block_access_list_hash.is_some(), "block {}", header.number);
}

#[tokio::test]
async fn the_engine_builds_prague_blocks() -> Result<()> {
    three_blocks(build_prague_test_genesis()).await?.iter().for_each(assert_before_amsterdam);
    Ok(())
}

#[tokio::test]
async fn the_engine_builds_osaka_blocks() -> Result<()> {
    three_blocks(build_test_genesis_with_amsterdam_at(None))
        .await?
        .iter()
        .for_each(assert_before_amsterdam);
    Ok(())
}

#[tokio::test]
async fn the_engine_builds_amsterdam_blocks_from_genesis() -> Result<()> {
    three_blocks(build_test_genesis_with_amsterdam_at(Some(0)))
        .await?
        .iter()
        .for_each(assert_amsterdam);
    Ok(())
}

#[tokio::test]
async fn the_engine_builds_blocks_across_the_amsterdam_fork() -> Result<()> {
    let headers = three_blocks(build_test_genesis_with_amsterdam_at(Some(5))).await?;
    assert_before_amsterdam(&headers[0]);
    headers[1..].iter().for_each(assert_amsterdam);
    Ok(())
}
