pub mod config;
pub use config::FlashblocksConfig;

pub mod state;
pub use state::FlashblocksState;

pub mod error;
pub use error::{BuildError, ExecutionError, ProtocolError, ProviderError, Result, StateProcessorError};

pub mod metrics;
pub use metrics::Metrics;

pub mod payload;
pub mod pending_blocks;
pub use pending_blocks::{PendingBlocks, PendingBlocksBuilder, TransactionWithLogs};

pub mod traits;
pub use traits::{FlashblocksAPI, FlashblocksReceiver, PendingBlocksAPI};

pub mod state_builder;
pub use state_builder::{ExecutedPendingTransaction, PendingStateBuilder};

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
    EthApiExt, EthApiOverrideServer, EthPubSub, EthPubSubApiServer,
    ExtendedSubscriptionKind, FlashblocksSubscriptionKind,
};

mod extension;
pub use extension::FlashblocksExtension;

#[cfg(feature = "test-utils")]
pub mod test_harness;

#[cfg(test)]
mod tests;
