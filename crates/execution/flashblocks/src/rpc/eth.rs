//! RPC trait definitions and implementations for flashblocks.

use std::{collections::HashSet, sync::Arc, time::Duration};

use alloy_eips::{BlockId, BlockNumberOrTag};
use alloy_network::Ethereum;
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types::{
    BlockOverrides, Filter, Index, Log, TransactionRequest,
    simulate::{SimulatePayload, SimulatedBlock},
    state::EvmOverrides,
};
use alloy_rpc_types_eth::state::StateOverride;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
};
use jsonrpsee_types::ErrorObjectOwned;
use reth_primitives_traits::NodePrimitives;
use reth_provider::{CanonStateNotifications, CanonStateSubscriptions, StateProvider};
use reth_rpc::EthFilter;
use reth_rpc_eth_api::{
    EthApiTypes, EthFilterApiServer, FromEthApiError, RpcBlock, RpcReceipt, RpcTransaction,
    helpers::{EthBlocks, EthCall, EthState, EthTransactions, FullEthApi, LoadState},
};
use reth_rpc_eth_types::EthApiError;
use tokio::{
    sync::broadcast::{self, error::RecvError},
    time,
};
use tokio_stream::{
    StreamExt,
    wrappers::{BroadcastStream, errors::BroadcastStreamRecvError},
};
use tracing::{debug, trace, warn};

use crate::{
    metrics::Metrics,
    pending_blocks::PendingBlocks,
    traits::{FlashblocksAPI, PendingBlocksAPI},
};

/// Eth API override trait for flashblocks integration.
#[cfg_attr(not(test), rpc(server, namespace = "eth"))]
#[cfg_attr(test, rpc(server, client, namespace = "eth"))]
pub trait EthApiOverride {
    /// Returns block by number, with flashblock support for pending blocks.
    #[method(name = "getBlockByNumber")]
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> RpcResult<Option<RpcBlock<Ethereum>>>;

    /// Returns transaction receipt, checking flashblocks first.
    #[method(name = "getTransactionReceipt")]
    async fn get_transaction_receipt(
        &self,
        tx_hash: TxHash,
    ) -> RpcResult<Option<RpcReceipt<Ethereum>>>;

    /// Returns account balance, with flashblock support for pending state.
    #[method(name = "getBalance")]
    async fn get_balance(&self, address: Address, block_number: Option<BlockId>)
    -> RpcResult<U256>;

    /// Returns transaction count for an address.
    #[method(name = "getTransactionCount")]
    async fn get_transaction_count(
        &self,
        address: Address,
        block_number: Option<BlockId>,
    ) -> RpcResult<U256>;

    /// Returns transaction by hash, checking flashblocks first.
    #[method(name = "getTransactionByHash")]
    async fn transaction_by_hash(
        &self,
        tx_hash: TxHash,
    ) -> RpcResult<Option<RpcTransaction<Ethereum>>>;

    /// Sends a raw transaction and waits for inclusion in a flashblock.
    #[method(name = "sendRawTransactionSync")]
    async fn send_raw_transaction_sync(
        &self,
        transaction: alloy_primitives::Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Ethereum>>;

    /// Executes a call with flashblock state support.
    #[method(name = "call")]
    async fn call(
        &self,
        transaction: TransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<alloy_primitives::Bytes>;

    /// Estimates gas with flashblock state support.
    #[method(name = "estimateGas")]
    async fn estimate_gas(
        &self,
        transaction: TransactionRequest,
        block_number: Option<BlockId>,
        overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256>;

    /// Simulates transactions with flashblock state support.
    #[method(name = "simulateV1")]
    async fn simulate_v1(
        &self,
        opts: SimulatePayload<TransactionRequest>,
        block_number: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<RpcBlock<Ethereum>>>>;

    /// Returns logs matching the filter, including pending flashblock logs.
    #[method(name = "getLogs")]
    async fn get_logs(&self, filter: Filter) -> RpcResult<Vec<Log>>;

    /// Returns the number of transactions in a block by block number.
    #[method(name = "getBlockTransactionCountByNumber")]
    async fn get_block_transaction_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>>;

    /// Returns the receipts of a block, with the pending block built from flashblocks.
    #[method(name = "getBlockReceipts")]
    async fn block_receipts(
        &self,
        block_id: BlockId,
    ) -> RpcResult<Option<Vec<RpcReceipt<Ethereum>>>>;

    /// Returns a transaction by block number and index, with the pending block built from
    /// flashblocks.
    #[method(name = "getTransactionByBlockNumberAndIndex")]
    async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumberOrTag,
        index: Index,
    ) -> RpcResult<Option<RpcTransaction<Ethereum>>>;
}

/// Extended Eth API with flashblocks support.
#[derive(Debug)]
pub struct EthApiExt<Eth: EthApiTypes, FB> {
    eth_api: Eth,
    eth_filter: EthFilter<Eth>,
    flashblocks_state: Arc<FB>,
    metrics: Metrics,
}

impl<Eth: EthApiTypes, FB> EthApiExt<Eth, FB> {
    /// Creates a new extended Eth API instance with flashblocks support.
    pub fn new(eth_api: Eth, eth_filter: EthFilter<Eth>, flashblocks_state: Arc<FB>) -> Self {
        Self { eth_api, eth_filter, flashblocks_state, metrics: Metrics::default() }
    }
}

#[async_trait]
impl<Eth, FB> EthApiOverrideServer for EthApiExt<Eth, FB>
where
    Eth: FullEthApi<NetworkTypes = Ethereum> + Send + Sync + 'static,
    FB: FlashblocksAPI + Send + Sync + 'static,
    jsonrpsee_types::error::ErrorObject<'static>: From<Eth::Error>,
{
    async fn block_by_number(
        &self,
        number: BlockNumberOrTag,
        full: bool,
    ) -> RpcResult<Option<RpcBlock<Ethereum>>> {
        debug!(
            message = "rpc::block_by_number",
            block_number = ?number
        );

        // Without a snapshot, `pending` resolves through the node's `eth` API: the block the
        // engine has executed but not yet made canonical, else the latest block.
        if number.is_pending() {
            self.metrics.rpc_get_block_by_number.increment(1);
            let pending_blocks = self.flashblocks_state.get_pending_blocks();
            if pending_blocks.as_ref().is_some() {
                return Ok(pending_blocks.get_block(full));
            }
        }

        EthBlocks::rpc_block(&self.eth_api, number.into(), full).await.map_err(Into::into)
    }

    async fn get_transaction_receipt(
        &self,
        tx_hash: TxHash,
    ) -> RpcResult<Option<RpcReceipt<Ethereum>>> {
        debug!(
            message = "rpc::get_transaction_receipt",
            tx_hash = %tx_hash
        );

        // Check canonical chain first to avoid race condition where flashblocks
        // state hasn't been cleared yet after canonical block commit
        if let Some(canonical_receipt) =
            EthTransactions::transaction_receipt(&self.eth_api, tx_hash).await?
        {
            return Ok(Some(canonical_receipt));
        }

        // Fall back to flashblocks for pending transactions
        let pending_blocks = self.flashblocks_state.get_pending_blocks();
        if let Some(fb_receipt) = pending_blocks.get_transaction_receipt(tx_hash) {
            self.metrics.rpc_get_transaction_receipt.increment(1);
            return Ok(Some(fb_receipt));
        }

        Ok(None)
    }

    async fn get_balance(
        &self,
        address: Address,
        block_number: Option<BlockId>,
    ) -> RpcResult<U256> {
        debug!(
            message = "rpc::get_balance",
            address = %address
        );
        let block_id = block_number.unwrap_or_default();
        if block_id.is_pending() {
            self.metrics.rpc_get_balance.increment(1);
            let pending_blocks = self.flashblocks_state.get_pending_blocks();
            if let Some(balance) = pending_blocks.get_balance(address) {
                return Ok(balance);
            }
        }

        EthState::balance(&self.eth_api, address, block_number).await.map_err(Into::into)
    }

    async fn get_transaction_count(
        &self,
        address: Address,
        block_number: Option<BlockId>,
    ) -> RpcResult<U256> {
        debug!(
            message = "rpc::get_transaction_count",
            address = %address,
        );

        let block_id = block_number.unwrap_or_default();
        if block_id.is_pending() {
            self.metrics.rpc_get_transaction_count.increment(1);
            // The executed nonce, which counts EIP-7702 authorizations and contract creations as
            // well as sent transactions. reth's own `pending` would add this node's pool.
            return LoadState::spawn_blocking_io_with_state(
                &self.eth_api,
                block_id,
                move |_, state| {
                    let nonce = state.account_nonce(&address).map_err(Eth::Error::from_eth_err)?;
                    Ok(U256::from(nonce.unwrap_or_default()))
                },
            )
            .await
            .map_err(Into::into);
        }

        EthState::transaction_count(&self.eth_api, address, block_number).await.map_err(Into::into)
    }

    async fn transaction_by_hash(
        &self,
        tx_hash: TxHash,
    ) -> RpcResult<Option<RpcTransaction<Ethereum>>> {
        debug!(
            message = "rpc::transaction_by_hash",
            tx_hash = %tx_hash
        );

        // Check canonical chain first to avoid race condition where flashblocks
        // state hasn't been cleared yet after canonical block commit
        if let Some(canonical_tx) = EthTransactions::transaction_by_hash(&self.eth_api, tx_hash)
            .await?
            .map(|tx| tx.into_transaction(self.eth_api.converter()))
            .transpose()
            .map_err(Eth::Error::from)?
        {
            return Ok(Some(canonical_tx));
        }

        // Fall back to flashblocks for pending transactions
        let pending_blocks = self.flashblocks_state.get_pending_blocks();
        if let Some(fb_transaction) = pending_blocks.get_transaction_by_hash(tx_hash) {
            self.metrics.rpc_get_transaction_by_hash.increment(1);
            return Ok(Some(fb_transaction));
        }

        Ok(None)
    }

    async fn send_raw_transaction_sync(
        &self,
        transaction: alloy_primitives::Bytes,
        timeout_ms: Option<u64>,
    ) -> RpcResult<RpcReceipt<Ethereum>> {
        debug!(message = "rpc::send_raw_transaction_sync");

        // A positive timeout shortens the configured one and never extends it. Zero or none is
        // the configured value, `--rpc.send-raw-transaction-sync-timeout`.
        let configured = EthTransactions::send_raw_transaction_sync_timeout(&self.eth_api);
        let timeout = timeout_ms
            .filter(|timeout_ms| *timeout_ms > 0)
            .map(Duration::from_millis)
            .map(|timeout| timeout.min(configured))
            .unwrap_or(configured);

        // Both receivers are taken before submission, so a snapshot or a canonical block that
        // lands while the pool accepts the transaction is not missed.
        let flashblocks = self.flashblocks_state.subscribe_to_flashblocks();
        let canonical = self.eth_api.provider().subscribe_to_canonical_state();

        let tx_hash = match EthTransactions::send_raw_transaction(&self.eth_api, transaction).await
        {
            Ok(hash) => hash,
            Err(e) => return Err(e.into()),
        };

        debug!(
            message = "rpc::send_raw_transaction_sync::sent_transaction",
            tx_hash = %tx_hash,
            timeout = ?timeout,
        );

        // The builder may hold the transaction already. Its receipt is then in the current
        // snapshot, and the wait below would end only with the next broadcast.
        if let Some(receipt) =
            self.flashblocks_state.get_pending_blocks().get_transaction_receipt(tx_hash)
        {
            debug!(message = "found receipt in the current snapshot", tx_hash = %tx_hash);
            self.metrics.rpc_send_raw_transaction_sync_flashblock.increment(1);
            return Ok(receipt);
        }

        tokio::select! {
            receipt = self.wait_for_flashblocks_receipt(flashblocks, tx_hash) => {
                let receipt = receipt.ok_or_else(|| self.confirmation_timeout(tx_hash, timeout))?;
                self.metrics.rpc_send_raw_transaction_sync_flashblock.increment(1);
                Ok(receipt)
            }
            receipt = self.wait_for_canonical_receipt(canonical, tx_hash) => {
                let receipt = receipt.ok_or_else(|| self.confirmation_timeout(tx_hash, timeout))?;
                self.metrics.rpc_send_raw_transaction_sync_canonical.increment(1);
                Ok(receipt)
            }
            _ = time::sleep(timeout) => Err(self.confirmation_timeout(tx_hash, timeout)),
        }
    }

    async fn call(
        &self,
        transaction: TransactionRequest,
        block_number: Option<BlockId>,
        state_overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<alloy_primitives::Bytes> {
        debug!(
            message = "rpc::call",
            transaction = ?transaction,
            block_number = ?block_number,
            state_overrides = ?state_overrides,
            block_overrides = ?block_overrides,
        );

        // `pending` resolves through the node's `eth` API: the snapshot's block environment on
        // the overlay while one is served, reth's own pending otherwise.
        let block_id = block_number.unwrap_or_default();
        if block_id.is_pending() {
            self.metrics.rpc_call.increment(1);
        }

        EthCall::call(
            &self.eth_api,
            transaction,
            Some(block_id),
            EvmOverrides::new(state_overrides, block_overrides),
        )
        .await
        .map_err(Into::into)
    }

    async fn estimate_gas(
        &self,
        transaction: TransactionRequest,
        block_number: Option<BlockId>,
        overrides: Option<StateOverride>,
        block_overrides: Option<Box<BlockOverrides>>,
    ) -> RpcResult<U256> {
        debug!(
            message = "rpc::estimate_gas",
            transaction = ?transaction,
            block_number = ?block_number,
            overrides = ?overrides,
            block_overrides = ?block_overrides,
        );

        let block_id = block_number.unwrap_or_default();
        if block_id.is_pending() {
            self.metrics.rpc_estimate_gas.increment(1);
        }

        EthCall::estimate_gas_at(
            &self.eth_api,
            transaction,
            block_id,
            EvmOverrides::new(overrides, block_overrides),
        )
        .await
        .map_err(Into::into)
    }

    async fn simulate_v1(
        &self,
        opts: SimulatePayload<TransactionRequest>,
        block_number: Option<BlockId>,
    ) -> RpcResult<Vec<SimulatedBlock<RpcBlock<Eth::NetworkTypes>>>> {
        debug!(
            message = "rpc::simulate_v1",
            block_number = ?block_number,
        );

        // `pending` builds the simulated blocks on the canonical tip, on the overlay's state.
        let block_id = block_number.unwrap_or_default();
        if block_id.is_pending() {
            self.metrics.rpc_simulate_v1.increment(1);
        }

        EthCall::simulate_v1(&self.eth_api, opts, Some(block_id)).await.map_err(Into::into)
    }

    async fn get_logs(&self, filter: Filter) -> RpcResult<Vec<Log>> {
        debug!(
            message = "rpc::get_logs",
            address = ?filter.address
        );

        // Check if this is a mixed query (toBlock is pending)
        let (from_block, to_block) = match &filter.block_option {
            alloy_rpc_types_eth::FilterBlockOption::Range { from_block, to_block } => {
                (*from_block, *to_block)
            }
            _ => {
                // Block hash queries or other formats - delegate to eth API
                return self.eth_filter.logs(filter).await;
            }
        };

        // If toBlock is not pending, delegate to eth API
        if !matches!(to_block, Some(BlockNumberOrTag::Pending)) {
            return self.eth_filter.logs(filter).await;
        }

        // Mixed query: toBlock is pending, so we need to combine historical + pending logs
        self.metrics.rpc_get_logs.increment(1);
        let mut all_logs = Vec::new();

        let pending_blocks = self.flashblocks_state.get_pending_blocks();
        if pending_blocks.is_none() {
            return self.eth_filter.logs(filter).await;
        }

        let mut fetched_logs = HashSet::new();
        // Get historical logs if fromBlock is not pending
        if !matches!(from_block, Some(BlockNumberOrTag::Pending)) {
            // Create a filter for historical data (fromBlock to latest)
            let mut historical_filter = filter.clone();
            historical_filter.block_option = alloy_rpc_types_eth::FilterBlockOption::Range {
                from_block,
                to_block: Some(BlockNumberOrTag::Latest),
            };

            let historical_logs: Vec<Log> = self.eth_filter.logs(historical_filter).await?;
            for log in &historical_logs {
                fetched_logs.insert((log.block_number, log.log_index));
            }
            all_logs.extend(historical_logs);
        }

        // Always get pending logs when toBlock is pending
        let pending_logs = pending_blocks.get_pending_logs(&filter);

        // Dedup any logs from the pending state that may already have been covered in the
        // historical logs
        let deduped_pending_logs: Vec<Log> = pending_logs
            .iter()
            .filter(|log| !fetched_logs.contains(&(log.block_number, log.log_index)))
            .cloned()
            .collect();
        all_logs.extend(deduped_pending_logs);

        Ok(all_logs)
    }

    async fn get_block_transaction_count_by_number(
        &self,
        number: BlockNumberOrTag,
    ) -> RpcResult<Option<U256>> {
        debug!(
            message = "rpc::get_block_transaction_count_by_number",
            block_number = ?number
        );

        if number.is_pending() {
            self.metrics.rpc_get_block_transaction_count_by_number.increment(1);
            let pending_blocks = self.flashblocks_state.get_pending_blocks();
            if let Some(block) = pending_blocks.get_block(false) {
                let count = block.transactions.len();
                return Ok(Some(U256::from(count)));
            }
        }

        EthBlocks::block_transaction_count(&self.eth_api, number.into())
            .await
            .map(|opt| opt.map(U256::from))
            .map_err(Into::into)
    }

    async fn block_receipts(
        &self,
        block_id: BlockId,
    ) -> RpcResult<Option<Vec<RpcReceipt<Ethereum>>>> {
        debug!(message = "rpc::block_receipts", block_id = ?block_id);

        if block_id.is_pending() {
            self.metrics.rpc_get_block_receipts.increment(1);
            if let Some(receipts) = self.flashblocks_state.get_pending_blocks().get_block_receipts()
            {
                return Ok(Some(receipts));
            }
        }

        EthBlocks::block_receipts(&self.eth_api, block_id).await.map_err(Into::into)
    }

    async fn transaction_by_block_number_and_index(
        &self,
        number: BlockNumberOrTag,
        index: Index,
    ) -> RpcResult<Option<RpcTransaction<Ethereum>>> {
        debug!(
            message = "rpc::transaction_by_block_number_and_index",
            block_number = ?number,
            index = ?index,
        );

        if number.is_pending() {
            self.metrics.rpc_get_transaction_by_block_number_and_index.increment(1);
            let pending_blocks = self.flashblocks_state.get_pending_blocks();
            if pending_blocks.as_ref().is_some() {
                return Ok(pending_blocks.get_transaction_by_index(index.into()));
            }
        }

        EthTransactions::transaction_by_block_and_tx_index(
            &self.eth_api,
            number.into(),
            index.into(),
        )
        .await
        .map_err(Into::into)
    }
}

impl<Eth, FB> EthApiExt<Eth, FB>
where
    Eth: FullEthApi<NetworkTypes = Ethereum> + Send + Sync + 'static,
    FB: FlashblocksAPI + Send + Sync + 'static,
{
    /// Counts a wait that ended without a receipt, and builds its error.
    fn confirmation_timeout(&self, tx_hash: TxHash, timeout: Duration) -> ErrorObjectOwned {
        self.metrics.rpc_send_raw_transaction_sync_timeout.increment(1);
        EthApiError::TransactionConfirmationTimeout { hash: tx_hash, duration: timeout }.into()
    }

    async fn wait_for_flashblocks_receipt(
        &self,
        mut receiver: broadcast::Receiver<Arc<PendingBlocks>>,
        tx_hash: TxHash,
    ) -> Option<RpcReceipt<Ethereum>> {
        loop {
            match receiver.recv().await {
                Ok(pending_state) if pending_state.get_receipt(tx_hash).is_some() => {
                    debug!(message = "found receipt in flashblock", tx_hash = %tx_hash);
                    return pending_state.get_receipt(tx_hash).cloned();
                }
                Ok(_) => {
                    trace!(message = "flashblock does not contain receipt", tx_hash = %tx_hash);
                }
                Err(RecvError::Closed) => {
                    debug!(message = "flashblocks receipt queue closed");
                    return None;
                }
                Err(RecvError::Lagged(_)) => {
                    warn!("Flashblocks receipt queue lagged, maybe missing receipts");
                }
            }
        }
    }

    async fn wait_for_canonical_receipt<N: NodePrimitives>(
        &self,
        receiver: CanonStateNotifications<N>,
        tx_hash: TxHash,
    ) -> Option<RpcReceipt<Ethereum>> {
        let mut stream = BroadcastStream::new(receiver);

        while let Some(result) = stream.next().await {
            let canon_state = match result {
                Ok(canon_state) => canon_state,
                // The receipt may have been in a skipped notification; the caller still times out.
                Err(BroadcastStreamRecvError::Lagged(skipped)) => {
                    warn!(
                        message = "canonical state subscription lagged while awaiting receipt",
                        tx_hash = %tx_hash,
                        skipped_notifications = skipped,
                    );
                    continue;
                }
            };

            for (block_receipt, _) in canon_state.block_receipts() {
                for (canonical_tx_hash, _) in &block_receipt.tx_receipts {
                    if *canonical_tx_hash == tx_hash {
                        debug!(
                            message = "found receipt in canonical state",
                            tx_hash = %tx_hash
                        );
                        return EthTransactions::transaction_receipt(&self.eth_api, tx_hash)
                            .await
                            .ok()
                            .flatten();
                    }
                }
            }
        }
        None
    }
}
