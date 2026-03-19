//! Flashblocks state management.

use std::sync::Arc;

use alloy_consensus::Header;
use arc_swap::{ArcSwapOption, Guard};
use reth::{
    chainspec::{ChainSpec, ChainSpecProvider},
    providers::{BlockReaderIdExt, StateProviderFactory},
};
use reth_ethereum_primitives::Block;
use reth_primitives_traits::RecoveredBlock;
use tokio::sync::{
    Mutex,
    broadcast::{self, Sender},
    mpsc,
};
use tracing::{debug, error, info};

use crate::{
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
        self.start_with_options(client, false);
    }

    /// Starts the flashblocks state processor with per-transaction state root simulation.
    ///
    /// When `simulate_state_root` is true, the processor computes a state root after
    /// each non-deposit transaction, storing the timing for metering purposes.
    pub fn start_with_options<Client>(&self, client: Client, simulate_state_root: bool)
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
            simulate_state_root,
            Arc::clone(&self.rx),
            chain_spec,
            self.flashblock_sender.clone(),
        );

        tokio::spawn(async move {
            state_processor.start().await;
        });
    }

    /// Handles a canonical block being received.
    pub fn on_canonical_block_received(&self, block: &RecoveredBlock<Block>) {
        let block_number = block.number;
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
