//! Engine API integration for canonical block production.

use std::{fmt, marker::PhantomData};

use alloy_eips::eip7685::RequestsOrHash;
use alloy_primitives::B256;
use alloy_rpc_types_engine::{
    ExecutionPayloadEnvelopeV4, ExecutionPayloadV3, ForkchoiceState, ForkchoiceUpdated,
    PayloadAttributes, PayloadId, PayloadStatus,
};
use eyre::Result;
use jsonrpsee::core::client::SubscriptionClientT;
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
    ) -> impl std::future::Future<
        Output = impl SubscriptionClientT + Send + Sync + Unpin + 'static,
    > + Send;
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

/// Thin wrapper around a typed Engine API client.
pub struct EngineApi<P: EngineProtocol = IpcEngine> {
    address: EngineAddress,
    jwt_secret: JwtSecret,
    _phantom: PhantomData<P>,
}

impl EngineApi<IpcEngine> {
    /// Build a new IPC-backed Engine API client.
    pub fn new(path: String) -> Result<Self> {
        let jwt_secret = JwtSecret::from_hex(DEFAULT_JWT_SECRET.to_string())?;
        Ok(Self { address: EngineAddress::Ipc(path), jwt_secret, _phantom: PhantomData })
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

    /// Get a payload by ID.
    pub async fn get_payload(
        &self,
        payload_id: PayloadId,
    ) -> Result<ExecutionPayloadEnvelopeV4> {
        debug!(payload_id = %payload_id, "Fetching payload");
        Ok(EngineApiClient::<EthEngineTypes>::get_payload_v4(&self.client().await, payload_id)
            .await?)
    }

    /// Submit a new payload.
    pub async fn new_payload(
        &self,
        payload: ExecutionPayloadV3,
        versioned_hashes: Vec<B256>,
        parent_beacon_block_root: B256,
        execution_requests: RequestsOrHash,
    ) -> Result<PayloadStatus> {
        debug!("Submitting new payload");
        Ok(EngineApiClient::<EthEngineTypes>::new_payload_v4(
            &self.client().await,
            payload,
            versioned_hashes,
            parent_beacon_block_root,
            execution_requests,
        )
        .await?)
    }

    /// Update forkchoice.
    pub async fn update_forkchoice(
        &self,
        current_head: B256,
        new_head: B256,
        payload_attributes: Option<PayloadAttributes>,
    ) -> Result<ForkchoiceUpdated> {
        debug!("Updating forkchoice (current: {current_head}, new: {new_head})");
        let result = EngineApiClient::<EthEngineTypes>::fork_choice_updated_v3(
            &self.client().await,
            ForkchoiceState {
                head_block_hash: new_head,
                safe_block_hash: current_head,
                finalized_block_hash: current_head,
            },
            payload_attributes,
        )
        .await;

        Ok(result?)
    }
}
