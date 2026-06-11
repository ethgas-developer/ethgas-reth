//! Test utilities for integration testing.
//!
//! This module provides testing infrastructure including:
//! - [`TestHarness`] and [`TestHarnessBuilder`] - Unified test harness for node and engine.
//! - [`LocalNode`] and [`LocalNodeProvider`] - Local node setup.
//! - [`EngineApi`] - Engine API client.
//! - [`Account`] - Test accounts with signing capabilities.
//! - Test constants, genesis, and contract bindings.

// Re-export from ethgas-test-utils
pub use ethgas_test_utils::{
    Account, DEVNET_CHAIN_ID, GENESIS_GAS_LIMIT, build_test_genesis,
    DoubleCounter, Minimal7702Account, MockERC20, TransparentUpgradeableProxy,
};

mod constants;
pub use constants::{
    BLOCK_BUILD_DELAY_MS, BLOCK_TIME_SECONDS, DEFAULT_JWT_SECRET,
    GAS_LIMIT, NODE_STARTUP_DELAY_MS,
};

mod engine;
pub use engine::{EngineApi, EngineProtocol, IpcEngine};

mod harness;
pub use harness::{TestHarness, TestHarnessBuilder};

mod node;
pub use node::{LocalNode, LocalNodeProvider};

mod tracing;
pub use tracing::init_silenced_tracing;

// Re-export signer traits for use in tests
pub use alloy_signer::SignerSync;
