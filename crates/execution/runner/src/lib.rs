//! Node assembly for the Ethgas node: the builder hooks, the extension trait that
//! features plug into, and the runner that launches the configured node.

pub mod eth_api;

pub mod pending_state;
pub use pending_state::{PendingOverlay, PendingStateSource};

mod builder;
pub use builder::{EthgasRpcContext, NodeHooks};

mod extension;
pub use extension::{EthgasNodeExtension, FromExtensionConfig};

mod runner;
pub use runner::EthgasNodeRunner;

pub mod types;

#[cfg(any(test, feature = "test-utils"))]
pub mod test_utils;
