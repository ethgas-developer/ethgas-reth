//! Shared constants used across integration tests.

use alloy_primitives::B256;

/// Block time in seconds for test node configuration.
pub const BLOCK_TIME_SECONDS: u64 = 2;
/// Gas limit for test blocks.
pub const GAS_LIMIT: u64 = 200_000_000;
/// Delay in milliseconds to wait for node startup.
pub const NODE_STARTUP_DELAY_MS: u64 = 500;
/// Delay in milliseconds to wait for block building.
pub const BLOCK_BUILD_DELAY_MS: u64 = 100;

/// All-zeros secret for local testing only.
pub const DEFAULT_JWT_SECRET: B256 = B256::ZERO;
