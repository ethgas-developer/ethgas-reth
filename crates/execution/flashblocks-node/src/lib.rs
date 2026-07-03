//! Flashblocks node extension.
//!
//! Wires the flashblocks feature (canonical block subscription and RPC surface) into the Ethgas
//! node builder. Keeps the node-builder coupling out of the [`ethgas_reth_flashblocks`] library
//! crate, which stays node-agnostic.

mod extension;
pub use extension::FlashblocksExtension;

#[cfg(feature = "test-utils")]
pub mod test_harness;
