//! Flashblocks support for the Ethgas node.
//!
//! Consumes flashblocks from the ethgas-builder over a WebSocket, replays them into a
//! pending state on top of the canonical tip, and serves that state through an
//! `eth_` RPC and pub-sub surface that overrides reth's own.

pub mod config;
pub use config::FlashblocksConfig;

pub mod state;
pub use state::FlashblocksState;

pub mod error;
pub use error::{
    BuildError, ExecutionError, ProtocolError, ProviderError, Result, StateProcessorError,
};

pub mod metrics;
pub use metrics::Metrics;

pub mod payload;
pub mod pending_blocks;
pub use pending_blocks::{PendingBlocks, PendingBlocksBuilder, TransactionWithLogs};

pub mod traits;
pub use traits::{FlashblocksAPI, FlashblocksReceiver, PendingBlocksAPI};

mod block_assembler;
pub use block_assembler::{AssembledBlock, BlockAssembler};

pub mod cache;
pub use cache::FlashblockCache;

pub mod processor;
pub use processor::{StateProcessor, StateUpdate};

pub mod subscription;
pub use subscription::FlashblocksSubscriber;

pub mod validation;
pub use validation::{
    CanonicalBlockReconciler, FlashblockSequenceValidator, ReconciliationStrategy,
    ReorgDetectionResult, ReorgDetector, SequenceValidationResult,
};

pub mod rpc;
pub use rpc::{
    EthApiExt, EthApiOverrideServer, EthPubSub, EthPubSubApiServer, ExtendedSubscriptionKind,
    FlashblocksSubscriptionKind,
};
