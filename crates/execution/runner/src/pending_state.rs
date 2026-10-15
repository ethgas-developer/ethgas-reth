//! The seam between the node's `eth` API and whatever supplies pending state.
//!
//! Declared here so [`crate::eth_api::EthgasEthApi`] can answer the `pending` tag without the
//! runner depending on the flashblocks crate.

use std::{fmt::Debug, sync::Arc};

use alloy_consensus::Header;
use alloy_primitives::Sealed;
use reth_ethereum_primitives::EthPrimitives;
use reth_primitives_traits::{NodePrimitives, SealedHeader};
use reth_revm::db::BundleState;

/// Pending state as an overlay on canonical state.
#[derive(Debug, Clone)]
pub struct PendingOverlay<N: NodePrimitives = EthPrimitives> {
    /// The canonical header the bundle was executed against, sealed so a reorg cannot move it.
    pub anchor: Sealed<Header>,
    /// The header of the highest pending block the snapshot covers: the block environment that a
    /// call at `pending` runs in.
    pub latest_header: SealedHeader<N::BlockHeader>,
    /// The state changes of every pending block, read in place over the anchor's state. Shared
    /// with the snapshot, so cheap to clone.
    pub bundle: Arc<BundleState>,
}

impl PendingOverlay {
    /// Shapes a snapshot's bundle into an overlay on `anchor`, with `latest` as the pending
    /// block's header.
    pub fn from_bundle(
        anchor: Sealed<Header>,
        latest: Sealed<Header>,
        bundle: Arc<BundleState>,
    ) -> Self {
        let latest_header = SealedHeader::new(latest.inner().clone(), latest.hash());

        Self { anchor, latest_header, bundle }
    }
}

/// Supplies the pending state that `eth` methods answer the `pending` tag from.
pub trait PendingStateSource<N: NodePrimitives = EthPrimitives>: Debug + Send + Sync {
    /// The current overlay, or `None` when no pending state is published. The anchor and the
    /// bundle must come from one snapshot read.
    fn pending_overlay(&self) -> Option<PendingOverlay<N>>;
}
