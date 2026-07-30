//! Shared test utilities: funded accounts, a devnet genesis builder, and
//! `sol!` bindings for the contracts used across the test suites.

#![cfg_attr(not(test), warn(unused_crate_dependencies))]

mod accounts;
pub use accounts::Account;

mod genesis;
pub use genesis::{DEVNET_CHAIN_ID, GENESIS_GAS_LIMIT, build_test_genesis};

mod contracts;
pub use contracts::{DoubleCounter, Minimal7702Account, MockERC20, TransparentUpgradeableProxy};
