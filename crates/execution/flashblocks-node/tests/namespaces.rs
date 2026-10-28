//! The `pending` tag on methods outside the default `eth,net,web3` modules: `ots_hasCode` and the
//! call-tracing methods read the overlay, and the tracers run in the flashblock's block
//! environment; the block-replaying methods and `eth_callBundle` read canonical state.

use DoubleCounter::DoubleCounterInstance;
use alloy_consensus::TxType;
use alloy_eips::BlockNumberOrTag;
use alloy_primitives::{
    Address, B256, Bytes, U256, address, bytes, keccak256, map::foldhash::HashMap,
};
use alloy_provider::Provider;
use alloy_rpc_types::simulate::{SimBlock, SimulatePayload};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::{TransactionInput, TransactionRequest};
use ethgas_flashblocks_node::test_harness::FlashblocksHarness;
use ethgas_node_runner::test_utils::{Account, DoubleCounter, slot_number_at};
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
        self.deployment_payload_for(1)
    }

    /// The base flashblock of block `number`, which deploys the counter.
    fn deployment_payload_for(&self, number: u64) -> FlashBlock {
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
                block_number: number,
                gas_limit: 30_000_000,
                timestamp: 0,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::ZERO,
                slot_number: Some(slot_number_at(0)),
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                transactions: vec![self.deployment_tx.clone()],
                ..Default::default()
            },
            metadata: Metadata {
                block_number: number,
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

    fn count2(&self) -> TransactionRequest {
        DoubleCounterInstance::new(self.counter, self.harness.provider())
            .count2()
            .into_transaction_request()
    }
}

/// `NUMBER PUSH0 MSTORE TIMESTAMP PUSH1 32 MSTORE BASEFEE PUSH1 64 MSTORE COINBASE PUSH1 96 MSTORE
/// PUSH1 128 PUSH0 RETURN`: init code that returns the four block-environment words as its code.
const RETURN_BLOCK_ENV: Bytes = bytes!("0x435f5242602052486040524160605260805ff3");
const ENV_TIMESTAMP: u64 = 1_700_000_000;
const ENV_BASE_FEE: u64 = 7;
const ENV_FEE_RECIPIENT: Address = address!("0x00000000000000000000000000000000000c0ffe");

fn block_env_words(output: &Value) -> Result<Vec<U256>> {
    let output: Bytes = output.as_str().expect("output").parse()?;
    Ok(output.chunks(32).map(U256::from_be_slice).collect())
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

/// `trace_callMany` and `trace_rawTransaction` take their state id from `evm_env_at`, so at
/// `pending` they run on the overlay as `trace_call` does.
#[tokio::test]
async fn trace_call_many_and_raw_transaction_at_pending_read_the_snapshot() -> Result<()> {
    let setup = Setup::new().await?;
    let client = setup.harness.rpc_client()?;
    setup.harness.send_flashblock(setup.deployment_payload()).await?;
    let one = U256::from(1);

    let traces: Value = client
        .request("trace_callMany", (vec![(setup.count1(), vec!["trace"])], "pending"))
        .await?;
    let output: Bytes = traces[0]["output"].as_str().expect("output").parse()?;
    assert_eq!(U256::from_be_slice(&output), one, "trace_callMany must see the counter");

    let (raw, _) = Account::Alice.sign_txn_request(setup.count1().nonce(0))?;
    let trace: Value =
        client.request("trace_rawTransaction", (raw, vec!["trace"], "pending")).await?;
    let output: Bytes = trace["output"].as_str().expect("output").parse()?;
    assert_eq!(U256::from_be_slice(&output), one, "trace_rawTransaction must see the counter");

    Ok(())
}

/// The call-tracing methods at `pending` run in the flashblock's block environment: its number,
/// timestamp, base fee and fee recipient, not the latest block's.
#[tokio::test]
async fn call_tracing_at_pending_runs_in_the_flashblocks_block_environment() -> Result<()> {
    let setup = Setup::new().await?;
    let client = setup.harness.rpc_client()?;
    let mut payload = setup.deployment_payload();
    let base = payload.base.as_mut().expect("a base flashblock carries a base");
    base.timestamp = ENV_TIMESTAMP;
    base.base_fee_per_gas = U256::from(ENV_BASE_FEE);
    base.fee_recipient = ENV_FEE_RECIPIENT;
    setup.harness.send_flashblock(payload).await?;
    let expected = vec![
        U256::from(1),
        U256::from(ENV_TIMESTAMP),
        U256::from(ENV_BASE_FEE),
        U256::from_be_slice(ENV_FEE_RECIPIENT.as_slice()),
    ];
    // A fee cap, so that the calls keep the block's base fee.
    let create = TransactionRequest::default()
        .from(Account::Alice.address())
        .max_fee_per_gas(1_000_000_000)
        .max_priority_fee_per_gas(0)
        .input(TransactionInput::new(RETURN_BLOCK_ENV));

    let debug_trace_call: Value = client
        .request("debug_traceCall", (create.clone(), "pending", json!({ "tracer": "callTracer" })))
        .await?;
    let trace_call: Value =
        client.request("trace_call", (create.clone(), ["trace"], "pending")).await?;
    let trace_call_many: Value =
        client.request("trace_callMany", (vec![(create, vec!["trace"])], "pending")).await?;
    let (raw, _, _) = Account::Charlie.create_deployment_tx(RETURN_BLOCK_ENV, 0)?;
    let trace_raw_transaction: Value =
        client.request("trace_rawTransaction", (raw, vec!["trace"], "pending")).await?;

    let environments = [
        ("debug_traceCall", block_env_words(&debug_trace_call["output"])?),
        ("trace_call", block_env_words(&trace_call["output"])?),
        ("trace_callMany", block_env_words(&trace_call_many[0]["output"])?),
        ("trace_rawTransaction", block_env_words(&trace_raw_transaction["output"])?),
    ];
    let wrong: Vec<_> = environments.iter().filter(|(_, words)| *words != expected).collect();
    assert!(wrong.is_empty(), "not the flashblock's environment {expected:?}: {wrong:?}");

    Ok(())
}

/// A state override at `pending` applies on top of the flashblock's state, through
/// `debug_traceCall` and `eth_simulateV1` as through `eth_call`: the overridden slot reads the
/// override, while the counter's other slot and its code stay as the flashblock left them.
#[tokio::test]
async fn state_overrides_at_pending_apply_on_top_of_the_flashblocks_state() -> Result<()> {
    let setup = Setup::new().await?;
    let provider = setup.harness.provider();
    let client = setup.harness.rpc_client()?;
    setup.harness.send_flashblock(setup.deployment_payload()).await?;
    let count1_is_five = json!({
        format!("{:#x}", setup.counter): {
            "stateDiff": { format!("{:#x}", B256::ZERO): format!("{:#x}", B256::with_last_byte(5)) }
        }
    });
    let expected = vec![U256::from(5), U256::from(1)];

    let mut traced = Vec::new();
    for call in [setup.count1(), setup.count2()] {
        let options = json!({ "tracer": "callTracer", "stateOverrides": count1_is_five });
        let trace: Value = client.request("debug_traceCall", (call, "pending", options)).await?;
        // The call tracer leaves `output` out when the call returns nothing.
        let output: Bytes = trace["output"].as_str().unwrap_or("0x").parse()?;
        traced.push(U256::from_be_slice(&output));
    }

    let simulation = SimulatePayload {
        block_state_calls: vec![SimBlock {
            calls: vec![setup.count1().gas_limit(100_000), setup.count2().gas_limit(100_000)],
            block_overrides: None,
            state_overrides: Some(serde_json::from_value(count1_is_five)?),
        }],
        trace_transfers: false,
        validation: false,
        return_full_transactions: false,
    };
    let blocks = provider.simulate(&simulation).block_id(BlockNumberOrTag::Pending.into()).await?;
    let simulated: Vec<U256> =
        blocks[0].calls.iter().map(|call| U256::from_be_slice(&call.return_data)).collect();
    assert_eq!((&traced, &simulated), (&expected, &expected), "(debug_traceCall, eth_simulateV1)");

    Ok(())
}

/// `eth_callMany` and `debug_traceCallMany` replay the block that `pending` resolves to, the
/// canonical tip, on its parent's state, and `eth_callBundle` takes reth's own pending
/// environment over the latest block's state. None of the three reaches the overlay.
#[tokio::test]
async fn block_replaying_methods_and_call_bundle_at_pending_read_canonical_state() -> Result<()> {
    let setup = Setup::new().await?;
    let client = setup.harness.rpc_client()?;
    // A tip with a parent, since the replay starts from the parent's state.
    setup.harness.advance_chain(1).await?;
    setup.harness.send_flashblock(setup.deployment_payload_for(2)).await?;
    let count1: Bytes = client.request("eth_call", (setup.count1(), "pending")).await?;
    assert_eq!(U256::from_be_slice(&count1), U256::from(1), "the snapshot is served");

    let bundles = vec![json!({ "transactions": [setup.count1()] })];
    let context = json!({ "blockNumber": "pending" });
    let results: Value = client.request("eth_callMany", (bundles.clone(), context.clone())).await?;
    assert_eq!(results[0][0]["value"], json!("0x"), "eth_callMany reads canonical state");

    let traces: Value =
        client.request("debug_traceCallMany", (bundles, context, json!({}))).await?;
    let returned = traces[0][0]["returnValue"].as_str().expect("returnValue");
    assert!(returned.trim_start_matches("0x").is_empty(), "debug_traceCallMany: {traces}");

    let (raw, _) = Account::Alice.sign_txn_request(setup.count1().nonce(0))?;
    let bundle = json!({ "txs": [raw], "blockNumber": "0x2", "stateBlockNumber": "pending" });
    let response: Value = client.request("eth_callBundle", (bundle,)).await?;
    assert_eq!(response["results"][0]["value"], json!("0x"), "eth_callBundle: {response}");

    Ok(())
}
