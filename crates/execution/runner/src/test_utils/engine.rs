//! Engine API integration for canonical block production.

use std::{fmt, marker::PhantomData, sync::Arc};

use alloy_eips::eip7685::{Requests, RequestsOrHash};
use alloy_primitives::B256;
use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV4, ExecutionPayloadEnvelopeV5, ExecutionPayloadEnvelopeV6,
    ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated, PayloadAttributes, PayloadId,
    PayloadStatus,
};
use eyre::Result;
use jsonrpsee::core::client::SubscriptionClientT;
use reth_chainspec::{ChainSpec, EthereumHardforks};
use reth_node_ethereum::EthEngineTypes;
use reth_rpc_api::EngineApiClient;
use reth_rpc_layer::JwtSecret;
use tracing::debug;

use crate::test_utils::constants::DEFAULT_JWT_SECRET;

/// Describes how to reach the Engine API endpoint.
#[derive(Clone, Debug)]
pub enum EngineAddress {
    /// Connect to an IPC endpoint.
    Ipc(String),
}

/// Abstraction over engine transports.
pub trait EngineProtocol: Send + Sync {
    /// Build a subscription-capable client for the Engine API.
    fn client(
        jwt: JwtSecret,
        address: EngineAddress,
    ) -> impl std::future::Future<Output = impl SubscriptionClientT + Send + Sync + Unpin + 'static> + Send;
}

/// Implementation of [`EngineProtocol`] that talks to the Engine API over IPC.
#[derive(Debug, Default, Clone, Copy)]
pub struct IpcEngine;

impl EngineProtocol for IpcEngine {
    async fn client(
        _: JwtSecret,
        address: EngineAddress,
    ) -> impl SubscriptionClientT + Send + Sync + Unpin + 'static {
        let EngineAddress::Ipc(path) = address;
        reth_ipc::client::IpcClientBuilder::default()
            .build(&path)
            .await
            .expect("Failed to create ipc client")
    }
}

/// A payload the engine built, from the `engine_getPayload` version its timestamp requires.
#[derive(Debug, Clone)]
pub enum EnginePayload {
    /// Prague, from `engine_getPayloadV4`.
    V4(ExecutionPayloadEnvelopeV4),
    /// Osaka, from `engine_getPayloadV5`.
    V5(ExecutionPayloadEnvelopeV5),
    /// Amsterdam, from `engine_getPayloadV6`.
    V6(ExecutionPayloadEnvelopeV6),
}

impl EnginePayload {
    /// The payload fields every version carries.
    pub const fn payload_v3(&self) -> &ExecutionPayloadV3 {
        match self {
            Self::V4(envelope) => &envelope.envelope_inner.execution_payload,
            Self::V5(envelope) => &envelope.execution_payload,
            Self::V6(envelope) => &envelope.execution_payload.payload_inner,
        }
    }

    /// The block's EIP-7685 requests.
    pub const fn execution_requests(&self) -> &Requests {
        match self {
            Self::V4(envelope) => &envelope.execution_requests,
            Self::V5(envelope) => &envelope.execution_requests,
            Self::V6(envelope) => &envelope.execution_requests,
        }
    }
}

/// Thin wrapper around a typed Engine API client.
///
/// It speaks the method versions each fork requires, from the chain spec: `forkchoiceUpdatedV4`
/// everywhere, `getPayloadV4` and `newPayloadV4` before Osaka, `getPayloadV5` and
/// `newPayloadV4` on Osaka, `getPayloadV6` and `newPayloadV5` from Amsterdam.
pub struct EngineApi<P: EngineProtocol = IpcEngine> {
    address: EngineAddress,
    jwt_secret: JwtSecret,
    chain_spec: Arc<ChainSpec>,
    _phantom: PhantomData<P>,
}

impl EngineApi<IpcEngine> {
    /// Build a new IPC-backed Engine API client for the chain `chain_spec` describes.
    pub fn new(path: String, chain_spec: Arc<ChainSpec>) -> Result<Self> {
        let jwt_secret = JwtSecret::from_hex(DEFAULT_JWT_SECRET.to_string())?;
        Ok(Self {
            address: EngineAddress::Ipc(path),
            jwt_secret,
            chain_spec,
            _phantom: PhantomData,
        })
    }
}

impl<P: EngineProtocol> fmt::Debug for EngineApi<P> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EngineApi").field("address", &self.address).finish_non_exhaustive()
    }
}

impl<P: EngineProtocol> EngineApi<P> {
    async fn client(&self) -> impl SubscriptionClientT + Send + Sync + Unpin + 'static + use<P> {
        P::client(self.jwt_secret, self.address.clone()).await
    }

    /// Get the payload built for a block at `timestamp`.
    pub async fn get_payload(
        &self,
        payload_id: PayloadId,
        timestamp: u64,
    ) -> Result<EnginePayload> {
        debug!(payload_id = %payload_id, "Fetching payload");
        let client = self.client().await;
        Ok(if self.chain_spec.is_amsterdam_active_at_timestamp(timestamp) {
            EnginePayload::V6(
                EngineApiClient::<EthEngineTypes>::get_payload_v6(&client, payload_id).await?,
            )
        } else if self.chain_spec.is_osaka_active_at_timestamp(timestamp) {
            EnginePayload::V5(
                EngineApiClient::<EthEngineTypes>::get_payload_v5(&client, payload_id).await?,
            )
        } else {
            EnginePayload::V4(
                EngineApiClient::<EthEngineTypes>::get_payload_v4(&client, payload_id).await?,
            )
        })
    }

    /// Submit a payload the engine built, with no blobs.
    pub async fn new_payload(
        &self,
        payload: EnginePayload,
        parent_beacon_block_root: B256,
    ) -> Result<PayloadStatus> {
        debug!("Submitting new payload");
        let client = self.client().await;
        let requests = RequestsOrHash::Requests(payload.execution_requests().clone());
        Ok(match payload {
            EnginePayload::V6(envelope) => {
                EngineApiClient::<EthEngineTypes>::new_payload_v5(
                    &client,
                    envelope.execution_payload,
                    vec![],
                    parent_beacon_block_root,
                    requests,
                )
                .await?
            }
            payload => {
                EngineApiClient::<EthEngineTypes>::new_payload_v4(
                    &client,
                    payload.payload_v3().clone(),
                    vec![],
                    parent_beacon_block_root,
                    requests,
                )
                .await?
            }
        })
    }

    /// Update forkchoice.
    pub async fn update_forkchoice(
        &self,
        current_head: B256,
        new_head: B256,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated> {
        debug!("Updating forkchoice (current: {current_head}, new: {new_head})");
        let result = EngineApiClient::<EthEngineTypes>::fork_choice_updated_v4(
            &self.client().await,
            ForkchoiceState {
                head_block_hash: new_head,
                safe_block_hash: current_head,
                finalized_block_hash: current_head,
            },
            payload_attributes,
            None,
        )
        .await;

        Ok(result?)
    }
}
