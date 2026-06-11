//! Integration tests for `eth_call` with ERC-20 token operations.
//!
//! Tests cover:
//! - Basic ERC-20 functionality (transfer, mint, burn, approve, transferFrom)
//! - `TransparentUpgradeableProxy` with ERC-20 (USDC-style delegatecall patterns)
//!
//! These tests use `FlashblocksHarness` with manually constructed flashblock payloads
//! to properly test `eth_call` against contract state.
//!
//! Contract sources:
//! - `MockERC20`: Solmate's `MockERC20` (lib/solmate)
//! - `TransparentUpgradeableProxy`: `OpenZeppelin`'s proxy (lib/openzeppelin-contracts)

use alloy_consensus::TxType;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{Address, B256, Bytes, TxHash, U256, keccak256, map::foldhash::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use alloy_sol_types::{SolCall, SolConstructor, SolValue};
use ethgas_node_runner::test_utils::{Account, MockERC20, TransparentUpgradeableProxy};
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use reth_ethereum_primitives::Receipt;

/// Builds a synthetic receipt per transaction, keyed by the EIP-2718 tx hash
/// (`keccak256` of the encoded typed transaction). Our flashblock format carries receipts in
/// metadata and the processor requires one per transaction.
fn metadata_receipts(transactions: &[Bytes]) -> HashMap<TxHash, Receipt> {
    metadata_receipts_from(transactions, 0)
}

/// Like [`metadata_receipts`], but continues the cumulative gas from `start_cumulative_gas`.
///
/// Receipts must stay monotonic across the flashblocks of a single block: the processor derives
/// per-transaction gas as `receipt.cumulative_gas_used() - running_total`, which underflows (and
/// kills the processing task) if a later flashblock's receipts restart their cumulative from zero.
fn metadata_receipts_from(
    transactions: &[Bytes],
    start_cumulative_gas: u64,
) -> HashMap<TxHash, Receipt> {
    let mut receipts = HashMap::default();
    let mut cumulative_gas_used = start_cumulative_gas;
    for tx in transactions {
        cumulative_gas_used += 350_000;
        receipts.insert(
            keccak256(tx.as_ref()),
            Receipt { tx_type: TxType::Eip1559, success: true, cumulative_gas_used, logs: vec![] },
        );
    }
    receipts
}

struct Erc20TestSetup {
    harness: FlashblocksHarness,
    token_address: Address,
    token_deploy_tx: Bytes,
    proxy_address: Option<Address>,
    proxy_deploy_tx: Option<Bytes>,
}

impl Erc20TestSetup {
    async fn new(with_proxy: bool) -> Result<Self> {
        let harness = FlashblocksHarness::new().await?;
        let deployer = Account::Deployer;

        // Deploy MockERC20 from solmate with constructor args (name, symbol, decimals)
        let token_constructor = MockERC20::constructorCall {
            _name: "Test Token".to_string(),
            _symbol: "TEST".to_string(),
            _decimals: 18,
        };
        let token_deploy_data =
            [MockERC20::BYTECODE.to_vec(), token_constructor.abi_encode()].concat();
        let (token_deploy_tx, token_address, _) =
            deployer.create_deployment_tx(Bytes::from(token_deploy_data), 0)?;

        let (proxy_address, proxy_deploy_tx) = if with_proxy {
            // Deploy TransparentUpgradeableProxy from OpenZeppelin
            // Constructor: (implementation, initialOwner, data)
            let proxy_constructor = TransparentUpgradeableProxy::constructorCall {
                _logic: token_address,
                initialOwner: deployer.address(),
                // Our compiled OZ proxy reverts `ERC1967ProxyUninitialized()` when deployed with
                // empty data, so seed it with a harmless view call (`name()`) as the initializer.
                _data: Bytes::from(MockERC20::nameCall {}.abi_encode()),
            };
            let proxy_deploy_data =
                [TransparentUpgradeableProxy::BYTECODE.to_vec(), proxy_constructor.abi_encode()]
                    .concat();

            let (proxy_tx, proxy_addr, _) =
                deployer.create_deployment_tx(Bytes::from(proxy_deploy_data), 1)?;
            (Some(proxy_addr), Some(proxy_tx))
        } else {
            (None, None)
        };

        Ok(Self { harness, token_address, token_deploy_tx, proxy_address, proxy_deploy_tx })
    }

    /// Create the base flashblock payload (empty initial block)
    fn create_base_payload(&self) -> FlashBlock {
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
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                blob_gas_used: 0,
                transactions: vec![],
                ..Default::default()
            },
            metadata: Metadata {
                block_number: 1,
                receipts: HashMap::default(),
                new_account_balances: HashMap::default(),
            },
        }
    }

    /// Create flashblock payload with token deployment
    fn create_deploy_payload(&self) -> FlashBlock {
        let mut transactions = vec![self.token_deploy_tx.clone()];
        if let Some(proxy_tx) = &self.proxy_deploy_tx {
            transactions.push(proxy_tx.clone());
        }
        let receipts = metadata_receipts(&transactions);

        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 1,
            base: None,
            diff: ExecutionPayloadFlashblockDeltaV1 {
                state_root: B256::default(),
                receipts_root: B256::default(),
                gas_used: 700_000,
                block_hash: B256::default(),
                blob_gas_used: 0,
                transactions,
                withdrawals: Vec::new(),
                logs_bloom: Default::default(),
                excess_blob_gas: 0,
            },
            metadata: Metadata {
                block_number: 1,
                receipts,
                new_account_balances: HashMap::default(),
            },
        }
    }

    /// Create flashblock payload with mint transaction
    fn create_mint_payload(&self, mint_tx: Bytes) -> FlashBlock {
        // The mint flashblock follows the deployment flashblock, so its receipt's cumulative gas
        // must continue past the token (and optional proxy) deployment transactions.
        let prior_deploy_txs = 1 + u64::from(self.proxy_deploy_tx.is_some());
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 2,
            base: None,
            diff: ExecutionPayloadFlashblockDeltaV1 {
                state_root: B256::default(),
                receipts_root: B256::default(),
                gas_used: 750_000,
                block_hash: B256::default(),
                blob_gas_used: 0,
                transactions: vec![mint_tx.clone()],
                withdrawals: Vec::new(),
                logs_bloom: Default::default(),
                excess_blob_gas: 0,
            },
            metadata: Metadata {
                block_number: 1,
                receipts: metadata_receipts_from(
                    std::slice::from_ref(&mint_tx),
                    prior_deploy_txs * 350_000,
                ),
                new_account_balances: HashMap::default(),
            },
        }
    }

    async fn send_base_and_deploy(&self) -> Result<()> {
        self.harness.send_flashblock(self.create_base_payload()).await?;
        self.harness.send_flashblock(self.create_deploy_payload()).await?;
        Ok(())
    }
}

/// Test basic ERC-20 token deployment and name/symbol queries
#[tokio::test]
async fn test_erc20_deployment() -> Result<()> {
    let setup = Erc20TestSetup::new(false).await?;
    let provider = setup.harness.provider();

    // Send deployment payloads
    setup.send_base_and_deploy().await?;

    // Verify contract is deployed by querying name
    let token = MockERC20::MockERC20Instance::new(setup.token_address, provider.clone());
    let name_call = token.name().into_transaction_request();

    let result = provider.call(name_call).block(BlockNumberOrTag::Pending.into()).await?;
    let name = String::abi_decode(&result)?;
    assert_eq!(name, "Test Token");

    // Query symbol
    let symbol_call = token.symbol().into_transaction_request();
    let result = provider.call(symbol_call).block(BlockNumberOrTag::Pending.into()).await?;
    let symbol = String::abi_decode(&result)?;
    assert_eq!(symbol, "TEST");

    // Query decimals (returns uint8, but ABI encodes as uint256)
    let decimals_call = token.decimals().into_transaction_request();
    let result = provider.call(decimals_call).block(BlockNumberOrTag::Pending.into()).await?;
    let decimals = U256::abi_decode(&result)?;
    assert_eq!(decimals, U256::from(18));

    Ok(())
}

/// Test ERC-20 token deployment with `TransparentUpgradeableProxy`
#[tokio::test]
async fn test_proxy_erc20_deployment() -> Result<()> {
    let setup = Erc20TestSetup::new(true).await?;
    let provider = setup.harness.provider();

    // Send deployment payloads
    setup.send_base_and_deploy().await?;

    // Verify token implementation is deployed
    let token = MockERC20::MockERC20Instance::new(setup.token_address, provider.clone());
    let name_call = token.name().into_transaction_request();
    let result = provider.call(name_call).block(BlockNumberOrTag::Pending.into()).await?;
    let name = String::abi_decode(&result)?;
    assert_eq!(name, "Test Token");

    Ok(())
}

/// Test ERC-20 mint functionality
#[tokio::test]
async fn test_erc20_mint() -> Result<()> {
    let setup = Erc20TestSetup::new(false).await?;
    let provider = setup.harness.provider();

    // Deploy contracts first
    setup.send_base_and_deploy().await?;

    // Check initial balance is zero
    let token = MockERC20::MockERC20Instance::new(setup.token_address, provider.clone());
    let balance_call = token.balanceOf(Account::Alice.address()).into_transaction_request();
    let result =
        provider.call(balance_call.clone()).block(BlockNumberOrTag::Pending.into()).await?;
    let initial_balance = U256::abi_decode(&result)?;
    assert_eq!(initial_balance, U256::ZERO);

    // Create mint transaction
    let mint_amount = U256::from(1000u64);
    let mint_tx_request =
        token.mint(Account::Alice.address(), mint_amount).into_transaction_request();
    let (mint_tx, _) = Account::Deployer.sign_txn_request(mint_tx_request.nonce(1))?;

    // Send mint flashblock
    let mint_payload = setup.create_mint_payload(mint_tx);
    setup.harness.send_flashblock(mint_payload).await?;

    // Verify balance after mint
    let result = provider.call(balance_call).block(BlockNumberOrTag::Pending.into()).await?;
    let balance_after = U256::abi_decode(&result)?;
    assert_eq!(balance_after, mint_amount);

    Ok(())
}

/// Test ERC-20 mint through `TransparentUpgradeableProxy`.
///
/// Exercises a deploy → mint → read sequence entirely through the proxy in pending flashblocks
/// state. Two fixture details matter: the proxy must be seeded with non-empty initializer `_data`
/// (our compiled OZ proxy reverts `ERC1967ProxyUninitialized()` otherwise — see `new`), and the
/// mint receipt's cumulative gas must stay monotonic across the deploy flashblock (see
/// `create_mint_payload` / `metadata_receipts_from`).
#[tokio::test]
async fn test_proxy_erc20_mint() -> Result<()> {
    let setup = Erc20TestSetup::new(true).await?;
    let provider = setup.harness.provider();

    // Deploy contracts first
    setup.send_base_and_deploy().await?;

    // Check initial balance is zero through proxy
    let proxy_address = setup.proxy_address.unwrap();
    let token_via_proxy = MockERC20::MockERC20Instance::new(proxy_address, provider.clone());
    let balance_call =
        token_via_proxy.balanceOf(Account::Alice.address()).into_transaction_request();
    let result =
        provider.call(balance_call.clone()).block(BlockNumberOrTag::Pending.into()).await?;
    let initial_balance = U256::abi_decode(&result)?;
    assert_eq!(initial_balance, U256::ZERO);

    // Create mint transaction through proxy
    let mint_amount = U256::from(5000u64);
    let mint_tx_request =
        token_via_proxy.mint(Account::Alice.address(), mint_amount).into_transaction_request();
    let (mint_tx, _) = Account::Deployer.sign_txn_request(mint_tx_request.nonce(2))?;

    // Send mint flashblock (note: interaction_address returns proxy)
    let mint_payload = setup.create_mint_payload(mint_tx);
    setup.harness.send_flashblock(mint_payload).await?;

    // Verify balance after mint through proxy
    let result = provider.call(balance_call).block(BlockNumberOrTag::Pending.into()).await?;
    let balance_after = U256::abi_decode(&result)?;
    assert_eq!(balance_after, mint_amount);

    Ok(())
}
