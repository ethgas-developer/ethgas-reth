//! The seam between the node's `eth` API and whatever supplies pending state.
//!
//! Declared here so [`crate::eth_api::EthgasEthApi`] can answer the `pending` tag without the
//! runner depending on the flashblocks crate.

use std::{fmt::Debug, sync::Arc};

use alloy_consensus::{BlockBody, Header};
use alloy_primitives::{B256, Sealed};
use reth_chain_state::ExecutedBlock;
use reth_ethereum_primitives::{Block, EthPrimitives, Receipt};
use reth_evm::execute::BlockExecutionOutput;
use reth_primitives_traits::{NodePrimitives, RecoveredBlock, SealedBlock, SealedHeader};
use reth_revm::db::BundleState;
use reth_trie_common::ComputedTrieData;

/// Pending state as an overlay on canonical state.
#[derive(Debug, Clone)]
pub struct PendingOverlay<N: NodePrimitives = EthPrimitives> {
    /// The canonical header the bundle was executed against, sealed so a reorg cannot move it.
    pub anchor: Sealed<Header>,
    /// The highest block number the snapshot covers.
    pub latest_block_number: u64,
    /// The pending state as an executed block, ready to append to the anchor's state. Cheap to
    /// clone.
    pub executed: ExecutedBlock<N>,
}

impl PendingOverlay {
    /// Shapes a bundle into an overlay on `anchor`. Copies the bundle, so build once per snapshot.
    ///
    /// The executed block is a child of the anchor, as appending it to the anchor's state
    /// requires. Pending blocks have no hash on this node, so it carries a zero sentinel, which
    /// the overlay serves only for `BLOCKHASH` of that block number. Nothing reads its empty body.
    pub fn from_bundle(
        anchor: Sealed<Header>,
        latest_block_number: u64,
        bundle: BundleState,
    ) -> Self {
        let header =
            Header { parent_hash: anchor.hash(), number: anchor.number + 1, ..Default::default() };
        let sealed = SealedBlock::<Block>::from_sealed_parts(
            SealedHeader::new(header, B256::ZERO),
            BlockBody::default(),
        );
        let output = BlockExecutionOutput::<Receipt> { result: Default::default(), state: bundle };

        let executed = ExecutedBlock::new(
            Arc::new(RecoveredBlock::new_sealed(sealed, Vec::new())),
            Arc::new(output),
            ComputedTrieData::default(),
        );

        Self { anchor, latest_block_number, executed }
    }
}

/// Supplies the pending state that `eth` methods answer the `pending` tag from.
pub trait PendingStateSource<N: NodePrimitives = EthPrimitives>: Debug + Send + Sync {
    /// The current overlay, or `None` when no pending state is published. The anchor and the
    /// bundle must come from one snapshot read.
    fn pending_overlay(&self) -> Option<PendingOverlay<N>>;
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{BlockHeader, Sealable};
    use alloy_eips::BlockNumHash;

    use super::*;

    /// reth derives the anchor from the appended block's parent number and hash.
    #[test]
    fn test_executed_block_is_a_child_of_the_anchor() {
        let anchor = Header { number: 7, ..Default::default() }.seal_slow();

        let overlay = PendingOverlay::from_bundle(anchor.clone(), 9, BundleState::default());
        let block = overlay.executed.recovered_block();

        assert_eq!(block.parent_num_hash(), BlockNumHash::new(7, anchor.hash()));
        assert_eq!(block.hash(), B256::ZERO, "a pending block has no hash");
    }
}
