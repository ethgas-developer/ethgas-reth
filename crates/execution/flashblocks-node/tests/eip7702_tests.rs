//! EIP-7702 delegation transaction tests for pending flashblocks state.
//!
//! Verify that EIP-7702 authorization/delegation transactions are accepted into pending flashblocks
//! state and surfaced over the flashblocks RPC overrides. Adapted from base for plain Ethereum:
//! there is no L1 deposit transaction, and our flashblock format carries receipts in
//! `metadata.receipts` (keyed by the EIP-2718 tx hash).

use alloy_consensus::{SignableTransaction, TxEip1559, TxEip7702, TxType};
use alloy_eips::{eip2718::Encodable2718, eip7702::Authorization};
use alloy_primitives::{
    Address, B256, Bytes, TxHash, TxKind, U256, keccak256, map::foldhash::HashMap,
};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use alloy_signer::SignerSync;
use alloy_sol_types::SolCall;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, Minimal7702Account, slot_number_at};
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use reth_ethereum_primitives::Receipt;

/// Cumulative gas used after the base flashblock (contract deployment).
const BASE_CUMULATIVE_GAS: u64 = 500_000;

struct TestSetup {
    harness: FlashblocksHarness,
    account_contract_address: Address,
    account_deploy_tx: Bytes,
    account_deploy_hash: TxHash,
}

impl TestSetup {
    async fn new() -> Result<Self> {
        let harness = FlashblocksHarness::new().await?;
        let deployer = Account::Deployer;
        let deploy_data = Minimal7702Account::BYTECODE.to_vec();
        let (account_deploy_tx, account_contract_address, account_deploy_hash) =
            deployer.create_deployment_tx(Bytes::from(deploy_data), 0)?;
        Ok(Self { harness, account_contract_address, account_deploy_tx, account_deploy_hash })
    }
}

const fn receipt(cumulative_gas_used: u64) -> Receipt {
    Receipt { tx_type: TxType::Eip1559, success: true, cumulative_gas_used, logs: vec![] }
}

fn build_authorization(
    chain_id: u64,
    contract_address: Address,
    nonce: u64,
    account: Account,
) -> alloy_eips::eip7702::SignedAuthorization {
    let auth = Authorization { chain_id: U256::from(chain_id), address: contract_address, nonce };
    let signature = account.signer().sign_hash_sync(&auth.signature_hash()).expect("signing works");
    auth.into_signed(signature)
}

fn build_eip7702_tx(
    chain_id: u64,
    nonce: u64,
    to: Address,
    value: U256,
    input: Bytes,
    authorization_list: Vec<alloy_eips::eip7702::SignedAuthorization>,
    account: Account,
) -> Bytes {
    let tx = TxEip7702 {
        chain_id,
        nonce,
        gas_limit: 200_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to,
        value,
        access_list: Default::default(),
        authorization_list,
        input,
    };
    let signature = account.signer().sign_hash_sync(&tx.signature_hash()).expect("signing works");
    tx.into_signed(signature).encoded_2718().into()
}

fn build_eip1559_tx(
    chain_id: u64,
    nonce: u64,
    to: Address,
    value: U256,
    input: Bytes,
    account: Account,
) -> Bytes {
    let tx = TxEip1559 {
        chain_id,
        nonce,
        gas_limit: 200_000,
        max_fee_per_gas: 1_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(to),
        value,
        access_list: Default::default(),
        input,
    };
    let signature = account.signer().sign_hash_sync(&tx.signature_hash()).expect("signing works");
    tx.into_signed(signature).encoded_2718().into()
}

fn create_base_flashblock(setup: &TestSetup) -> FlashBlock {
    let mut receipts = HashMap::default();
    receipts.insert(setup.account_deploy_hash, receipt(BASE_CUMULATIVE_GAS));
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index: 0,
        base: Some(ExecutionPayloadBaseV1 {
            parent_beacon_block_root: B256::default(),
            parent_hash: B256::default(),
            fee_recipient: Address::ZERO,
            prev_randao: B256::default(),
            block_number: 1,
            gas_limit: 30_000_000,
            timestamp: 0,
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::ZERO,
            slot_number: Some(slot_number_at(0)),
        }),
        diff: ExecutionPayloadFlashblockDeltaV1 {
            blob_gas_used: 0,
            transactions: vec![setup.account_deploy_tx.clone()],
            ..Default::default()
        },
        metadata: Metadata {
            block_number: 1,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

/// Asserts that `account` delegates to `target` at `pending`: its code is the EIP-7702 designator.
async fn assert_delegates(setup: &TestSetup, account: Account, target: Address) -> Result<()> {
    let code = setup.harness.provider().get_code_at(account.address()).pending().await?;
    let designator: Bytes = [&[0xef, 0x01, 0x00][..], target.as_slice()].concat().into();
    assert_eq!(code, designator, "the authorization must delegate");
    Ok(())
}

/// A non-base flashblock carrying `(encoded_tx, tx_hash, cumulative_gas)` entries.
fn delta_flashblock(index: u64, txs: Vec<(Bytes, TxHash, u64)>) -> FlashBlock {
    let mut receipts = HashMap::default();
    let mut transactions = Vec::new();
    for (tx, hash, cumulative) in txs {
        receipts.insert(hash, receipt(cumulative));
        transactions.push(tx);
    }
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index,
        base: None,
        diff: ExecutionPayloadFlashblockDeltaV1 {
            blob_gas_used: 0,
            transactions,
            ..Default::default()
        },
        metadata: Metadata {
            block_number: 1,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

#[tokio::test]
async fn test_eip7702_delegation_in_pending_flashblock() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();

    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    let auth = build_authorization(chain_id, setup.account_contract_address, 1, Account::Alice);
    let increment = Minimal7702Account::incrementCall {};
    let tx = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::from(increment.abi_encode()),
        vec![auth],
        Account::Alice,
    );
    let tx_hash = keccak256(&tx);

    setup
        .harness
        .send_flashblock(delta_flashblock(1, vec![(tx, tx_hash, BASE_CUMULATIVE_GAS + 50_000)]))
        .await?;

    let pending = setup.harness.provider().get_transaction_by_hash(tx_hash).await?;
    assert!(pending.is_some(), "EIP-7702 transaction should be in pending state");
    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;
    let counter = setup
        .harness
        .provider()
        .get_storage_at(Account::Alice.address(), U256::ZERO)
        .pending()
        .await?;
    assert_eq!(counter, U256::from(1), "increment() runs in the delegating account's storage");

    Ok(())
}

#[tokio::test]
async fn test_eip7702_multiple_delegations_same_flashblock() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();

    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    let increment = Minimal7702Account::incrementCall {};
    let auth_alice =
        build_authorization(chain_id, setup.account_contract_address, 1, Account::Alice);
    let tx_alice = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::from(increment.abi_encode()),
        vec![auth_alice],
        Account::Alice,
    );
    let auth_bob = build_authorization(chain_id, setup.account_contract_address, 1, Account::Bob);
    let tx_bob = build_eip7702_tx(
        chain_id,
        0,
        Account::Bob.address(),
        U256::ZERO,
        Bytes::from(increment.abi_encode()),
        vec![auth_bob],
        Account::Bob,
    );
    let hash_alice = keccak256(&tx_alice);
    let hash_bob = keccak256(&tx_bob);

    setup
        .harness
        .send_flashblock(delta_flashblock(
            1,
            vec![
                (tx_alice, hash_alice, BASE_CUMULATIVE_GAS + 50_000),
                (tx_bob, hash_bob, BASE_CUMULATIVE_GAS + 100_000),
            ],
        ))
        .await?;

    let provider = setup.harness.provider();
    assert!(
        provider.get_transaction_by_hash(hash_alice).await?.is_some(),
        "Alice's EIP-7702 tx should be pending"
    );
    assert!(
        provider.get_transaction_by_hash(hash_bob).await?.is_some(),
        "Bob's EIP-7702 tx should be pending"
    );
    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;
    assert_delegates(&setup, Account::Bob, setup.account_contract_address).await?;

    Ok(())
}

#[tokio::test]
async fn test_eip7702_pending_receipt() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();

    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    let auth = build_authorization(chain_id, setup.account_contract_address, 1, Account::Alice);
    let increment = Minimal7702Account::incrementCall {};
    let tx = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::from(increment.abi_encode()),
        vec![auth],
        Account::Alice,
    );
    let tx_hash = keccak256(&tx);

    setup
        .harness
        .send_flashblock(delta_flashblock(1, vec![(tx, tx_hash, BASE_CUMULATIVE_GAS + 50_000)]))
        .await?;

    let receipt = setup.harness.provider().get_transaction_receipt(tx_hash).await?;
    assert!(receipt.is_some(), "EIP-7702 receipt should be available in pending state");
    assert!(receipt.unwrap().status(), "EIP-7702 transaction should have succeeded");
    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;

    Ok(())
}

#[tokio::test]
async fn test_eip7702_delegation_then_execution() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();

    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    // Flashblock 1: delegation only (empty input just sets up the delegation).
    let auth = build_authorization(chain_id, setup.account_contract_address, 1, Account::Alice);
    let delegation_tx = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::new(),
        vec![auth],
        Account::Alice,
    );
    let delegation_hash = keccak256(&delegation_tx);
    setup
        .harness
        .send_flashblock(delta_flashblock(
            1,
            vec![(delegation_tx, delegation_hash, BASE_CUMULATIVE_GAS + 30_000)],
        ))
        .await?;

    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;

    // Flashblock 2: execute increment() through the now-delegated EOA (EIP-1559). The delegation
    // transaction and its authorization each raised Alice's nonce.
    let increment = Minimal7702Account::incrementCall {};
    let execution_tx = build_eip1559_tx(
        chain_id,
        2,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::from(increment.abi_encode()),
        Account::Alice,
    );
    let execution_hash = keccak256(&execution_tx);
    setup
        .harness
        .send_flashblock(delta_flashblock(
            2,
            vec![(execution_tx, execution_hash, BASE_CUMULATIVE_GAS + 55_000)],
        ))
        .await?;

    let provider = setup.harness.provider();
    assert!(
        provider.get_transaction_receipt(delegation_hash).await?.is_some(),
        "delegation tx receipt should exist"
    );
    assert!(
        provider.get_transaction_receipt(execution_hash).await?.is_some(),
        "execution tx receipt should exist"
    );
    let counter = provider.get_storage_at(Account::Alice.address(), U256::ZERO).pending().await?;
    assert_eq!(counter, U256::from(1), "increment() must run through the delegation");

    Ok(())
}

/// A self-sponsored authorization raises the sender's nonce twice: once for the transaction, once
/// for the authorization.
#[tokio::test]
async fn test_pending_nonce_counts_a_self_sponsored_authorization() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();
    let provider = setup.harness.provider();
    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    let auth = build_authorization(chain_id, setup.account_contract_address, 1, Account::Alice);
    let tx = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::new(),
        vec![auth],
        Account::Alice,
    );
    let tx_hash = keccak256(&tx);
    setup
        .harness
        .send_flashblock(delta_flashblock(1, vec![(tx, tx_hash, BASE_CUMULATIVE_GAS + 30_000)]))
        .await?;
    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;

    let alice = Account::Alice.address();
    assert_eq!(provider.get_transaction_count(alice).pending().await?, 2);
    assert_eq!(provider.get_account_info(alice).pending().await?.nonce, 2);

    Ok(())
}

/// An authorization raises its signer's nonce although the signer sent nothing.
#[tokio::test]
async fn test_pending_nonce_counts_a_sponsored_authorization() -> Result<()> {
    let setup = TestSetup::new().await?;
    let chain_id = setup.harness.chain_id();
    let provider = setup.harness.provider();
    setup.harness.send_flashblock(create_base_flashblock(&setup)).await?;

    let auth = build_authorization(chain_id, setup.account_contract_address, 0, Account::Alice);
    let tx = build_eip7702_tx(
        chain_id,
        0,
        Account::Alice.address(),
        U256::ZERO,
        Bytes::new(),
        vec![auth],
        Account::Bob,
    );
    let tx_hash = keccak256(&tx);
    setup
        .harness
        .send_flashblock(delta_flashblock(1, vec![(tx, tx_hash, BASE_CUMULATIVE_GAS + 30_000)]))
        .await?;
    assert_delegates(&setup, Account::Alice, setup.account_contract_address).await?;

    let alice = Account::Alice.address();
    assert_eq!(provider.get_transaction_count(alice).pending().await?, 1);
    assert_eq!(provider.get_account_info(alice).pending().await?.nonce, 1);
    assert_eq!(provider.get_transaction_count(Account::Bob.address()).pending().await?, 1);

    Ok(())
}
