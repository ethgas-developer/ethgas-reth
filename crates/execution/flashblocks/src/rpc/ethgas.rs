//! The `ethgas` namespace, and the `eth` fee overrides that serve the same value.

use std::{
    sync::Arc,
    time::{Duration, Instant},
};

use alloy_eips::BlockId;
use alloy_network::Ethereum;
use alloy_primitives::U256;
use alloy_rpc_types::TransactionRequest;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
};
use jsonrpsee_types::ErrorObject;
use reth_ethereum_primitives::{EthPrimitives, TransactionSigned};
use reth_provider::BlockNumReader;
use reth_rpc_eth_api::{
    RpcNodeCore,
    helpers::{EthFees, EthState, EthTransactions, FullEthApi},
};
use reth_rpc_eth_types::FillTransaction;

use crate::{
    fee::{InclusionPriorityFee, builder_inclusion_fee, fallback_inclusion_fee},
    metrics::Metrics,
    traits::FlashblocksAPI,
};

#[cfg_attr(not(test), rpc(server, namespace = "ethgas"))]
#[cfg_attr(test, rpc(server, client, namespace = "ethgas"))]
pub trait EthgasApi {
    /// The fee that clears the builder's gate for the next flashblock, or the node's own
    /// suggestion.
    #[method(name = "inclusionPriorityFee")]
    async fn inclusion_priority_fee(&self) -> RpcResult<InclusionPriorityFee>;
}

/// `eth_maxPriorityFeePerGas` and `eth_gasPrice` served from the builder's inclusion fee, with the
/// node's own values as fallback.
#[cfg_attr(not(test), rpc(server, namespace = "eth"))]
#[cfg_attr(test, rpc(server, client, namespace = "eth"))]
pub trait EthFeeOverride {
    /// Returns the value of `ethgas_inclusionPriorityFee` as a bare quantity.
    #[method(name = "maxPriorityFeePerGas")]
    async fn max_priority_fee_per_gas(&self) -> RpcResult<U256>;

    /// Returns the next block's base fee plus the builder's inclusion fee, or the node's own gas
    /// price.
    #[method(name = "gasPrice")]
    async fn gas_price(&self) -> RpcResult<U256>;

    /// Fills a transaction's defaults. A request that sets no fee gets the builder's inclusion fee
    /// as its priority fee while that fee is served, and a request without a nonce gets the
    /// `pending` transaction count, which counts the flashblocks and this node's mempool.
    #[method(name = "fillTransaction")]
    async fn fill_transaction(
        &self,
        request: TransactionRequest,
    ) -> RpcResult<FillTransaction<TransactionSigned>>;
}

/// Serves the builder's inclusion fee with the node's oracle as fallback.
#[derive(Debug)]
pub struct EthgasApiExt<Eth, FB> {
    eth_api: Eth,
    flashblocks_state: Arc<FB>,
    max_age: Duration,
    ceiling: U256,
    metrics: Metrics,
}

impl<Eth: Clone, FB> Clone for EthgasApiExt<Eth, FB> {
    fn clone(&self) -> Self {
        Self {
            eth_api: self.eth_api.clone(),
            flashblocks_state: Arc::clone(&self.flashblocks_state),
            max_age: self.max_age,
            ceiling: self.ceiling,
            metrics: self.metrics.clone(),
        }
    }
}

impl<Eth, FB> EthgasApiExt<Eth, FB> {
    pub fn new(eth_api: Eth, flashblocks_state: Arc<FB>, max_age: Duration, ceiling: U256) -> Self {
        Self { eth_api, flashblocks_state, max_age, ceiling, metrics: Metrics::default() }
    }
}

impl<Eth, FB> EthgasApiExt<Eth, FB>
where
    Eth: FullEthApi + Send + Sync + 'static,
    FB: FlashblocksAPI + Send + Sync + 'static,
    ErrorObject<'static>: From<Eth::Error>,
{
    fn builder_fee(&self) -> Option<InclusionPriorityFee> {
        let latest = self.flashblocks_state.latest_inclusion_fee()?;
        // The fee prices the block its flashblock belongs to. Once that block is sealed, the next
        // one can be another builder's, which has no inclusion gate.
        if self.eth_api.provider().best_block_number().is_ok_and(|best| latest.block_number <= best)
        {
            return None;
        }
        builder_inclusion_fee(Some(&latest), Instant::now(), self.max_age, self.ceiling)
    }

    async fn inclusion_fee(&self) -> RpcResult<InclusionPriorityFee> {
        if let Some(served) = self.builder_fee() {
            self.metrics.rpc_inclusion_fee_builder.increment(1);
            return Ok(served);
        }

        let suggested = EthFees::suggested_priority_fee(&self.eth_api).await?;
        self.metrics.rpc_inclusion_fee_fallback.increment(1);
        Ok(fallback_inclusion_fee(suggested))
    }
}

#[async_trait]
impl<Eth, FB> EthgasApiServer for EthgasApiExt<Eth, FB>
where
    Eth: FullEthApi + Send + Sync + 'static,
    FB: FlashblocksAPI + Send + Sync + 'static,
    ErrorObject<'static>: From<Eth::Error>,
{
    async fn inclusion_priority_fee(&self) -> RpcResult<InclusionPriorityFee> {
        self.inclusion_fee().await
    }
}

#[async_trait]
impl<Eth, FB> EthFeeOverrideServer for EthgasApiExt<Eth, FB>
where
    Eth: FullEthApi<NetworkTypes = Ethereum>
        + RpcNodeCore<Primitives = EthPrimitives>
        + Send
        + Sync
        + 'static,
    FB: FlashblocksAPI + Send + Sync + 'static,
    ErrorObject<'static>: From<Eth::Error>,
{
    async fn max_priority_fee_per_gas(&self) -> RpcResult<U256> {
        Ok(self.inclusion_fee().await?.max_priority_fee_per_gas)
    }

    async fn gas_price(&self) -> RpcResult<U256> {
        if let Some(served) = self.builder_fee() {
            let base_fee = EthFees::base_fee(&self.eth_api).await?.unwrap_or_default();
            self.metrics.rpc_inclusion_fee_builder.increment(1);
            return Ok(base_fee.saturating_add(served.max_priority_fee_per_gas));
        }

        let gas_price = EthFees::gas_price(&self.eth_api).await?;
        self.metrics.rpc_inclusion_fee_fallback.increment(1);
        Ok(gas_price)
    }

    async fn fill_transaction(
        &self,
        mut request: TransactionRequest,
    ) -> RpcResult<FillTransaction<TransactionSigned>> {
        // reth fills a missing nonce from the latest state and this node's pool. The count at
        // `pending` adds the flashblocks' executed nonce, as `eth_getTransactionCount` serves it.
        if request.nonce.is_none() &&
            let Some(from) = request.from
        {
            let next = EthState::transaction_count(&self.eth_api, from, Some(BlockId::pending()))
                .await
                .map_err(Into::into)?;
            request.nonce = Some(next.saturating_to());
        }

        let sets_no_fee = request.gas_price.is_none() &&
            request.max_fee_per_gas.is_none() &&
            request.max_priority_fee_per_gas.is_none();
        // reth fills the rest around a priority fee that is set: `maxFeePerGas` becomes twice the
        // base fee plus this fee.
        if sets_no_fee && let Some(served) = self.builder_fee() {
            request.max_priority_fee_per_gas =
                Some(served.max_priority_fee_per_gas.saturating_to());
            self.metrics.rpc_inclusion_fee_builder.increment(1);
        }

        EthTransactions::fill_transaction(&self.eth_api, request).await.map_err(Into::into)
    }
}
