//! Account destruction at `pending`.
//!
//! After Cancun (EIP-6780), `SELFDESTRUCT` deletes an account only in the transaction that
//! created it; every other `SELFDESTRUCT` moves the balance and keeps the code. The deleting case
//! is observable at `pending` when the address held a balance before the creation: the balance
//! is gone, and the account with it.

use alloy_primitives::{Address, B256, Bytes, U256, address, keccak256, map::foldhash::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::TransactionRequest;
use alloy_sol_types::SolConstructor;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, SelfDestructor};
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use reth_ethereum_primitives::Receipt;

const BENEFICIARY: Address = address!("0x00000000000000000000000000000000000000b1");
const PREFUND: u64 = 5;

/// The base flashblock of block `number` on `parent_hash`, carrying `transactions`.
fn base_payload(number: u64, parent_hash: B256, transactions: Vec<Bytes>) -> FlashBlock {
    let mut receipts = HashMap::default();
    let mut cumulative_gas_used = 0;
    for tx in &transactions {
        cumulative_gas_used += 100_000;
        receipts.insert(
            keccak256(tx),
            Receipt {
                tx_type: alloy_consensus::TxType::Eip1559,
                success: true,
                cumulative_gas_used,
                logs: vec![],
            },
        );
    }
    FlashBlock {
        payload_id: PayloadId::new([0; 8]),
        index: 0,
        base: Some(ExecutionPayloadBaseV1 {
            parent_beacon_block_root: B256::ZERO,
            parent_hash,
            fee_recipient: Address::ZERO,
            prev_randao: B256::ZERO,
            block_number: number,
            gas_limit: 30_000_000,
            timestamp: 0,
            extra_data: Bytes::new(),
            base_fee_per_gas: U256::ZERO,
        }),
        diff: ExecutionPayloadFlashblockDeltaV1 { transactions, ..Default::default() },
        metadata: Metadata {
            block_number: number,
            receipts,
            new_account_balances: HashMap::default(),
            inclusion_fee: None,
        },
    }
}

#[tokio::test]
async fn a_contract_destroyed_in_its_creating_transaction_is_gone_at_pending() -> Result<()> {
    let harness = FlashblocksHarness::new().await?;
    let provider = harness.provider();
    let deployer = Account::Deployer;
    let init_code = [
        SelfDestructor::BYTECODE.to_vec(),
        SelfDestructor::constructorCall { beneficiary: BENEFICIARY, onCreate: true }.abi_encode(),
    ]
    .concat();
    let (deployment_tx, contract, _) = deployer.create_deployment_tx(init_code.into(), 0)?;

    // The address holds a balance before the contract exists.
    let (prefund_tx, _) = Account::Alice.sign_txn_request(
        TransactionRequest::default().to(contract).value(U256::from(PREFUND)).nonce(0),
    )?;
    harness.build_block_from_transactions(vec![prefund_tx]).await?;
    assert_eq!(provider.get_balance(contract).await?, U256::from(PREFUND));

    let parent = harness.latest_block().hash();
    harness.send_flashblock(base_payload(2, parent, vec![deployment_tx])).await?;

    assert_eq!(
        provider.get_balance(contract).pending().await?,
        U256::ZERO,
        "the account is deleted with its balance"
    );
    assert_eq!(provider.get_balance(BENEFICIARY).pending().await?, U256::from(PREFUND));
    let info = provider.get_account_info(contract).pending().await?;
    assert_eq!((info.nonce, info.balance, info.code), (0, U256::ZERO, Bytes::new()));
    assert_eq!(provider.get_transaction_count(contract).pending().await?, 0);
    assert_eq!(provider.get_code_at(contract).pending().await?, Bytes::new());

    // Nothing of it is canonical.
    assert_eq!(provider.get_balance(contract).await?, U256::from(PREFUND));
    assert_eq!(provider.get_balance(BENEFICIARY).await?, U256::ZERO);

    Ok(())
}
