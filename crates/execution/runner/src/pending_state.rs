//! The seam between the node's `eth` API and whatever supplies pending state.
//!
//! Declared here so [`crate::eth_api::EthgasEthApi`] can answer the `pending` tag without the
//! runner depending on the flashblocks crate.

use std::{fmt::Debug, sync::Arc};

use alloy_consensus::{BlockBody, Header};
use alloy_primitives::Sealed;
use reth_chain_state::ExecutedBlock;
use reth_ethereum_primitives::{Block, Receipt};
use reth_evm::execute::BlockExecutionOutput;
use reth_primitives_traits::{RecoveredBlock, SealedBlock, SealedHeader};
use reth_revm::db::BundleState;
use reth_trie_common::ComputedTrieData;

/// Pending state as an overlay on canonical state.
#[derive(Debug, Clone)]
pub struct PendingOverlay {
    /// The canonical header the bundle was executed against, sealed so a reorg cannot move it.
    pub anchor: Sealed<Header>,
    /// The highest block number the snapshot covers.
    pub latest_block_number: u64,
    /// The pending state, shaped for [`reth_chain_state::BlockState`]. Cheap to clone.
    pub executed: ExecutedBlock,
}

impl PendingOverlay {
    /// Shapes a bundle into an overlay on `anchor`. Copies the bundle, so build once per snapshot.
    ///
    /// The executed block carries the anchor's header and an empty body on purpose; see
    /// `doc/pending-state.md` §12.
    pub fn from_bundle(
        anchor: Sealed<Header>,
        latest_block_number: u64,
        bundle: BundleState,
    ) -> Self {
        let (header, hash) = anchor.clone().into_parts();
        let sealed = SealedBlock::<Block>::from_sealed_parts(
            SealedHeader::new(header, hash),
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
pub trait PendingStateSource: Debug + Send + Sync {
    /// The current overlay, or `None` when no pending state is published. The anchor and the
    /// bundle must come from one snapshot read.
    fn pending_overlay(&self) -> Option<PendingOverlay>;
}
