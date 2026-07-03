//! Genesis configuration utilities for testing.

use std::collections::BTreeMap;

use alloy_genesis::{ChainConfig, Genesis, GenesisAccount};
use alloy_primitives::{Address, B256, Bytes, U256, utils::parse_ether};

use crate::Account;

/// Chain ID for devnet test network (Ethereum mainnet ID for local testing).
pub const DEVNET_CHAIN_ID: u64 = 1;

/// Gas limit for genesis block configuration.
pub const GENESIS_GAS_LIMIT: u64 = 100_000_000;

/// Builds a test genesis configuration programmatically.
///
/// Creates an Ethereum genesis with:
/// - All EVM hardforks enabled from genesis (including Prague)
/// - Pre-funded test accounts from the `Account` enum
pub fn build_test_genesis() -> Genesis {
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
