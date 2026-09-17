//! The node's `eth` API.
//!
//! A newtype over reth's [`EthApi`] that delegates everything except the locally built pending
//! block. See [`LoadPendingBlock`] below for why that one is overridden.

use std::{future::Future, sync::Arc};

use alloy_eips::{BlockNumberOrTag, eip2718::WithEncoded};
use alloy_network::Ethereum;
use alloy_primitives::{B256, U256};
use reth_chainspec::{ChainSpecProvider, EthereumHardforks, Hardforks};
use reth_evm::ConfigureEvm;
use reth_node_api::{FullNodeComponents, HeaderTy, NodeTypes, PrimitivesTy};
use reth_node_builder::rpc::{EthApiBuilder, EthApiCtx};
use reth_provider::{BlockReaderIdExt, ProviderHeader, StateProviderBox};
use reth_rpc::{EthApi, eth::core::EthRpcConverterFor};
use reth_rpc_eth_api::{
    EthApiTypes, FromEvmError, RpcConvert, RpcNodeCore, RpcNodeCoreExt,
    helpers::{
        Call, EthApiSpec, EthBlocks, EthCall, EthFees, EthState, EthTransactions, LoadBlock,
        LoadFee, LoadPendingBlock, LoadReceipt, LoadState, LoadTransaction, SpawnBlocking, Trace,
        bal::GetBlockAccessList,
        estimate::EstimateCall,
        pending_block::{BuildPendingEnv, PendingEnvBuilder},
        spec::SignersForRpc,
        subscriptions::EthSubscriptions,
    },
};
use reth_rpc_eth_types::{
    EthApiError, FeeHistoryCache, GasPriceOracle, PendingBlock, block::BlockAndReceipts,
    builder::config::PendingBlockKind,
};
use reth_tasks::pool::{BlockingTaskGuard, BlockingTaskPool};
use reth_transaction_pool::{PoolTx, TransactionOrigin};

/// The node's `eth` API.
#[derive(Debug)]
pub struct EthgasEthApi<N: RpcNodeCore, Rpc: RpcConvert> {
    inner: EthApi<N, Rpc>,
}

impl<N: RpcNodeCore, Rpc: RpcConvert> Clone for EthgasEthApi<N, Rpc> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone() }
    }
}

impl<N: RpcNodeCore, Rpc: RpcConvert> EthgasEthApi<N, Rpc> {
    /// Wraps a reth [`EthApi`].
    pub const fn new(inner: EthApi<N, Rpc>) -> Self {
        Self { inner }
    }

    /// Returns the wrapped reth [`EthApi`].
    pub const fn inner(&self) -> &EthApi<N, Rpc> {
        &self.inner
    }
}

impl<N, Rpc> EthApiTypes for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    type Error = EthApiError;
    type NetworkTypes = Rpc::Network;
    type RpcConvert = Rpc;

    fn converter(&self) -> &Self::RpcConvert {
        self.inner.converter()
    }

    fn eth_api_settings(&self) -> &reth_rpc_eth_types::EthApiSettings {
        self.inner.eth_api_settings()
    }
}

impl<N, Rpc> RpcNodeCore for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives>,
{
    type Primitives = N::Primitives;
    type Provider = N::Provider;
    type Pool = N::Pool;
    type Evm = N::Evm;
    type Network = N::Network;

    #[inline]
    fn pool(&self) -> &Self::Pool {
        self.inner.pool()
    }

    #[inline]
    fn evm_config(&self) -> &Self::Evm {
        self.inner.evm_config()
    }

    #[inline]
    fn network(&self) -> &Self::Network {
        self.inner.network()
    }

    #[inline]
    fn provider(&self) -> &Self::Provider {
        self.inner.provider()
    }
}

impl<N, Rpc> RpcNodeCoreExt for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives>,
{
    #[inline]
    fn cache(&self) -> &reth_rpc_eth_types::EthStateCache<N::Primitives> {
        self.inner.cache()
    }
}

impl<N, Rpc> EthApiSpec for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn starting_block(&self) -> U256 {
        self.inner.starting_block()
    }
}

impl<N, Rpc> SpawnBlocking for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn io_task_spawner(&self) -> &reth_tasks::Runtime {
        self.inner.io_task_spawner()
    }

    #[inline]
    fn tracing_task_pool(&self) -> &BlockingTaskPool {
        self.inner.tracing_task_pool()
    }

    #[inline]
    fn tracing_task_guard(&self) -> &BlockingTaskGuard {
        self.inner.tracing_task_guard()
    }

    #[inline]
    fn blocking_io_task_guard(&self) -> &Arc<tokio::sync::Semaphore> {
        self.inner.blocking_io_task_guard()
    }
}

impl<N, Rpc> LoadFee for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn gas_oracle(&self) -> &GasPriceOracle<Self::Provider> {
        self.inner.gas_oracle()
    }

    #[inline]
    fn fee_history_cache(&self) -> &FeeHistoryCache<ProviderHeader<Self::Provider>> {
        self.inner.fee_history_cache()
    }
}

impl<N, Rpc> LoadState for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> EthState for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn max_proof_window(&self) -> u64 {
        self.inner.max_proof_window()
    }
}

impl<N, Rpc> EthFees for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> Trace for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> LoadBlock for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> EthBlocks for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> LoadReceipt for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> LoadTransaction for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> EthTransactions for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn signers(&self) -> &SignersForRpc<Self::Provider, Self::NetworkTypes> {
        EthTransactions::signers(&self.inner)
    }

    #[inline]
    fn send_raw_transaction_sync_timeout(&self) -> std::time::Duration {
        self.inner.send_raw_transaction_sync_timeout()
    }

    #[inline]
    fn send_pool_transaction(
        &self,
        origin: TransactionOrigin,
        tx: WithEncoded<PoolTx<Self::Pool>>,
    ) -> impl Future<Output = Result<B256, Self::Error>> + Send {
        self.inner.send_pool_transaction(origin, tx)
    }
}

impl<N, Rpc> Call for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn call_gas_limit(&self) -> u64 {
        self.inner.call_gas_limit()
    }

    #[inline]
    fn max_simulate_blocks(&self) -> u64 {
        self.inner.max_simulate_blocks()
    }
}

impl<N, Rpc> EstimateCall for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> EthCall for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> LoadPendingBlock for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore<Provider: ChainSpecProvider<ChainSpec: EthereumHardforks>>,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    #[inline]
    fn pending_block(&self) -> &tokio::sync::Mutex<Option<PendingBlock<N::Primitives>>> {
        self.inner.pending_block()
    }

    #[inline]
    fn pending_env_builder(&self) -> &dyn PendingEnvBuilder<Self::Evm> {
        self.inner.pending_env_builder()
    }

    #[inline]
    fn pending_block_kind(&self) -> PendingBlockKind {
        self.inner.pending_block_kind()
    }

    /// Never overlays a pool-built block on latest.
    ///
    /// The flashblocks snapshot is the only pending state this node recognises. A pool-built
    /// overlay would answer `pending` with transactions no builder selected, in an order no
    /// builder chose.
    async fn local_pending_state(&self) -> Result<Option<StateProviderBox>, Self::Error>
    where
        Self: SpawnBlocking,
    {
        Ok(None)
    }

    /// Returns the canonical tip as the locally built pending block.
    ///
    /// Methods this node overrides answer `pending` from flashblocks. The rest land here, and the
    /// canonical tip is behind the flashblocks view but real, where a pool-built block is not.
    async fn local_pending_block(
        &self,
    ) -> Result<Option<BlockAndReceipts<Self::Primitives>>, Self::Error> {
        let latest = self
            .provider()
            .latest_header()?
            .ok_or_else(|| EthApiError::HeaderNotFound(BlockNumberOrTag::Latest.into()))?;

        let cached = self.cache().get_block_and_receipts(latest.hash()).await?;

        Ok(cached.map(|(block, receipts)| BlockAndReceipts { block, receipts }))
    }
}

impl<N, Rpc> GetBlockAccessList for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
}

impl<N, Rpc> EthSubscriptions for EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives, Error = EthApiError, Evm = N::Evm>,
    EthApiError: FromEvmError<N::Evm>,
{
    fn header_stream(
        &self,
    ) -> impl futures::Stream<Item = reth_rpc_eth_api::RpcHeader<Self::NetworkTypes>> + Send + Unpin
    {
        self.inner.header_stream()
    }
}

/// Builds [`EthgasEthApi`] in place of reth's own `eth` API.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct EthgasEthApiBuilder;

impl<N> EthApiBuilder<N> for EthgasEthApiBuilder
where
    N: FullNodeComponents<
            Types: NodeTypes<ChainSpec: Hardforks + EthereumHardforks>,
            Evm: ConfigureEvm<NextBlockEnvCtx: BuildPendingEnv<HeaderTy<N::Types>>>,
        >,
    alloy_rpc_types::TransactionRequest:
        reth_rpc_convert::SignableTxRequest<reth_node_api::TxTy<N::Types>>,
    EthRpcConverterFor<N, Ethereum>: RpcConvert<
            Primitives = PrimitivesTy<N::Types>,
            Error = EthApiError,
            Network = Ethereum,
            Evm = N::Evm,
        >,
    EthApiError: FromEvmError<N::Evm>,
{
    type EthApi = EthgasEthApi<N, EthRpcConverterFor<N, Ethereum>>;

    async fn build_eth_api(self, ctx: EthApiCtx<'_, N>) -> eyre::Result<Self::EthApi> {
        Ok(EthgasEthApi::new(ctx.eth_api_builder().map_converter(|r| r.with_network()).build()))
    }
}
