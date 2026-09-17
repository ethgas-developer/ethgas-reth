//! Flashblocks state management.

use std::sync::{
    Arc,
    atomic::{AtomicU64, Ordering},
};

use alloy_consensus::Header;
use arc_swap::{ArcSwapOption, Guard};
use reth_chainspec::{ChainSpec, ChainSpecProvider};
use reth_ethereum_primitives::Block;
use reth_primitives_traits::RecoveredBlock;
use reth_provider::{BlockReaderIdExt, StateProviderFactory};
use tokio::sync::{
    Mutex,
    broadcast::{self, Sender},
    mpsc,
};
use tracing::{debug, error, info};

use crate::{
    metrics::Metrics,
    payload::FlashBlock,
    pending_blocks::PendingBlocks,
    processor::{StateProcessor, StateUpdate},
    traits::{FlashblocksAPI, FlashblocksReceiver},
};

// Buffer 4s of flashblocks for flashblock_sender
const BUFFER_SIZE: usize = 20;

/// Manages the pending flashblock state and processes incoming updates.
///
/// Unlike the old generic `FlashblocksState<Client>`, this version defers client binding
/// to [`start`](Self::start), allowing the state to be created before the node is launched.
#[derive(Debug)]
pub struct FlashblocksState {
    pending_blocks: Arc<ArcSwapOption<PendingBlocks>>,
    queue: mpsc::UnboundedSender<StateUpdate>,
    rx: Arc<Mutex<mpsc::UnboundedReceiver<StateUpdate>>>,
    flashblock_sender: Sender<Arc<PendingBlocks>>,
    max_pending_blocks_depth: u64,
    last_canonical_block: Arc<AtomicU64>,
    metrics: Metrics,
}

impl FlashblocksState {
    /// Creates a new flashblocks state manager.
    ///
    /// The state is created without a client. Call [`start`](Self::start) with a client
    /// to spawn the state processor after the node is launched.
    pub fn new(max_pending_blocks_depth: u64) -> Self {
        let (tx, rx) = mpsc::unbounded_channel::<StateUpdate>();
        let pending_blocks: Arc<ArcSwapOption<PendingBlocks>> = Arc::new(ArcSwapOption::new(None));
        let (flashblock_sender, _) = broadcast::channel(BUFFER_SIZE);

        Self {
            pending_blocks,
            queue: tx,
            rx: Arc::new(Mutex::new(rx)),
            flashblock_sender,
            max_pending_blocks_depth,
            last_canonical_block: Arc::new(AtomicU64::new(0)),
            metrics: Metrics::default(),
        }
    }

    /// Starts the flashblocks state processor with the given client.
    ///
    /// This spawns a background task that processes canonical blocks and flashblocks.
    /// Should be called after the node is launched and the provider is available.
    pub fn start<Client>(&self, client: Client)
    where
        Client: StateProviderFactory
            + ChainSpecProvider<ChainSpec = ChainSpec>
            + BlockReaderIdExt<Header = Header>
            + Clone
            + 'static,
    {
        let chain_spec = client.chain_spec();
        let state_processor = StateProcessor::new(
            client,
            Arc::clone(&self.pending_blocks),
            self.max_pending_blocks_depth,
            Arc::clone(&self.rx),
            chain_spec,
            self.flashblock_sender.clone(),
            Arc::clone(&self.last_canonical_block),
        );

        tokio::spawn(async move {
            state_processor.start().await;
        });
    }

    /// Drops the published snapshot when it is anchored more than `max_pending_blocks_depth`
    /// blocks behind `canonical_block_number`.
    ///
    /// [`StateProcessor`] enforces the same bound, but only once it reaches the matching queue
    /// entry. Repeating it on the receiving task is what bounds staleness by chain progress
    /// rather than by processor progress.
    fn drop_pending_behind(&self, canonical_block_number: u64) {
        let published = self.pending_blocks.load();
        let Some(stale) = published.as_ref() else { return };

        // Measured from the earliest pending block, matching the bound the processor and the
        // reconciler apply, so the three cannot disagree about which snapshots survive.
        let earliest_pending_block = stale.earliest_block_number();
        if canonical_block_number.saturating_sub(earliest_pending_block) <=
            self.max_pending_blocks_depth
        {
            return;
        }

        // Clear only the snapshot that was judged. The processor publishes concurrently, and
        // anything published after the load above is anchored on a later tip than this one.
        // Losing that race costs nothing, because an absent snapshot is always safe to serve.
        let current = self.pending_blocks.compare_and_swap(&published, None);
        if !current.as_ref().is_some_and(|current| Arc::ptr_eq(current, stale)) {
            return;
        }

        debug!(
            message = "dropping pending snapshot anchored too far behind the canonical tip",
            canonical_block_number,
            earliest_pending_block,
            max_depth = self.max_pending_blocks_depth,
        );
        self.metrics.pending_drop_stale.increment(1);
    }

    /// Handles a canonical block being received.
    pub fn on_canonical_block_received(&self, block: &RecoveredBlock<Block>) {
        let block_number = block.number;
        // `store`, not `fetch_max`: a reorg moves the tip down, and holding the higher height
        // would suppress every flashblock built on the replacement chain.
        self.last_canonical_block.store(block_number, Ordering::Relaxed);
        self.drop_pending_behind(block_number);
        match self.queue.send(StateUpdate::Canonical(block.clone())) {
            Ok(_) => {
                info!(message = "added canonical block to processing queue", block_number)
            }
            Err(e) => {
                error!(message = "could not add canonical block to processing queue", block_number, error = %e);
            }
        }
    }
}

impl FlashblocksReceiver for FlashblocksState {
    fn on_flashblock_received(&self, flashblock: FlashBlock) {
        let flashblock_index = flashblock.index;
        let block_number = flashblock.metadata.block_number;

        // Keep superseded payloads out of the queue entirely. The processor repeats this check
        // for payloads that were fresh on arrival but went stale while queued.
        if block_number <= self.last_canonical_block.load(Ordering::Relaxed) {
            debug!(
                message = "dropping flashblock for an already canonical block",
                block_number, flashblock_index,
            );
            self.metrics.flashblock_superseded.increment(1);
            return;
        }

        match self.queue.send(StateUpdate::Flashblock(flashblock)) {
            Ok(_) => {
                debug!(
                    message = "added flashblock to processing queue",
                    block_number, flashblock_index,
                );
            }
            Err(e) => {
                error!(message = "could not add flashblock to processing queue", block_number, flashblock_index, error = %e);
            }
        }
    }
}

impl FlashblocksAPI for FlashblocksState {
    fn get_pending_blocks(&self) -> Guard<Option<Arc<PendingBlocks>>> {
        self.pending_blocks.load()
    }

    fn subscribe_to_flashblocks(&self) -> broadcast::Receiver<Arc<PendingBlocks>> {
        self.flashblock_sender.subscribe()
    }
}

impl Default for FlashblocksState {
    fn default() -> Self {
        Self::new(10)
    }
}

impl FlashblocksState {
    /// Sets the pending blocks directly for testing purposes.
    ///
    /// This bypasses the normal flashblock processing pipeline and allows
    /// tests to inject a pre-built `PendingBlocks` state.
    pub fn set_pending_blocks_for_testing(&self, pending_blocks: Option<PendingBlocks>) {
        self.pending_blocks.store(pending_blocks.map(Arc::new));
    }
}

#[cfg(test)]
mod tests {
    use reth_primitives_traits::Block as BlockT;

    use super::*;
    use crate::payload::Metadata;

    fn canonical_block(number: u64) -> RecoveredBlock<Block> {
        let block =
            Block { header: Header { number, ..Default::default() }, body: Default::default() };
        BlockT::try_into_recovered(block).expect("recover empty block")
    }

    fn flashblock(block_number: u64) -> FlashBlock {
        FlashBlock {
            metadata: Metadata { block_number, ..Default::default() },
            ..Default::default()
        }
    }

    /// A flashblock whose block is already canonical must never reach the queue: applying one
    /// fails sequence validation and clears the snapshot built for a later block.
    #[tokio::test]
    async fn flashblocks_at_or_behind_canonical_are_dropped() {
        let state = FlashblocksState::new(3);
        state.on_canonical_block_received(&canonical_block(10));

        state.on_flashblock_received(flashblock(9));
        state.on_flashblock_received(flashblock(10));
        state.on_flashblock_received(flashblock(11));

        let mut rx = state.rx.lock().await;
        let queued: Vec<StateUpdate> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

        assert_eq!(queued.len(), 2, "only the canonical block and flashblock 11 should queue");
        assert!(matches!(queued[0], StateUpdate::Canonical(_)));
        match &queued[1] {
            StateUpdate::Flashblock(fb) => assert_eq!(fb.metadata.block_number, 11),
            other => panic!("unexpected update queued: {other:?}"),
        }
    }

    /// A reorg moves the tip down, so the gate must reopen for the replacement chain.
    #[tokio::test]
    async fn tip_moving_back_reopens_the_gate() {
        let state = FlashblocksState::new(3);
        state.on_canonical_block_received(&canonical_block(10));
        state.on_canonical_block_received(&canonical_block(8));

        state.on_flashblock_received(flashblock(9));

        let mut rx = state.rx.lock().await;
        let queued: Vec<StateUpdate> = std::iter::from_fn(|| rx.try_recv().ok()).collect();

        assert!(
            matches!(queued.last(), Some(StateUpdate::Flashblock(fb)) if fb.metadata.block_number == 9),
            "flashblock 9 should queue once the tip moves back to 8"
        );
    }
}
