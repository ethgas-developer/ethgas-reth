//! Genesis configuration utilities for testing.

use std::collections::BTreeMap;

use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use alloy_primitives::{Address, B256, Bytes, U256, utils::parse_ether};

use crate::Account;

/// Chain ID for devnet test network (Ethereum mainnet ID for local testing).
pub const DEVNET_CHAIN_ID: u64 = 1;

/// Gas limit for genesis block configuration.
pub const GENESIS_GAS_LIMIT: u64 = 100_000_000;

/// Added to a block's timestamp to give its slot number, so that no slot equals a block number or
/// a timestamp and a test can tell the three apart.
const SLOT_NUMBER_OFFSET: u64 = 1_000_000;

/// The slot number of the test chain's block at `timestamp`. Every fixture that names a slot uses
/// it, so a flashblock and a canonical block at the same timestamp agree.
pub const fn slot_number_at(timestamp: u64) -> u64 {
    timestamp + SLOT_NUMBER_OFFSET
}

/// Builds the test genesis: every fork through Amsterdam active from genesis, and the accounts of
/// the `Account` enum funded.
pub fn build_test_genesis() -> Genesis {
    build_test_genesis_with_amsterdam_at(Some(0))
}

/// [`build_test_genesis`] with Osaka at genesis and Amsterdam at `amsterdam_time`: `Some(0)` at
/// genesis, a later timestamp to fork while a test runs, `None` never.
pub fn build_test_genesis_with_amsterdam_at(amsterdam_time: Option<u64>) -> Genesis {
    let mut genesis = build_prague_test_genesis();
    genesis.config.osaka_time = Some(0);
    genesis.config.amsterdam_time = amsterdam_time;
    genesis
}

/// The test genesis with Prague as its newest fork, for the engine methods Osaka retired.
pub fn build_prague_test_genesis() -> Genesis {
    // Test account balance: 1 million ETH
    let test_account_balance: U256 = parse_ether("1000000").expect("valid ether amount");

    // Build chain config with all hardforks enabled at genesis
    let config = ChainConfig {
        chain_id: DEVNET_CHAIN_ID,
        // Block-based EVM hardforks (all at block 0)
        homestead_block: Some(0),
        eip150_block: Some(0),
        eip155_block: Some(0),
        eip158_block: Some(0),
        byzantium_block: Some(0),
        constantinople_block: Some(0),
        petersburg_block: Some(0),
        istanbul_block: Some(0),
        muir_glacier_block: Some(0),
        berlin_block: Some(0),
        london_block: Some(0),
        arrow_glacier_block: Some(0),
        gray_glacier_block: Some(0),
        merge_netsplit_block: Some(0),
        // Time-based hardforks
        shanghai_time: Some(0),
        cancun_time: Some(0),
        prague_time: Some(0),
        // Post-merge settings
        terminal_total_difficulty: Some(U256::ZERO),
        terminal_total_difficulty_passed: true,
        ..Default::default()
    };

    // Pre-fund all test accounts
    let alloc: BTreeMap<Address, GenesisAccount> = Account::all()
        .into_iter()
        .map(|account| {
            (account.address(), GenesisAccount::default().with_balance(test_account_balance))
        })
        .collect();

    Genesis {
        config,
        alloc,
        gas_limit: GENESIS_GAS_LIMIT,
        // Keep the base fee well below the test transactions' `max_fee_per_gas` (1 gwei) so the
        // EIP-1559-derived next-block base fee can never exceed the cap and reject inclusion.
        // Effective gas price stays capped at the tx `max_fee`, so gas costs are unchanged.
        base_fee_per_gas: Some(100),
        difficulty: U256::ZERO,
        nonce: 0,
        timestamp: 1,
        extra_data: Bytes::from_static(&[0x00]),
        mix_hash: B256::ZERO,
        coinbase: Address::ZERO,
        ..Default::default()
    }
}
