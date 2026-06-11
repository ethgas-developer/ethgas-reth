//! Flashblocks state processor.

use std::{collections::BTreeMap, sync::Arc};

use alloy_consensus::{
    Header, TxEnvelope, TxReceipt,
    transaction::{Recovered, SignerRecoverable, TransactionMeta},
};
use alloy_eips::BlockNumberOrTag;
use alloy_network::TransactionResponse;
use alloy_primitives::{B256, BlockNumber, map::foldhash::HashMap};
use alloy_rpc_types::{TransactionTrait, state::StateOverride};
use alloy_rpc_types_eth::Log;
use arc_swap::ArcSwapOption;
use alloy_hardforks::EthereumHardforks;
use reth_chainspec::{ChainSpec, ChainSpecProvider, EthChainSpec};
use reth_ethereum_primitives::Block;
use reth_evm::{ConfigureEvm, Evm, NextBlockEnvAttributes, block::SystemCaller};
use reth_evm_ethereum::EthEvmConfig;
use reth_provider::{BlockReaderIdExt, StateProviderFactory};
use reth_revm::{
    DatabaseCommit, State, context::result::ResultAndState, database::StateProviderDatabase,
};
use reth_rpc_eth_types::receipt::build_receipt;
use reth_primitives_traits::RecoveredBlock;
use reth_revm::db::states::bundle_state::BundleRetention;
use reth_rpc_convert::transaction::ConvertReceiptInput;
use tokio::sync::{Mutex, broadcast::Sender, mpsc::UnboundedReceiver};
use tracing::{debug, error, warn};

use crate::{
    block_assembler::BlockAssembler,
    cache::FlashblockCache,
    error::{ProviderError, StateProcessorError},
    payload::FlashBlock,
    pending_blocks::{PendingBlocks, PendingBlocksBuilder},
    validation::{
        CanonicalBlockReconciler, FlashblockSequenceValidator, ReconciliationStrategy,
        ReorgDetector, SequenceValidationResult,
    },
};

/// Messages consumed by the state processor.
#[derive(Debug, Clone)]
pub enum StateUpdate {
    /// New canonical block to reconcile against pending state.
    Canonical(RecoveredBlock<Block>),
    /// Incoming flashblock payload to extend pending state.
    Flashblock(FlashBlock),
}

/// Processes flashblocks and canonical blocks to keep pending state updated.
#[derive(Debug, Clone)]
pub struct StateProcessor<Client> {
    rx: Arc<Mutex<UnboundedReceiver<StateUpdate>>>,
    pending_blocks: Arc<ArcSwapOption<PendingBlocks>>,
    max_depth: u64,
    client: Client,
    chain_spec: Arc<ChainSpec>,
    sender: Sender<Arc<PendingBlocks>>,
    cache: Arc<Mutex<FlashblockCache>>,
}

impl<Client> StateProcessor<Client>
where
    Client: StateProviderFactory
        + ChainSpecProvider<ChainSpec: EthChainSpec<Header = Header> + EthereumHardforks>
        + BlockReaderIdExt<Header = Header>
        + Clone
        + 'static,
{
    /// Creates a new state processor wired to the provided channels and state.
    pub fn new(
        client: Client,
        pending_blocks: Arc<ArcSwapOption<PendingBlocks>>,
        max_depth: u64,
        rx: Arc<Mutex<UnboundedReceiver<StateUpdate>>>,
        chain_spec: Arc<ChainSpec>,
        sender: Sender<Arc<PendingBlocks>>,
    ) -> Self {
        let cache = client
            .best_block_number()
            .map_or_else(|_| FlashblockCache::new(0), FlashblockCache::new);

        Self {
            pending_blocks,
            client,
            max_depth,
            rx,
            chain_spec,
            sender,
            cache: Arc::new(Mutex::new(cache)),
        }
    }

    /// Processes updates from the queue until the channel closes.
    pub async fn start(&self) {
        while let Some(update) = self.rx.lock().await.recv().await {
            let prev_pending_blocks = self.pending_blocks.load_full();

            match update {
                StateUpdate::Canonical(block) => {
                    debug!(message = "processing canonical block", block_number = block.number);
                    match self.process_canonical_block(prev_pending_blocks, &block) {
                        Ok(new_pending_blocks) => {
                            self.pending_blocks.swap(new_pending_blocks);

                            let mut cache = self.cache.lock().await;
                            cache.update_canonical(block.number);
                            let cached = cache.drain(block.number + 1);
                            drop(cache);

                            if !cached.is_empty() {
                                debug!(
                                    message = "replaying cached flashblocks after canonical block",
                                    canonical_block = block.number,
                                    cached_count = cached.len(),
                                );
                                for flashblock in cached {
                                    let fb_prev = self.pending_blocks.load_full();
                                    self.apply_flashblock(fb_prev, flashblock).await;
                                }
                            }
                        }
                        Err(e) => {
                            error!(message = "could not process canonical block", error = %e);
                        }
                    }
                }
                StateUpdate::Flashblock(flashblock) => {
                    debug!(
                        message = "processing flashblock",
                        block_number = flashblock.metadata.block_number,
                        flashblock_index = flashblock.index
                    );
                    self.apply_flashblock(prev_pending_blocks, flashblock).await;
                }
            }
        }
    }

    async fn apply_flashblock(
        &self,
        prev_pending_blocks: Option<Arc<PendingBlocks>>,
        flashblock: FlashBlock,
    ) {
        match self.process_flashblock(prev_pending_blocks, &flashblock) {
            Ok(new_pending_blocks) => {
                if let Some(ref pb) = new_pending_blocks {
                    _ = self.sender.send(Arc::clone(pb));
                }
                self.pending_blocks.swap(new_pending_blocks);
            }
            Err(e) => {
                match e {
                    StateProcessorError::Provider(ProviderError::MissingCanonicalHeader {
                        ..
                    }) => {
                        if self.cache.lock().await.insert(flashblock) {
                            debug!(message = "cached flashblock pending canonical block", error = %e);
                            return;
                        }
                    }
                    StateProcessorError::MissingFirstFlashblock => {
                        let mut cache = self.cache.lock().await;
                        if flashblock.index > 0
                            && cache.has_flashblock(
                                flashblock.metadata.block_number,
                                flashblock.index - 1,
                            )
                            && cache.insert(flashblock)
                        {
                            return;
                        }
                        return;
                    }
                    _ => {}
                }

                if !matches!(
                    e,
                    StateProcessorError::Provider(ProviderError::MissingCanonicalHeader { .. })
                ) {
                    error!(message = "could not process Flashblock", error = %e);
                }
            }
        }
    }

    fn process_canonical_block(
        &self,
        prev_pending_blocks: Option<Arc<PendingBlocks>>,
        block: &RecoveredBlock<Block>,
    ) -> crate::error::Result<Option<Arc<PendingBlocks>>> {
        let pending_blocks = match &prev_pending_blocks {
            Some(pb) => pb,
            None => {
                debug!(message = "no pending state to update with canonical block, skipping");
                return Ok(None);
            }
        };

        let mut flashblocks = pending_blocks.get_flashblocks();

        // Check for reorg by comparing transaction sets
        let tracked_txn_hashes: Vec<_> = pending_blocks
            .get_transactions_for_block(block.number)
            .map(|tx| tx.tx_hash())
            .collect();
        let block_txn_hashes: Vec<B256> =
            block.body().transactions().map(|tx| *tx.tx_hash()).collect();

        let reorg_result = ReorgDetector::detect(&tracked_txn_hashes, &block_txn_hashes);
        let reorg_detected = reorg_result.is_reorg();

        // Determine the reconciliation strategy
        let strategy = CanonicalBlockReconciler::reconcile(
            Some(pending_blocks.earliest_block_number()),
            Some(pending_blocks.latest_block_number()),
            block.number,
            self.max_depth,
            reorg_detected,
        );

        match strategy {
            ReconciliationStrategy::CatchUp => {
                debug!(
                    message = "pending snapshot cleared because canonical caught up",
                    latest_pending_block = pending_blocks.latest_block_number(),
                    canonical_block = block.number,
                );
                Ok(None)
            }
            ReconciliationStrategy::HandleReorg => {
                warn!(
                    message = "reorg detected, recomputing pending flashblocks going ahead of reorg",
                    tracked_txn_hashes = ?tracked_txn_hashes,
                    block_txn_hashes = ?block_txn_hashes,
                );

                // If there is a reorg, we re-process all future flashblocks without reusing the existing pending state
                flashblocks.retain(|flashblock| flashblock.metadata.block_number > block.number);
                self.build_pending_state(None, &flashblocks)
            }
            ReconciliationStrategy::DepthLimitExceeded { depth, max_depth } => {
                debug!(
                    message = "pending blocks depth exceeds max depth, resetting pending blocks",
                    pending_blocks_depth = depth,
                    max_depth = max_depth,
                );

                flashblocks.retain(|flashblock| flashblock.metadata.block_number > block.number);
                self.build_pending_state(None, &flashblocks)
            }
            ReconciliationStrategy::Continue => {
                debug!(
                    message = "canonical block behind latest pending block, continuing with existing pending state",
                    latest_pending_block = pending_blocks.latest_block_number(),
                    earliest_pending_block = pending_blocks.earliest_block_number(),
                    canonical_block = block.number,
                    pending_txns_for_block = ?tracked_txn_hashes.len(),
                    canonical_txns_for_block = ?block_txn_hashes.len(),
                );
                // If no reorg, we can continue building on top of the existing pending state
                // NOTE: We do not retain specific flashblocks here to avoid losing track of our "earliest" pending block number
                self.build_pending_state(prev_pending_blocks, &flashblocks)
            }
            ReconciliationStrategy::NoPendingState => {
                // This case is already handled above, but included for completeness
                debug!(message = "no pending state to update with canonical block, skipping");
                Ok(None)
            }
        }
    }

    fn process_flashblock(
        &self,
        prev_pending_blocks: Option<Arc<PendingBlocks>>,
        flashblock: &FlashBlock,
    ) -> crate::error::Result<Option<Arc<PendingBlocks>>> {
        let pending_blocks = match &prev_pending_blocks {
            Some(pb) => pb,
            None => {
                if flashblock.index == 0 {
                    return self.build_pending_state(None, &vec![flashblock.clone()]);
                }

                return Err(StateProcessorError::MissingFirstFlashblock);
            }
        };

        let validation_result = FlashblockSequenceValidator::validate(
            pending_blocks.latest_block_number(),
            pending_blocks.latest_flashblock_index(),
            flashblock.metadata.block_number,
            flashblock.index,
        );

        match validation_result {
            SequenceValidationResult::NextInSequence
            | SequenceValidationResult::FirstOfNextBlock => {
                // We have received the next flashblock for the current block
                // or the first flashblock for the next block
                let mut flashblocks = pending_blocks.get_flashblocks();
                flashblocks.push(flashblock.clone());
                self.build_pending_state(prev_pending_blocks, &flashblocks)
            }
            SequenceValidationResult::Duplicate => {
                // We have received a duplicate flashblock for the current block

                warn!(
                    message = "Received duplicate Flashblock for current block, ignoring",
                    curr_block = %pending_blocks.latest_block_number(),
                    flashblock_index = %flashblock.index,
                );
                Ok(prev_pending_blocks)
            }
            SequenceValidationResult::InvalidNewBlockIndex { block_number, index: _ } => {
                // We have received a non-zero flashblock for a new block

                error!(
                    message = "Received non-zero index Flashblock for new block, zeroing Flashblocks until we receive a base Flashblock",
                    curr_block = %pending_blocks.latest_block_number(),
                    new_block = %block_number,
                );
                Ok(None)
            }
            SequenceValidationResult::NonSequentialGap { expected: _, actual: _ } => {
                // We have received a non-sequential Flashblock for the current block

                error!(
                    message = "Received non-sequential Flashblock for current block, zeroing Flashblocks until we receive a base Flashblock",
                    curr_block = %pending_blocks.latest_block_number(),
                    new_block = %flashblock.metadata.block_number,
                );
                Ok(None)
            }
        }
    }

    fn build_pending_state(
        &self,
        prev_pending_blocks: Option<Arc<PendingBlocks>>,
        flashblocks: &Vec<FlashBlock>,
    ) -> crate::error::Result<Option<Arc<PendingBlocks>>> {
        let mut flashblocks_per_block = BTreeMap::<BlockNumber, Vec<FlashBlock>>::new();
        for flashblock in flashblocks {
            flashblocks_per_block
                .entry(flashblock.metadata.block_number)
                .or_default()
                .push(flashblock.clone());
        }

        let earliest_block_number = flashblocks_per_block.keys().min().unwrap();
        let canonical_block = earliest_block_number - 1;
        let mut last_block_header = self
            .client
            .header_by_number(canonical_block)
            .map_err(|e| ProviderError::StateProvider(e.to_string()))?
            .ok_or(ProviderError::MissingCanonicalHeader { block_number: canonical_block })?;

        let evm_config = EthEvmConfig::ethereum(self.chain_spec.clone());

        let state_provider = self
            .client
            .state_by_block_number_or_tag(BlockNumberOrTag::Number(canonical_block))
            .map_err(|e| ProviderError::StateProvider(e.to_string()))?;
        let state_provider_db = StateProviderDatabase::new(state_provider);
        let mut pending_blocks_builder = PendingBlocksBuilder::new();

        // Track state changes across flashblocks, accumulating bundle state
        // from previous pending blocks if available.
        let mut db = match &prev_pending_blocks {
            Some(pending_blocks) => State::builder()
                .with_database(state_provider_db)
                .with_bundle_update()
                .with_bundle_prestate(pending_blocks.get_bundle_state())
                .build(),
            None => State::builder().with_database(state_provider_db).with_bundle_update().build(),
        };

        let mut state_overrides =
            prev_pending_blocks.as_ref().map_or_else(StateOverride::default, |pending_blocks| {
                pending_blocks.get_state_overrides().unwrap_or_default()
            });

        for (_block_number, flashblocks) in flashblocks_per_block {
            let base = flashblocks
                .first()
                .ok_or(crate::error::ProtocolError::EmptyFlashblocks)?
                .base
                .clone()
                .ok_or(crate::error::ProtocolError::MissingBase)?;

            let receipt_by_hash = flashblocks
                .iter()
                .map(|flashblock| flashblock.metadata.receipts.clone())
                .fold(HashMap::default(), |mut acc, receipts| {
                    acc.extend(receipts);
                    acc
                });

            let updated_balances = flashblocks
                .iter()
                .map(|flashblock| flashblock.metadata.new_account_balances.clone())
                .fold(HashMap::default(), |mut acc, balances| {
                    acc.extend(balances);
                    acc
                });

            let assembled = BlockAssembler::assemble(&flashblocks)?;
            let block = assembled.block;
            let header = assembled.header;

            pending_blocks_builder.with_flashblocks(flashblocks.clone());
            pending_blocks_builder.with_header(header.clone());

            let block_env_attributes = NextBlockEnvAttributes {
                timestamp: base.timestamp,
                suggested_fee_recipient: base.fee_recipient,
                prev_randao: base.prev_randao,
                gas_limit: base.gas_limit,
                parent_beacon_block_root: Some(base.parent_beacon_block_root),
                withdrawals: None,
                extra_data: base.extra_data,
                slot_number: None,
            };

            let evm_env = evm_config
                .next_evm_env(&last_block_header, &block_env_attributes)
                .map_err(|e| crate::error::ExecutionError::EvmEnv(e.to_string()))?;
            let mut evm = evm_config.evm_with_env(db, evm_env);

            // Apply EIP-4788 (beacon root) and EIP-2935 (blockhashes) pre-execution
            // system calls so cached execution matches what the validator computes.
            let parent_hash = last_block_header.hash_slow();
            let mut system_caller = SystemCaller::new(self.chain_spec.clone());
            system_caller
                .apply_blockhashes_contract_call(parent_hash, &mut evm)
                .map_err(|e| crate::error::ExecutionError::EvmEnv(e.to_string()))?;
            system_caller
                .apply_beacon_root_contract_call(Some(base.parent_beacon_block_root), &mut evm)
                .map_err(|e| crate::error::ExecutionError::EvmEnv(e.to_string()))?;

            let mut gas_used = 0;
            let mut next_log_index = 0;

            for (idx, transaction) in block.body.transactions.iter().enumerate() {
                let sender = match prev_pending_blocks
                    .as_ref()
                    .and_then(|p| p.get_transaction_sender(transaction.tx_hash()))
                {
                    Some(cached) => cached,
                    None => transaction.recover_signer()?,
                };
                pending_blocks_builder.increment_nonce(sender);
                pending_blocks_builder.with_transaction_sender(*transaction.tx_hash(), sender);

                let receipt = receipt_by_hash.get(&transaction.tx_hash().clone()).cloned().ok_or(
                    crate::error::ExecutionError::TransactionFailed {
                        tx_hash: *transaction.tx_hash(),
                        sender,
                        reason: "missing receipt".to_string(),
                    },
                )?;

                let recovered_transaction = Recovered::new_unchecked(transaction.clone(), sender);
                let envelope = recovered_transaction.clone().convert::<TxEnvelope>();

                let effective_gas_price = block
                    .base_fee_per_gas
                    .map(|base_fee| {
                        transaction.effective_tip_per_gas(base_fee).unwrap_or_default()
                            + base_fee as u128
                    })
                    .unwrap_or_else(|| transaction.max_fee_per_gas());

                let rpc_txn = alloy_rpc_types::Transaction {
                    inner: envelope,
                    block_hash: Some(header.hash()),
                    block_number: Some(base.block_number),
                    transaction_index: Some(idx as u64),
                    effective_gas_price: Some(effective_gas_price),
                    block_timestamp: Some(base.timestamp),
                };

                pending_blocks_builder.with_transaction(rpc_txn);

                let meta = TransactionMeta {
                    tx_hash: *transaction.tx_hash(),
                    index: idx as u64,
                    block_hash: header.hash(),
                    block_number: block.number,
                    base_fee: block.base_fee_per_gas,
                    excess_blob_gas: block.excess_blob_gas,
                    timestamp: block.timestamp,
                };

                let input: ConvertReceiptInput<'_, reth_ethereum_primitives::EthPrimitives> =
                    ConvertReceiptInput {
                        receipt: receipt.clone(),
                        tx: Recovered::new_unchecked(transaction, sender),
                        gas_used: receipt.cumulative_gas_used() - gas_used,
                        next_log_index,
                        meta,
                    };

                let blob_params =
                    self.client.chain_spec().blob_params_at_timestamp(input.meta.timestamp);
                let eth_receipt =
                    build_receipt(input, blob_params, |receipt, next_log_index, meta| {
                        let mut log_index = next_log_index;
                        receipt
                            .map_logs(|log| {
                                let idx = log_index;
                                log_index += 1;
                                Log {
                                    inner: log,
                                    block_hash: Some(meta.block_hash),
                                    block_number: Some(meta.block_number),
                                    block_timestamp: Some(meta.timestamp),
                                    transaction_hash: Some(meta.tx_hash),
                                    transaction_index: Some(meta.index),
                                    log_index: Some(idx as u64),
                                    removed: false,
                                }
                            })
                            .into()
                    });

                pending_blocks_builder.with_receipt(*transaction.tx_hash(), eth_receipt);

                gas_used = receipt.cumulative_gas_used();
                next_log_index += receipt.logs().len();

                let mut should_execute_transaction = false;
                match &prev_pending_blocks {
                    Some(pending_blocks) => {
                        match pending_blocks.get_transaction_state(transaction.tx_hash()) {
                            Some(state) => {
                                pending_blocks_builder
                                    .with_transaction_state(*transaction.tx_hash(), state);
                            }
                            None => {
                                should_execute_transaction = true;
                            }
                        }
                    }
                    None => {
                        should_execute_transaction = true;
                    }
                }

                if should_execute_transaction {
                    let ResultAndState { state, .. } = evm
                        .transact(recovered_transaction)
                        .map_err(|e| crate::error::ExecutionError::TransactionFailed {
                            tx_hash: *transaction.tx_hash(),
                            sender,
                            reason: e.to_string(),
                        })?;
                    for (addr, acc) in &state {
                        let existing_override = state_overrides.entry(*addr).or_default();
                        existing_override.balance = Some(acc.info.balance);
                        existing_override.nonce = Some(acc.info.nonce);
                        existing_override.code = acc.info.code.clone().map(|code| code.bytes());

                        let existing =
                            existing_override.state_diff.get_or_insert_with(Default::default);
                        let changed_slots = acc
                            .storage
                            .iter()
                            .map(|(&key, slot)| (B256::from(key), B256::from(slot.present_value)));

                        existing.extend(changed_slots);
                    }
                    pending_blocks_builder
                        .with_transaction_state(*transaction.tx_hash(), state.clone());
                    evm.db_mut().commit(state);
                }
            }

            for (address, balance) in updated_balances {
                pending_blocks_builder.with_account_balance(address, balance);
            }

            db = evm.into_db();
            last_block_header = block.header.clone();
        }

        // Extract the accumulated bundle state.
        db.merge_transitions(BundleRetention::Reverts);
        pending_blocks_builder.with_bundle_state(db.take_bundle());
        pending_blocks_builder.with_state_overrides(state_overrides);
        Ok(Some(Arc::new(pending_blocks_builder.build()?)))
    }
}
