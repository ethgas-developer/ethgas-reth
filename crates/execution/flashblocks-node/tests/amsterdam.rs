//! Pending state on a chain where Amsterdam is active from genesis: the slot number the producer
//! sends reaches the pending header, the execution of flashblock transactions and calls at
//! `pending`.

use std::{sync::Arc, time::Duration};

use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, bytes};
use alloy_provider::Provider;
use alloy_rpc_types_eth::TransactionRequest;
use ethgas_flashblocks_node::test_harness::{FlashblockBuilder, FlashblocksBuilderTestHarness};
use ethgas_node_runner::test_utils::{
    Account, build_test_genesis_with_amsterdam_at, slot_number_at,
};
use ethgas_reth_flashblocks::FlashblocksAPI;
use eyre::Result;
use reth_chainspec::{ChainSpec, EthChainSpec};
use reth_provider::ChainSpecProvider;
use reth_transaction_pool::test_utils::TransactionBuilder;
use serde_json::Value;
use tokio::time::timeout;

/// Genesis is at timestamp 1 and the fixture's first block 2 s later.
const FIRST_BLOCK_SLOT: u64 = slot_number_at(3);

/// Creation code that returns the slot number: `SLOTNUM PUSH0 MSTORE PUSH1 32 PUSH0 RETURN`.
const RETURN_SLOT_NUMBER: Bytes = bytes!("0x4b5f5260205ff3");

/// Creation code that stores the slot number at slot 0 of the new account:
/// `SLOTNUM PUSH0 SSTORE STOP`.
const STORE_SLOT_NUMBER: Bytes = bytes!("0x4b5f5500");

async fn amsterdam_harness() -> FlashblocksBuilderTestHarness {
    let genesis = build_test_genesis_with_amsterdam_at(Some(0));
    FlashblocksBuilderTestHarness::with_chain_spec(Arc::new(ChainSpec::from(genesis))).await
}

#[tokio::test]
async fn the_pending_header_carries_the_slot_and_no_block_access_list_hash() -> Result<()> {
    let harness = amsterdam_harness().await;
    let mut snapshots = harness.flashblocks.subscribe_to_flashblocks();
    harness.send_flashblock(FlashblockBuilder::new_base(&harness).build()).await;
    timeout(Duration::from_secs(5), snapshots.recv()).await??;

    let block: Value = harness
        .node
        .provider()
        .raw_request("eth_getBlockByNumber".into(), (BlockNumberOrTag::Pending, false))
        .await?;
    assert_eq!(block["number"], "0x1");
    assert_eq!(block["slotNumber"], format!("{FIRST_BLOCK_SLOT:#x}"));
    assert!(block.get("blockAccessListHash").is_none(), "{block}");
    Ok(())
}

#[tokio::test]
async fn a_call_at_pending_reads_the_flashblock_slot() -> Result<()> {
    let harness = amsterdam_harness().await;
    let mut snapshots = harness.flashblocks.subscribe_to_flashblocks();
    harness.send_flashblock(FlashblockBuilder::new_base(&harness).build()).await;
    timeout(Duration::from_secs(5), snapshots.recv()).await??;

    let call = TransactionRequest {
        from: Some(Account::Alice.address()),
        to: Some(TxKind::Create),
        input: RETURN_SLOT_NUMBER.into(),
        ..Default::default()
    };
    let returned =
        harness.node.provider().call(call).block(BlockNumberOrTag::Pending.into()).await?;
    assert_eq!(B256::from_slice(&returned), B256::from(U256::from(FIRST_BLOCK_SLOT)));
    Ok(())
}

#[tokio::test]
async fn pending_storage_holds_the_slot_a_flashblock_transaction_read() -> Result<()> {
    let harness = amsterdam_harness().await;
    let mut snapshots = harness.flashblocks.subscribe_to_flashblocks();
    harness.send_flashblock(FlashblockBuilder::new_base(&harness).build()).await;

    let mut creation = TransactionBuilder::default()
        .signer(FlashblocksBuilderTestHarness::decode_private_key(Account::Alice))
        .chain_id(harness.provider.chain_spec().chain_id())
        .nonce(0)
        .gas_limit(1_000_000)
        .max_fee_per_gas(1_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000)
        .input(STORE_SLOT_NUMBER);
    creation.to = TxKind::Create;
    harness
        .send_flashblock(
            FlashblockBuilder::new(&harness, 1)
                .with_transactions(vec![creation.into_eip1559()])
                .build(),
        )
        .await;
    timeout(Duration::from_secs(5), snapshots.recv()).await??;
    timeout(Duration::from_secs(5), snapshots.recv()).await??;

    let created: Address = Account::Alice.address().create(0);
    let stored = harness
        .node
        .provider()
        .get_storage_at(created, U256::ZERO)
        .block_id(BlockNumberOrTag::Pending.into())
        .await?;
    assert_eq!(stored, U256::from(FIRST_BLOCK_SLOT));
    Ok(())
}
