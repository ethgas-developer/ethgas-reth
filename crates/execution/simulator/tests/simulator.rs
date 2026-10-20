//! The block simulator over RPC. The validation methods execute a payload the node built but did
//! not import, and `relay_inclusionList` publishes the pool's pending transactions after each
//! canonical block.

use std::{sync::Arc, time::Duration};

use alloy_consensus::{SignableTransaction, TxEip1559, TxEnvelope};
use alloy_eips::{
    eip2718::Encodable2718,
    eip7685::{Requests, RequestsOrHash},
};
use alloy_primitives::{Address, B256, Bytes, TxKind, U256, address, keccak256};
use alloy_provider::Provider;
use alloy_rpc_types_beacon::{
    BlsPublicKey, BlsSignature,
    relay::{
        BidTrace, BuilderBlockValidationRequestV5, BuilderBlockValidationRequestV6,
        SignedBidSubmissionV5, SignedBidSubmissionV6,
    },
    requests::{ExecutionRequestsV4, ExecutionRequestsV5},
};
use alloy_rpc_types_engine::{
    CancunPayloadFields, ExecutionPayload, ExecutionPayloadEnvelopeV5, ExecutionPayloadEnvelopeV6,
    ExecutionPayloadSidecar, ExecutionPayloadV3, PayloadAttributes, PraguePayloadFields,
};
use ethgas_node_runner::test_utils::{
    Account, BLOCK_BUILD_DELAY_MS, BLOCK_TIME_SECONDS, DEVNET_CHAIN_ID, EnginePayload, SignerSync,
    TestHarness, build_test_genesis, build_test_genesis_with_amsterdam_at, slot_number_at,
};
use ethgas_simulator::{
    ExtendedValidationRequestV5, ExtendedValidationRequestV6, InclusionListTxWithMetadata,
    InclusionListWithMetadata, SimulatorConfig, SimulatorExtension,
};
use eyre::{Result, bail, eyre};
use reth_chainspec::{ChainSpec, EthereumHardforks};
use serde::Serialize;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    time::sleep,
};
use url::Url;

/// The fee recipient of every payload the tests build, and the proposer the bids pay.
const FEE_RECIPIENT: Address = address!("0x000000000000000000000000000000000000fee5");

/// A blacklist provider nothing listens on: the simulator runs with no disallow list.
const NO_BLACKLIST: &str = "http://127.0.0.1:1/blacklist";

const V5: &str = "relay_validateBuilderSubmissionV5";
const V6: &str = "relay_validateBuilderSubmissionV6";

async fn simulator_node(chain_spec: ChainSpec, blacklist_provider: &str) -> Result<TestHarness> {
    let config = SimulatorConfig::new(Url::parse(blacklist_provider)?);
    TestHarness::builder()
        .with_chain_spec(Arc::new(chain_spec))
        .with_ext::<SimulatorExtension>(Some(config))
        .build()
        .await
}

/// A transfer from Alice that pays the fee recipient a priority fee.
fn tipping_transfer(nonce: u64) -> Result<Bytes> {
    let tx = TxEip1559 {
        chain_id: DEVNET_CHAIN_ID,
        nonce,
        gas_limit: 21_000,
        max_fee_per_gas: 10_000_000_000,
        max_priority_fee_per_gas: 1_000_000_000,
        to: TxKind::Call(Account::Bob.address()),
        value: U256::from(1),
        ..Default::default()
    };
    let signature = Account::Alice.signer().sign_hash_sync(&tx.signature_hash())?;
    Ok(TxEnvelope::from(tx.into_signed(signature)).encoded_2718().into())
}

/// Has the node build a payload on its head from its pool, without importing it. Every payload
/// uses a zero parent beacon block root.
async fn build_payload(harness: &TestHarness) -> Result<EnginePayload> {
    let head = harness.latest_block();
    let timestamp = head.header().timestamp + BLOCK_TIME_SECONDS;
    let attributes = PayloadAttributes {
        timestamp,
        suggested_fee_recipient: FEE_RECIPIENT,
        withdrawals: Some(vec![]),
        parent_beacon_block_root: Some(B256::ZERO),
        slot_number: harness
            .chain_spec()
            .is_amsterdam_active_at_timestamp(timestamp)
            .then(|| slot_number_at(timestamp)),
        ..Default::default()
    };
    let updated =
        harness.engine().update_forkchoice(head.hash(), head.hash(), Some(attributes)).await?;
    let payload_id =
        updated.payload_id.ok_or_else(|| eyre!("the forkchoice update started no payload"))?;
    sleep(Duration::from_millis(BLOCK_BUILD_DELAY_MS)).await;
    harness.engine().get_payload(payload_id, timestamp).await
}

/// Builds a payload that holds one tipping transfer.
async fn build_tipping_payload(harness: &TestHarness) -> Result<EnginePayload> {
    let _ = harness.provider().send_raw_transaction(&tipping_transfer(0)?).await?;
    let payload = build_payload(harness).await?;
    assert_eq!(payload.payload_v3().payload_inner.payload_inner.transactions.len(), 1);
    Ok(payload)
}

/// A bid for `payload` that promises the fee recipient `value`.
const fn bid(payload: &ExecutionPayloadV3, value: U256) -> BidTrace {
    let block = &payload.payload_inner.payload_inner;
    BidTrace {
        slot: block.block_number,
        parent_hash: block.parent_hash,
        block_hash: block.block_hash,
        builder_pubkey: BlsPublicKey::ZERO,
        proposer_pubkey: BlsPublicKey::ZERO,
        proposer_fee_recipient: FEE_RECIPIENT,
        gas_limit: block.gas_limit,
        gas_used: block.gas_used,
        value,
    }
}

fn v5_request(envelope: &ExecutionPayloadEnvelopeV5) -> Result<ExtendedValidationRequestV5> {
    Ok(ExtendedValidationRequestV5 {
        base: BuilderBlockValidationRequestV5 {
            request: SignedBidSubmissionV5 {
                message: bid(&envelope.execution_payload, envelope.block_value),
                execution_payload: envelope.execution_payload.clone(),
                blobs_bundle: envelope.blobs_bundle.clone(),
                execution_requests: ExecutionRequestsV4::try_from(&envelope.execution_requests)?,
                signature: BlsSignature::ZERO,
            },
            registered_gas_limit: envelope.execution_payload.payload_inner.payload_inner.gas_limit,
            parent_beacon_block_root: B256::ZERO,
        },
        inclusion_list: None,
        apply_blacklist: false,
    })
}

fn v6_request(envelope: &ExecutionPayloadEnvelopeV6) -> Result<ExtendedValidationRequestV6> {
    let payload = &envelope.execution_payload;
    Ok(ExtendedValidationRequestV6 {
        base: BuilderBlockValidationRequestV6 {
            request: SignedBidSubmissionV6 {
                message: bid(&payload.payload_inner, envelope.block_value),
                execution_payload: payload.clone(),
                blobs_bundle: envelope.blobs_bundle.clone(),
                execution_requests: ExecutionRequestsV5::try_from(&envelope.execution_requests)?,
                signature: BlsSignature::ZERO,
            },
            registered_gas_limit: payload.payload_inner.payload_inner.payload_inner.gas_limit,
            parent_beacon_block_root: B256::ZERO,
        },
        inclusion_list: None,
        apply_blacklist: false,
    })
}

/// The hash of the block `payload` describes, for a payload a test has altered.
fn block_hash(payload: ExecutionPayload, requests: Requests) -> Result<B256> {
    let sidecar = ExecutionPayloadSidecar::v4(
        CancunPayloadFields { parent_beacon_block_root: B256::ZERO, versioned_hashes: vec![] },
        PraguePayloadFields { requests: RequestsOrHash::Requests(requests) },
    );
    Ok(payload.try_into_block_with_sidecar::<TxEnvelope>(&sidecar)?.header.hash_slow())
}

/// Sends `request` to `method`. Returns the error message, or `None` when the simulator accepts
/// the block.
async fn simulate(
    harness: &TestHarness,
    method: &'static str,
    request: &impl Serialize,
) -> Result<Option<String>> {
    let request = serde_json::to_value(request)?;
    match harness.rpc_client()?.request::<_, ()>(method, (request,)).await {
        Ok(()) => Ok(None),
        Err(error) => match error.as_error_resp() {
            Some(payload) => Ok(Some(payload.message.to_string())),
            None => Err(error.into()),
        },
    }
}

fn inclusion_list(transaction: Bytes) -> InclusionListWithMetadata {
    InclusionListWithMetadata {
        txs: vec![InclusionListTxWithMetadata {
            hash: keccak256(&transaction),
            bytes: transaction,
        }],
    }
}

/// Serves `body` as the JSON answer to every request, on a free local port.
async fn serve_disallow_list(body: String) -> Result<Url> {
    let listener = TcpListener::bind("127.0.0.1:0").await?;
    let url = Url::parse(&format!("http://{}/blacklist", listener.local_addr()?))?;
    tokio::spawn(async move {
        while let Ok((mut stream, _)) = listener.accept().await {
            let _ = stream.read(&mut [0; 1024]).await;
            let response = format!(
                "HTTP/1.1 200 OK\r\ncontent-type: application/json\r\ncontent-length: {}\r\n\
                 connection: close\r\n\r\n{body}",
                body.len()
            );
            let _ = stream.write_all(response.as_bytes()).await;
        }
    });
    Ok(url)
}

#[tokio::test]
async fn validates_an_osaka_block_against_its_bid() -> Result<()> {
    let harness =
        simulator_node(build_test_genesis_with_amsterdam_at(None).into(), NO_BLACKLIST).await?;
    let EnginePayload::V5(envelope) = build_tipping_payload(&harness).await? else {
        bail!("an Osaka chain builds V5 payloads");
    };
    let mut request = v5_request(&envelope)?;
    assert_eq!(simulate(&harness, V5, &request).await?, None);

    request.base.request.message.value += U256::from(1);
    assert_eq!(
        simulate(&harness, V5, &request).await?.as_deref(),
        Some("could not verify proposer payment")
    );

    // A wrong state root, sealed into the block hash, shows only when the block executes.
    let mut request = v5_request(&envelope)?;
    let submission = &mut request.base.request;
    submission.execution_payload.payload_inner.payload_inner.state_root = B256::repeat_byte(0x11);
    let hash = block_hash(
        ExecutionPayload::V3(submission.execution_payload.clone()),
        envelope.execution_requests.clone(),
    )?;
    submission.execution_payload.payload_inner.payload_inner.block_hash = hash;
    submission.message.block_hash = hash;
    let error = simulate(&harness, V5, &request).await?.expect("the state root is wrong");
    assert!(error.starts_with("mismatched block state root"), "{error}");

    Ok(())
}

#[tokio::test]
async fn validates_an_amsterdam_block_and_its_access_list() -> Result<()> {
    let harness = simulator_node(build_test_genesis().into(), NO_BLACKLIST).await?;
    let EnginePayload::V6(envelope) = build_tipping_payload(&harness).await? else {
        bail!("an Amsterdam chain builds V6 payloads");
    };
    let mut request = v6_request(&envelope)?;
    assert_eq!(simulate(&harness, V6, &request).await?, None);

    // An empty access list, sealed into the block hash, is not the one execution produces.
    let submission = &mut request.base.request;
    submission.execution_payload.block_access_list = Bytes::from_static(&[0xc0]);
    let hash = block_hash(
        ExecutionPayload::V4(submission.execution_payload.clone()),
        envelope.execution_requests.clone(),
    )?;
    submission.execution_payload.payload_inner.payload_inner.payload_inner.block_hash = hash;
    submission.message.block_hash = hash;
    let error = simulate(&harness, V6, &request).await?.expect("the access list is wrong");
    assert!(error.starts_with("block access list hash mismatch"), "{error}");

    Ok(())
}

#[tokio::test]
async fn rejects_a_blacklisted_sender_only_when_the_relay_applies_the_blacklist() -> Result<()> {
    let sender = Account::Alice.address();
    let blacklist = serve_disallow_list(format!(r#"{{"items":["{sender}"]}}"#)).await?;
    let harness = simulator_node(build_test_genesis().into(), blacklist.as_str()).await?;
    let EnginePayload::V6(envelope) = build_tipping_payload(&harness).await? else {
        bail!("an Amsterdam chain builds V6 payloads");
    };
    let mut request = v6_request(&envelope)?;

    // The node fetches the list as it starts; poll until the list is in force.
    request.apply_blacklist = true;
    let expected = format!("block accesses blacklisted address: {sender}");
    let mut error = simulate(&harness, V6, &request).await?;
    for _ in 0..50 {
        if error.as_deref() == Some(expected.as_str()) {
            break;
        }
        sleep(Duration::from_millis(100)).await;
        error = simulate(&harness, V6, &request).await?;
    }
    assert_eq!(error.as_deref(), Some(expected.as_str()));

    request.apply_blacklist = false;
    assert_eq!(simulate(&harness, V6, &request).await?, None);
    Ok(())
}

#[tokio::test]
async fn inclusion_list_binds_only_transactions_that_would_succeed() -> Result<()> {
    let harness = simulator_node(build_test_genesis().into(), NO_BLACKLIST).await?;
    let EnginePayload::V6(envelope) = build_payload(&harness).await? else {
        bail!("an Amsterdam chain builds V6 payloads");
    };
    let mut request = v6_request(&envelope)?;

    request.inclusion_list = Some(inclusion_list(tipping_transfer(0)?));
    assert_eq!(
        simulate(&harness, V6, &request).await?.as_deref(),
        Some("inclusion list not satisfied")
    );

    // Alice's nonce is 0, so a transfer with nonce 1 fails on top of the block.
    request.inclusion_list = Some(inclusion_list(tipping_transfer(1)?));
    assert_eq!(simulate(&harness, V6, &request).await?, None);
    Ok(())
}

#[tokio::test]
async fn inclusion_list_publishes_pending_transactions_after_each_block() -> Result<()> {
    let harness = simulator_node(build_test_genesis().into(), NO_BLACKLIST).await?;
    let client = harness.rpc_client()?;

    // The payload is built before the transfer reaches the pool, so the transfer is still
    // pending when the block turns canonical.
    let payload = build_payload(&harness).await?;
    let block = &payload.payload_v3().payload_inner.payload_inner;
    let (parent, head) = (block.parent_hash, block.block_hash);
    let transfer = tipping_transfer(0)?;
    let _ = harness.provider().send_raw_transaction(&transfer).await?;
    // The list is built from the pool events received before the block's commit.
    sleep(Duration::from_millis(100)).await;
    let status = harness.engine().new_payload(payload, B256::ZERO).await?;
    if !status.is_valid() {
        bail!("the engine rejected the payload: {status:?}");
    }
    harness.engine().update_forkchoice(parent, head, None).await?;

    // The method fails until the first list is published.
    let mut published = None;
    for _ in 0..50 {
        if let Ok(list) = client.request_noparams::<Vec<Bytes>>("relay_inclusionList").await {
            published = Some(list);
            break;
        }
        sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(published, Some(vec![transfer]));
    Ok(())
}
