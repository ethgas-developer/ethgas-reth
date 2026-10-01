//! The node's `eth` API.
//!
//! A newtype over reth's [`EthApi`] that delegates everything except the locally built pending
//! block. See [`LoadPendingBlock`] below for why that one is overridden.

use std::{future::Future, sync::Arc};

use alloy_consensus::BlockHeader;
use alloy_eips::{BlockId, BlockNumberOrTag, eip2718::WithEncoded};
use alloy_network::Ethereum;
use alloy_primitives::{B256, U256};
use reth_chainspec::{ChainSpecProvider, EthereumHardforks, Hardforks};
use reth_ethereum_primitives::EthPrimitives;
use reth_evm::{ConfigureEvm, EvmEnvFor};
use reth_node_api::{FullNodeComponents, HeaderTy, NodeTypes, PrimitivesTy};
use reth_node_builder::rpc::{EthApiBuilder, EthApiCtx};
use reth_provider::{
    BlockIdReader, BlockNumReader, BlockReaderIdExt, ProviderError, ProviderHeader,
    StateProviderBox, StateProviderFactory,
};
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

use crate::pending_state::{PendingOverlay, PendingStateSource};
use reth_transaction_pool::{PoolTx, TransactionOrigin};

/// The node's `eth` API.
#[derive(Debug)]
pub struct EthgasEthApi<N: RpcNodeCore, Rpc: RpcConvert> {
    inner: EthApi<N, Rpc>,
    pending_state: Option<Arc<dyn PendingStateSource<N::Primitives>>>,
}

impl<N: RpcNodeCore, Rpc: RpcConvert> Clone for EthgasEthApi<N, Rpc> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), pending_state: self.pending_state.clone() }
    }
}

impl<N: RpcNodeCore, Rpc: RpcConvert> EthgasEthApi<N, Rpc> {
    /// Wraps a reth [`EthApi`].
    pub const fn new(
        inner: EthApi<N, Rpc>,
        pending_state: Option<Arc<dyn PendingStateSource<N::Primitives>>>,
    ) -> Self {
        Self { inner, pending_state }
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

impl<N, Rpc> EthgasEthApi<N, Rpc>
where
    N: RpcNodeCore,
    Rpc: RpcConvert<Primitives = N::Primitives>,
{
    /// The overlay that `pending` resolves to, or `None` when it resolves through the provider.
    ///
    /// An executed engine block at least as new as the snapshot is the better answer: it holds
    /// the transactions that sealed and the withdrawals the bundle omits. And the bundle is only
    /// coherent on the exact block it was executed against; the overlay resolves that block
    /// lazily, on its first read, so the check is here, where `None` still falls back.
    fn served_overlay(&self) -> Result<Option<PendingOverlay<N::Primitives>>, EthApiError> {
        let Some(overlay) = self.pending_state.as_ref().and_then(|source| source.pending_overlay())
        else {
            return Ok(None);
        };

        if let Ok(Some(engine_pending)) = self.provider().pending_block_num_hash() &&
            engine_pending.number >= overlay.latest_header.number()
        {
            return Ok(None);
        }

        if self.provider().block_number(overlay.anchor.hash())?.is_none() {
            return Ok(None);
        }

        Ok(Some(overlay))
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
    /// Runs `pending` in the pending block's environment, on the overlay.
    ///
    /// reth pairs a derived pending environment with the latest block's state id, which never
    /// reaches `local_pending_state`. With an overlay to serve, the pair is the snapshot's latest
    /// header and the `pending` tag, so `eth_call`, `eth_estimateGas` and the call-tracing
    /// methods execute on the state that header describes. Without one, reth's own pairing
    /// applies: the engine's block, or the latest block with a derived environment.
    async fn evm_env_at(&self, at: BlockId) -> Result<(EvmEnvFor<Self::Evm>, BlockId), Self::Error>
    where
        Self: SpawnBlocking,
    {
        let api_err = |err: EthApiError| -> Self::Error { err.into() };
        if at.is_pending() &&
            let Some(overlay) = self.served_overlay().map_err(api_err)?
        {
            let evm_env = self.inner.evm_env_for_header(&overlay.latest_header).map_err(api_err)?;
            return Ok((evm_env, BlockId::pending()));
        }

        self.inner.evm_env_at(at).await.map_err(api_err)
    }
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

    /// Refuses `pending` while an overlay is served.
    ///
    /// `eth_getProof`, `eth_getMultiProof` and `eth_getAccount` are the only callers. The overlay
    /// has no trie data, so a proof over it would be well-formed and match no block. When no
    /// overlay is served, `pending` resolves through the provider and reth's own check applies.
    fn ensure_within_proof_window(&self, block_id: BlockId) -> Result<(), Self::Error>
    where
        Self: EthApiSpec,
    {
        let api_err = |err: EthApiError| -> Self::Error { err.into() };
        if block_id.is_pending() && self.served_overlay().map_err(api_err)?.is_some() {
            return Err(EthApiError::InvalidParams(
                "pending is not supported by this method: pending state is built from \
                 flashblocks and carries no state root, so no proof can be derived from it"
                    .to_string(),
            )
            .into());
        }

        self.inner.ensure_within_proof_window(block_id).map_err(Into::into)
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

    /// Answers `pending` from the flashblocks snapshot, overlaid on the block it was built on.
    ///
    /// Every pending state read this node does not override passes through here. `Ok(None)`
    /// resolves through `BlockchainProvider::pending()`: the engine's pending block when it holds
    /// one, the latest block otherwise, never a pool-built block.
    async fn local_pending_state(&self) -> Result<Option<StateProviderBox>, Self::Error>
    where
        Self: SpawnBlocking,
    {
        let Some(overlay) = self.served_overlay()? else {
            return Ok(None);
        };

        let state = self
            .provider()
            .state_with_block_appended(overlay.anchor.hash(), overlay.executed)
            .map_err(|err| -> Self::Error {
                <EthApiError as From<ProviderError>>::from(err).into()
            })?;

        Ok(Some(state))
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

        Ok(cached.map(|(block, receipts)| BlockAndReceipts::new(block, receipts)))
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
pub struct EthgasEthApiBuilder {
    pending_state: Option<Arc<dyn PendingStateSource>>,
}

impl EthgasEthApiBuilder {
    /// Builds an `eth` API that answers `pending` from `pending_state`, or from the provider when
    /// it is `None`.
    pub const fn new(pending_state: Option<Arc<dyn PendingStateSource>>) -> Self {
        Self { pending_state }
    }
}

impl<N> EthApiBuilder<N> for EthgasEthApiBuilder
where
    N: FullNodeComponents<
            Types: NodeTypes<Primitives = EthPrimitives, ChainSpec: Hardforks + EthereumHardforks>,
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
        Ok(EthgasEthApi::new(
            ctx.eth_api_builder().map_converter(|r| r.with_network()).build(),
            self.pending_state,
        ))
    }
}
