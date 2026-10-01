//! Adapts the flashblocks state to the runner's pending-state seam.

use std::sync::{Arc, Mutex};

use ethgas_node_runner::{PendingOverlay, PendingStateSource};
use ethgas_reth_flashblocks::{FlashblocksAPI, FlashblocksState, PendingBlocks};

/// Serves the flashblocks snapshot as the node's pending state.
#[derive(Debug)]
pub struct FlashblocksPendingState {
    state: Arc<FlashblocksState>,
    /// The overlay built for a snapshot, keyed on that snapshot's identity.
    cached: Mutex<Option<(Arc<PendingBlocks>, PendingOverlay)>>,
}

impl FlashblocksPendingState {
    pub const fn new(state: Arc<FlashblocksState>) -> Self {
        Self { state, cached: Mutex::new(None) }
    }
}

impl PendingStateSource for FlashblocksPendingState {
    fn pending_overlay(&self) -> Option<PendingOverlay> {
        let mut cached = self.cached.lock().unwrap_or_else(|err| err.into_inner());

        let guard = self.state.get_pending_blocks();
        let Some(pending) = guard.as_ref() else {
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
            pending.latest_header(),
            (*pending.bundle_state()).clone(),
        );
        *cached = Some((Arc::clone(pending), overlay.clone()));

        Some(overlay)
    }
}
