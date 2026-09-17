//! The node's `eth` API.
//!
//! A newtype over reth's [`EthApi`] that delegates everything except the locally built pending
//! block. See [`LoadPendingBlock`] below for why that one is overridden.

use std::{future::Future, sync::Arc};

use alloy_eips::{BlockId, BlockNumberOrTag, eip2718::WithEncoded};
use alloy_network::Ethereum;
use alloy_primitives::{B256, U256};
use reth_chainspec::{ChainSpecProvider, EthereumHardforks, Hardforks};
use reth_evm::ConfigureEvm;
use reth_node_api::{FullNodeComponents, HeaderTy, NodeTypes, PrimitivesTy};
use reth_node_builder::rpc::{EthApiBuilder, EthApiCtx};
use reth_provider::{
    BlockIdReader, BlockReaderIdExt, ProviderError, ProviderHeader, StateProviderBox,
    StateProviderFactory,
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

use reth_chain_state::BlockState;

use crate::pending_state::PendingStateSource;
use reth_transaction_pool::{PoolTx, TransactionOrigin};

/// The node's `eth` API.
#[derive(Debug)]
pub struct EthgasEthApi<N: RpcNodeCore, Rpc: RpcConvert> {
    inner: EthApi<N, Rpc>,
    pending_state: Option<Arc<dyn PendingStateSource>>,
}

impl<N: RpcNodeCore, Rpc: RpcConvert> Clone for EthgasEthApi<N, Rpc> {
    fn clone(&self) -> Self {
        Self { inner: self.inner.clone(), pending_state: self.pending_state.clone() }
    }
}

impl<N: RpcNodeCore, Rpc: RpcConvert> EthgasEthApi<N, Rpc> {
    /// Wraps a reth [`EthApi`].
    ///
    /// Without a `pending_state`, the `pending` tag resolves to the canonical tip for every
    /// method this node does not override.
    pub const fn new(
        inner: EthApi<N, Rpc>,
        pending_state: Option<Arc<dyn PendingStateSource>>,
    ) -> Self {
        Self { inner, pending_state }
    }

    fn serves_pending_overlay(&self) -> bool {
        self.pending_state.as_ref().is_some_and(|source| source.pending_overlay().is_some())
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

    /// Refuses `pending` for the three methods that cannot answer it correctly.
    ///
    /// `eth_getProof`, `eth_getMultiProof` and `eth_getAccount` are the only callers of this
    /// method, so this is where the three of them are carved out. Each needs a state root, and
    /// the flashblocks overlay has no trie data: a proof built over it would splice the parent
    /// trie's branch hashes around a leaf holding pending values. That response is structurally
    /// valid and hashes to no block on any chain, which is worse than refusing, because nothing
    /// signals it to the caller.
    ///
    /// Conditioned on an overlay actually being served. Without one, `pending` resolves through
    /// the provider to a real executed block, and a proof over that is a real proof — so a node
    /// running without flashblocks keeps answering these three exactly as reth does.
    fn ensure_within_proof_window(&self, block_id: BlockId) -> Result<(), Self::Error>
    where
        Self: EthApiSpec,
    {
        if block_id.is_pending() && self.serves_pending_overlay() {
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

    /// Answers `pending` from the flashblocks snapshot, overlaid on the state it was built on.
    ///
    /// This is the single fork every pending state read passes through, so returning a provider
    /// here makes `eth_getCode`, `eth_getStorageAt`, `eth_getAccountInfo` and
    /// `eth_getStorageValues` flashblocks-aware at once, along with `eth_getBalance` on the path
    /// where its override finds no builder-reported balance.
    ///
    /// Falls back to `Ok(None)` whenever there is no source, no published snapshot, or a snapshot
    /// with no recorded anchor. That resolves through `BlockchainProvider::pending()`, which
    /// serves the engine's own pending block when one exists and the canonical tip otherwise —
    /// real either way, and never reth's pool-built block, which would answer with transactions
    /// no builder selected in an order no builder chose.
    ///
    /// The overlay carries no trie data, so the proof methods must not reach it. They do not:
    /// [`EthState::ensure_within_proof_window`] above refuses `pending` for all three.
    async fn local_pending_state(&self) -> Result<Option<StateProviderBox>, Self::Error>
    where
        Self: SpawnBlocking,
    {
        let Some(overlay) = self.pending_state.as_ref().and_then(|source| source.pending_overlay())
        else {
            return Ok(None);
        };

        // Stand aside for a real executed block. Between `newPayload` and `forkchoiceUpdated` the
        // engine holds a complete, executed pending block, and returning `Ok(None)` resolves to
        // it. That block carries the transactions that actually sealed and the withdrawals this
        // node never applies to the bundle, so where it is at least as new as the snapshot it is
        // strictly the better answer and the reconstruction must not outrank it.
        if let Ok(Some(engine_pending)) = self.provider().pending_block_num_hash() &&
            engine_pending.number >= overlay.latest_block_number
        {
            return Ok(None);
        }

        // Anchored by hash, not by number: the snapshot's bundle is only coherent on top of the
        // exact block it was executed against, and a number resolves elsewhere after a reorg.
        let historical = self.provider().history_by_block_hash(overlay.anchor.hash()).map_err(
            |err| -> Self::Error { <EthApiError as From<ProviderError>>::from(err).into() },
        )?;

        Ok(Some(Box::new(BlockState::new(overlay.executed).state_provider(historical))
            as StateProviderBox))
    }

    /// Returns the canonical tip as the locally built pending block.
    ///
    /// Methods this node overrides answer `pending` from flashblocks. The rest land here, and the
    /// canonical tip is behind the flashblocks view but real, where a pool-built block is not.
    /// Note this is the *block*, not the state: [`Self::local_pending_state`] above is what
    /// answers state reads.
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
pub struct EthgasEthApiBuilder {
    pending_state: Option<Arc<dyn PendingStateSource>>,
}

impl EthgasEthApiBuilder {
    /// Builds an `eth` API that answers `pending` from `pending_state`.
    ///
    /// `None` leaves the `pending` tag resolving to the canonical tip. Note that reth's
    /// `EthApiBuilder` requires `Default`, and the `Default` here carries no source — so a builder
    /// that reaches reth by that path serves canonical, never a pool-built block.
    pub const fn new(pending_state: Option<Arc<dyn PendingStateSource>>) -> Self {
        Self { pending_state }
    }
}

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
        Ok(EthgasEthApi::new(
            ctx.eth_api_builder().map_converter(|r| r.with_network()).build(),
            self.pending_state,
        ))
    }
}
