//! Adapts the flashblocks state to the runner's pending-state seam.
//!
//! Only this crate can hold the impl: the trait belongs to `ethgas-node-runner` and the type to
//! `ethgas-reth-flashblocks`.

use std::sync::{Arc, Mutex};

use ethgas_node_runner::{PendingOverlay, PendingStateSource};
use ethgas_reth_flashblocks::{FlashblocksAPI, FlashblocksState, PendingBlocks};

/// Serves the flashblocks snapshot as the node's pending state.
#[derive(Debug)]
pub struct FlashblocksPendingState {
    state: Arc<FlashblocksState>,
    /// The overlay built for a snapshot, keyed on that snapshot's identity.
    ///
    /// Holding the `Arc` is what makes the `ptr_eq` below sound: a cached snapshot cannot be
    /// freed, so its address cannot be reused by a later one. See `docs/pending-state.md` §12.
    cached: Mutex<Option<(Arc<PendingBlocks>, PendingOverlay)>>,
}

impl FlashblocksPendingState {
    /// Wraps the shared flashblocks state.
    pub const fn new(state: Arc<FlashblocksState>) -> Self {
        Self { state, cached: Mutex::new(None) }
    }
}

impl PendingStateSource for FlashblocksPendingState {
    /// `None` when no snapshot is published, and when one records no anchor — a bundle alone does
    /// not say which state it was built on.
    fn pending_overlay(&self) -> Option<PendingOverlay> {
        // Locked before the snapshot is read, so observation and update are serialized together.
        // Reading first lets a thread holding an older snapshot overwrite a newer entry.
        let mut cached = self.cached.lock().unwrap_or_else(|err| err.into_inner());

        let guard = self.state.get_pending_blocks();
        let Some(pending) = guard.as_ref() else {
            // Released with the snapshot, or it pins a whole `PendingBlocks` for the process life.
            *cached = None;
            return None;
        };
        let Some(anchor) = pending.anchor() else {
            *cached = None;
            return None;
        };

        if let Some((snapshot, overlay)) = cached.as_ref() &&
            Arc::ptr_eq(snapshot, pending)
        {
            return Some(overlay.clone());
        }

        let overlay = PendingOverlay::from_bundle(
            anchor.clone(),
            pending.latest_block_number(),
            (*pending.bundle_state()).clone(),
        );
        *cached = Some((Arc::clone(pending), overlay.clone()));

        Some(overlay)
    }
}
