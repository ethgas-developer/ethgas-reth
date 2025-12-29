use reth::chainspec::NamedChain;

mod accounts;
mod contracts;
mod engine;
mod fixtures;
mod flashblocks_harness;
mod harness;
mod node;

pub use flashblocks_harness::FlashblocksHarness;
pub use harness::TestHarness;
pub use node::LocalNodeProvider;
pub use accounts::TestAccounts;

// Re-export flashblocks types for tests
pub use ethgas_reth_flashblocks::{
    state::FlashblocksState,
    traits::{FlashblocksAPI, PendingBlocksAPI},
    payload::{ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, Flashblock, Metadata},
};


pub const CHAIN_ID: u64 = NamedChain::Hoodi as u64;
pub const DEFAULT_JWT_SECRET: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000000";

use std::sync::Once;

use tracing_subscriber::{EnvFilter, filter::LevelFilter};

static INIT: Once = Once::new();

/// Initializes tracing for integration tests while silencing the noisy executor warnings
/// (`reth_tasks` and `reth_node_builder::launch::common`) that appear whenever multiple nodes
/// reuse the global rayon/Tokio pools in a single process.
///
/// Tests call this helper before booting a harness; repeated calls are cheap and only the first one
/// installs the subscriber.
pub fn init_silenced_tracing() {
    INIT.call_once(|| {
        let mut filter =
            EnvFilter::builder().with_default_directive(LevelFilter::INFO.into()).from_env_lossy();

        for directive in ["reth_tasks=off", "reth_node_builder::launch::common=off"].into_iter() {
            if let Ok(directive) = directive.parse() {
                filter = filter.add_directive(directive);
            }
        }

        let _ = tracing_subscriber::fmt().with_env_filter(filter).with_test_writer().try_init();
    });
}
