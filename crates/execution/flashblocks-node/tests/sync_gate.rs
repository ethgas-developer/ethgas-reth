use ethgas_flashblocks_node::test_harness::{FlashblockBuilder, FlashblocksBuilderTestHarness};
use ethgas_reth_flashblocks::FlashblocksAPI;
use reth_network_p2p::sync::SyncState;

#[tokio::test]
async fn flashblocks_are_dropped_while_syncing() {
    let harness = FlashblocksBuilderTestHarness::new().await;

    harness.node.set_sync_state(SyncState::Syncing);
    harness.send_flashblock(FlashblockBuilder::new_base(&harness).build()).await;
    assert!(
        harness.flashblocks.get_pending_blocks().is_none(),
        "a flashblock received while syncing must not build pending state"
    );

    harness.node.set_sync_state(SyncState::Idle);
    harness.send_flashblock(FlashblockBuilder::new_base(&harness).build()).await;
    let pending = harness.flashblocks.get_pending_blocks();
    assert_eq!(
        pending.as_ref().map(|pending| pending.latest_block_number()),
        Some(1),
        "the first base flashblock after sync completes must rebuild pending state"
    );
}
