//! `pending` must never be served from reth's transaction pool.
//!
//! The eth methods this node overrides answer `pending` from the flashblocks snapshot. Every other
//! method resolves `pending` through reth, which by default builds a block out of its own pool.
//! `EthgasEthApi` overrides `local_pending_state` and `local_pending_block` so those methods read
//! canonical state instead. These tests hold that contract from the RPC surface.

use alloy_eips::BlockId;
use alloy_primitives::{Bytes, U256};
use alloy_provider::Provider;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, DoubleCounter};
use eyre::Result;

/// A contract deployed only in the pool must not exist at `pending`.
///
/// `eth_getCode` is not one of the overridden methods, so it lands on the locally built pending
/// block. If that block were pool-built the deployment would already be visible there, in a block
/// no builder ever produced.
#[tokio::test]
async fn pool_only_deployment_is_invisible_at_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    let (deployment_tx, contract_address, _) =
        Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;

    assert_eq!(
        provider.get_code_at(contract_address).block_id(BlockId::latest()).await?,
        Bytes::new(),
        "the contract must not exist before the deployment is sent"
    );

    let _pending = provider.send_raw_transaction(&deployment_tx).await?;

    assert_eq!(
        provider.get_code_at(contract_address).block_id(BlockId::pending()).await?,
        Bytes::new(),
        "pending must not carry a contract that only the pool knows about"
    );

    Ok(())
}

/// Storage written only by a pool transaction must not be readable at `pending`.
///
/// This guards the state path rather than the block path: `local_pending_state` returning `None` is
/// what makes `pending` resolve against canonical state. `DoubleCounter` initialises `count1` to 1,
/// so a pool-built pending state would report 1 for a contract the chain has never seen.
#[tokio::test]
async fn storage_written_only_in_the_pool_is_invisible_at_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();

    let (deployment_tx, contract_address, _) =
        Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;
    let _pending = provider.send_raw_transaction(&deployment_tx).await?;

    let count1 =
        provider.get_storage_at(contract_address, U256::ZERO).block_id(BlockId::pending()).await?;

    assert!(
        count1.is_zero(),
        "pending storage must come from canonical state, not from a pool-built block"
    );

    Ok(())
}
