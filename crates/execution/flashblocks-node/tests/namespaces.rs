//! The `pending` tag on methods outside the default `eth,net,web3` modules: `ots_hasCode` reads
//! the overlay, and the call-tracing methods read canonical state.

use DoubleCounter::DoubleCounterInstance;
use alloy_consensus::TxType;
use alloy_primitives::{Address, B256, Bytes, U256, keccak256, map::foldhash::HashMap};
use alloy_provider::Provider;
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::TransactionRequest;
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, DoubleCounter};
use ethgas_reth_flashblocks::payload::{
    ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
};
use eyre::Result;
use reth_ethereum_primitives::Receipt;
use serde_json::{Value, json};

const MODULES: &str = "eth,net,web3,ots,debug,trace";

struct Setup {
    harness: FlashblocksHarness,
    counter: Address,
    deployment_tx: Bytes,
}

impl Setup {
    async fn new() -> Result<Self> {
        let harness = FlashblocksHarness::with_rpc_modules(MODULES).await?;
        let (deployment_tx, counter, _) =
            Account::Deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;
        Ok(Self { harness, counter, deployment_tx })
    }

    /// The base flashblock of block 1, which deploys the counter.
    fn deployment_payload(&self) -> FlashBlock {
        let mut receipts = HashMap::default();
        receipts.insert(
            keccak256(&self.deployment_tx),
            Receipt {
                tx_type: TxType::Eip1559,
                success: true,
                cumulative_gas_used: 100_000,
                logs: vec![],
            },
        );
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 0,
            base: Some(ExecutionPayloadBaseV1 {
                parent_beacon_block_root: B256::ZERO,
                parent_hash: B256::ZERO,
                fee_recipient: Address::ZERO,
                prev_randao: B256::ZERO,
                block_number: 1,
                gas_limit: 30_000_000,
                timestamp: 0,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::ZERO,
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                transactions: vec![self.deployment_tx.clone()],
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

    fn count1(&self) -> TransactionRequest {
        DoubleCounterInstance::new(self.counter, self.harness.provider())
            .count1()
            .into_transaction_request()
    }
}

#[tokio::test]
async fn ots_has_code_reads_the_overlay_at_pending() -> Result<()> {
    let setup = Setup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.harness.send_flashblock(setup.deployment_payload()).await?;

    let pending: bool = client.request("ots_hasCode", (setup.counter, "pending")).await?;
    let latest: bool = client.request("ots_hasCode", (setup.counter, "latest")).await?;
    assert!(pending, "ots_hasCode must see a contract deployed in a flashblock");
    assert!(!latest);

    Ok(())
}

/// The call-tracing methods run on the overlay as `eth_call` does, and a copy of the deployment
/// in the pool changes nothing.
#[tokio::test]
async fn call_tracing_at_pending_reads_the_snapshot_not_the_pool() -> Result<()> {
    let setup = Setup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    setup.harness.send_flashblock(setup.deployment_payload()).await?;
    let _pool_copy = provider.send_raw_transaction(&setup.deployment_tx).await?;
    let one = U256::from(1);

    let count1: Bytes = client.request("eth_call", (setup.count1(), "pending")).await?;
    assert_eq!(U256::from_be_slice(&count1), one);

    let trace: Value =
        client.request("debug_traceCall", (setup.count1(), "pending", json!({}))).await?;
    let returned: Bytes = trace["returnValue"].as_str().expect("returnValue").parse()?;
    assert_eq!(U256::from_be_slice(&returned), one, "debug_traceCall must see the counter");
    let trace: Value =
        client.request("debug_traceCall", (setup.count1(), "latest", json!({}))).await?;
    assert!(
        trace["returnValue"].as_str().expect("returnValue").trim_start_matches("0x").is_empty(),
        "at latest the counter does not exist"
    );

    let trace: Value = client.request("trace_call", (setup.count1(), ["trace"], "pending")).await?;
    let output: Bytes = trace["output"].as_str().expect("output").parse()?;
    assert_eq!(U256::from_be_slice(&output), one, "trace_call must see the counter");

    let access_list: Value =
        client.request("eth_createAccessList", (setup.count1(), "pending")).await?;
    let entries = access_list["accessList"].as_array().expect("accessList");
    let counter = format!("{:#x}", setup.counter);
    assert!(
        entries.iter().any(|entry| {
            entry["address"].as_str().is_some_and(|address| address.eq_ignore_ascii_case(&counter)) &&
                entry["storageKeys"].as_array().is_some_and(|keys| !keys.is_empty())
        }),
        "eth_createAccessList must list the counter's storage: {entries:?}"
    );

    Ok(())
}
