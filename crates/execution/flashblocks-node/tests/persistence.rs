//! The pending-state overlay over an anchor the database holds below its tip.
//!
//! `--engine.persistence-threshold 0` persists every canonical block at once. A snapshot whose
//! anchor the chain has passed is then served over a state reth rebuilds by reverting the
//! database from its tip to the anchor, and the executed block's number is what places the
//! anchor. With the anchor in memory, reth walks the in-memory chain instead and the number is
//! never read.

use alloy_primitives::U256;
use alloy_provider::Provider;
use ethgas_flashblocks_node::test_harness::{FlashblockBuilder, FlashblocksBuilderTestHarness};
use ethgas_node_runner::test_utils::Account;
use eyre::Result;

#[tokio::test]
async fn pending_state_overlays_an_anchor_persisted_below_the_database_tip() -> Result<()> {
    let mut test = FlashblocksBuilderTestHarness::with_persistence_threshold(0).await;
    let provider = test.node.provider();
    let alice = Account::Alice.address();
    let bob = Account::Bob.address();
    let charlie = Account::Charlie.address();

    // Block 1 is the anchor: canonical, persisted, and known to the processor.
    test.new_canonical_block(vec![]).await;
    test.node.wait_until_persisted(1).await?;
    let bob_at_anchor = provider.get_balance(bob).await?;

    // A two-block snapshot on that anchor, so the executed block's number is not the snapshot's
    // latest block number.
    test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
    let transfer = test.build_transaction_to_send_eth(Account::Alice, Account::Bob, 100_000);
    test.send_flashblock(
        FlashblockBuilder::new(&test, 1).with_transactions(vec![transfer]).build(),
    )
    .await;
    test.send_flashblock(FlashblockBuilder::new_base(&test).with_canonical_block_number(2).build())
        .await;
    assert_eq!(provider.get_balance(bob).pending().await?, bob_at_anchor + U256::from(100_000));

    // The chain moves on without the processor hearing of it: blocks 2 and 3 turn canonical
    // and are persisted, and block 2 pays Bob. The anchor is now below the database tip.
    let canonical_transfer =
        test.build_transaction_to_send_eth_with_nonce(Account::Charlie, Account::Bob, 777, 0);
    test.new_canonical_block_without_processing(vec![canonical_transfer]).await;
    test.new_canonical_block_without_processing(vec![]).await;
    test.node.wait_until_persisted(3).await?;
    assert_eq!(provider.get_block_number().await?, 3);
    assert_eq!(provider.get_balance(bob).await?, bob_at_anchor + U256::from(777));

    // `pending` is the snapshot on its anchor: block 2's payment is not in it, the snapshot's is.
    assert_eq!(provider.get_balance(bob).pending().await?, bob_at_anchor + U256::from(100_000));
    assert_eq!(provider.get_transaction_count(alice).pending().await?, 1);
    assert_eq!(provider.get_transaction_count(charlie).pending().await?, 0);
    assert_eq!(provider.get_transaction_count(charlie).await?, 1);

    Ok(())
}
