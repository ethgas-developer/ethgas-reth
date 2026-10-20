//! The `relay_validateBuilderSubmission*` block validation API.

pub mod error;

use std::{fmt, fmt::Debug, sync::Arc, time::Duration};

use alloy_consensus::{BlockHeader, EnvKzgSettings, Transaction, TxReceipt};
use alloy_eips::{
    eip4844::kzg_to_versioned_hash,
    eip7685::{Requests, RequestsOrHash},
    eip7928::bal::Bal,
};
use alloy_primitives::{Address, B256, Bytes, U256};
use alloy_rpc_types_beacon::relay::{
    BidTrace, BuilderBlockValidationRequest, BuilderBlockValidationRequestV2,
    BuilderBlockValidationRequestV3, BuilderBlockValidationRequestV4,
    BuilderBlockValidationRequestV5, BuilderBlockValidationRequestV6,
};
use alloy_rpc_types_engine::{
    BlobsBundleV1, BlobsBundleV2, CancunPayloadFields, ExecutionData, ExecutionPayload,
    ExecutionPayloadSidecar, PraguePayloadFields,
};
use dashmap::DashSet;
use derive_more::Deref;
use ethgas_node_runner::types::EthProvider;
use jsonrpsee::{
    core::{RpcResult, async_trait},
    proc_macros::rpc,
    types::ErrorObject,
};
use metrics::{Gauge, gauge};
use metrics_derive::Metrics;
use reth_chainspec::{ChainSpecProvider, EthChainSpec, EthereumHardforks};
use reth_consensus::ConsensusError;
use reth_consensus_common::validation::MAX_RLP_BLOCK_SIZE;
use reth_ethereum_primitives::{Block, EthPrimitives, Receipt, TransactionSigned};
use reth_evm::{
    ConfigureEvm, Evm,
    execute::{BlockExecutionOutput, Executor},
};
use reth_node_ethereum::EthereumEngineValidator;
use reth_primitives_traits::{
    BlockBody, GotExpected, Recovered, RecoveredBlock, SealedBlock, SealedHeaderFor,
};
use reth_provider::{
    AccountReader, BlockReaderIdExt, HashedPostStateProvider, HeaderProvider, StateProvider,
    StateProviderBox, StateProviderFactory,
};
use reth_revm::{Database, State, cached::CachedReads, database::StateProviderDatabase};
use reth_rpc_eth_types::utils::recover_raw_transaction;
use reth_rpc_server_types::result::internal_rpc_err;
use reth_tasks::TaskExecutor;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use tokio::{
    spawn,
    sync::{RwLock, oneshot},
    time,
};
use tracing::{error, info, warn};

use crate::{
    blacklist::{DisallowListPayload, changed_disallow_hash},
    common::{RethConsensus, RethEvmConfig, RethPayloadValidator},
    payment::{
        PAYMENT_FORWARDER, PAYMENT_FORWARDER_CODE_HASH, multisend_paid_amount,
        payment_forwarder_recipient,
    },
    validation::error::{GetParentError, ValidationApiError},
};

/// Serves the `relay_validateBuilderSubmission*` block validation methods.
#[derive(Clone, Debug, Deref)]
pub struct ValidationApi {
    #[deref]
    inner: Arc<ValidationApiInner>,
}

impl ValidationApi {
    /// Create a new instance of the [`ValidationApi`]
    pub fn new(
        provider: EthProvider,
        consensus: Arc<RethConsensus>,
        evm_config: RethEvmConfig,
        config: ValidationApiConfig,
        task_spawner: TaskExecutor,
        payload_validator: Arc<EthereumEngineValidator>,
    ) -> Self {
        let ValidationApiConfig { blacklist_endpoint, validation_window } = config;
        let disallow = Arc::new(DashSet::new());

        let inner = Arc::new(ValidationApiInner {
            provider,
            consensus,
            payload_validator,
            evm_config,
            disallow: disallow.clone(),
            validation_window,
            cached_state: Default::default(),
            task_spawner,
            metrics: Default::default(),
        });

        inner.metrics.disallow_size.set(inner.disallow.len() as f64);
        inner.metrics.disallow_loaded.set(0.0);

        // Fetches the disallow list at once, then every five minutes.
        let client = reqwest::Client::new();
        let ep = blacklist_endpoint;
        let dash = disallow;
        let gauge = inner.metrics.disallow_size.clone();
        let loaded_gauge = inner.metrics.disallow_loaded.clone();
        spawn(async move {
            let mut interval = time::interval(Duration::from_secs(300));
            // Stays `None` until the first successful fetch, so a restart doesn't publish the
            // hash of the empty list we start out with.
            let mut current_hash: Option<String> = None;
            let mut loaded = false;
            loop {
                interval.tick().await;
                let failure = match client.get(&ep).send().await {
                    Ok(resp) if resp.status().is_success() => {
                        match resp.json::<DisallowListPayload>().await {
                            Ok(payload) => {
                                dash.clear();
                                for addr in payload.into_addresses() {
                                    dash.insert(addr);
                                }
                                gauge.set(dash.len() as f64);
                                loaded = true;
                                loaded_gauge.set(1.0);

                                if let Some(new_hash) =
                                    changed_disallow_hash(&dash, current_hash.as_deref())
                                {
                                    // Setting the value for old hash to zero and the new hash to
                                    // one.
                                    if let Some(prev) = current_hash.take() {
                                        gauge!("builder_validation_disallow_hash", "hash" => prev)
                                            .set(0.0);
                                    }
                                    gauge!("builder_validation_disallow_hash", "hash" => new_hash.clone())
                                        .set(1.0);
                                    info!(hash = %new_hash, size = dash.len(), "disallow list updated");
                                    current_hash = Some(new_hash);
                                }
                                None
                            }
                            Err(e) => Some(format!("could not read the disallow list: {e}")),
                        }
                    }
                    Ok(r) => Some(format!("blacklist fetch failed: HTTP {}", r.status())),
                    Err(e) => Some(format!("blacklist fetch failed: {e}")),
                };

                if let Some(reason) = failure {
                    if loaded {
                        error!(endpoint = %ep, "{reason}");
                    } else {
                        error!(
                            endpoint = %ep,
                            "{reason}; no disallow list is in force, this simulator applies no address filtering"
                        );
                    }
                }
            }
        });

        Self { inner }
    }

    /// Returns the cached reads for the given head hash.
    pub(crate) async fn cached_reads(&self, head: B256) -> CachedReads {
        let cache = self.inner.cached_state.read().await;
        if cache.0 == head { cache.1.clone() } else { Default::default() }
    }

    /// Updates the cached state for the given head hash.
    pub(crate) async fn update_cached_reads(&self, head: B256, cached_state: CachedReads) {
        let mut cache = self.inner.cached_state.write().await;
        if cache.0 == head {
            cache.1.extend(cached_state);
        } else {
            *cache = (head, cached_state)
        }
    }

    /// Runs `validation` on a blocking task and returns its result.
    async fn spawn_validation<F>(&self, validation: F) -> RpcResult<()>
    where
        F: Future<Output = Result<(), ValidationApiError>> + Send + 'static,
    {
        let (tx, rx) = oneshot::channel();
        self.task_spawner.spawn_blocking_task(async move {
            let _ = tx.send(validation.await.map_err(ErrorObject::from));
        });
        rx.await.map_err(|_| internal_rpc_err("Internal blocking task error"))?
    }
}

impl ValidationApi {
    /// Validates the given block and a [`BidTrace`] against it. `base_payment_tx_index` is
    /// `Some` only for the merged-block-only validation endpoint, and switches the payment
    /// check from `ensure_payment` (regular submissions, single trailing tx) to
    /// `ensure_merged_payment` (a merged block's base payment tx plus its distribution tx).
    pub async fn validate_message_against_block(
        &self,
        block: RecoveredBlock<Block>,
        message: BidTrace,
        _registered_gas_limit: u64,
        apply_blacklist: bool,
        inclusion_list: Option<InclusionListWithMetadata>,
        base_payment_tx_index: Option<usize>,
    ) -> Result<(), ValidationApiError> {
        self.validate_message_against_header(block.sealed_header(), &message)?;

        self.consensus.validate_header(block.sealed_header())?;
        self.consensus.validate_block_pre_execution(block.sealed_block())?;

        if !self.disallow.is_empty() && apply_blacklist {
            if self.disallow.contains(&block.beneficiary()) {
                return Err(ValidationApiError::Blacklist(block.beneficiary()));
            }
            if self.disallow.contains(&message.proposer_fee_recipient) {
                return Err(ValidationApiError::Blacklist(message.proposer_fee_recipient));
            }
            for (sender, tx) in block.senders_iter().zip(block.body().transactions()) {
                if self.disallow.contains(sender) {
                    return Err(ValidationApiError::Blacklist(*sender));
                }
                if let Some(to) = tx.to() &&
                    self.disallow.contains(&to)
                {
                    return Err(ValidationApiError::Blacklist(to));
                }
            }
        }

        let parent_header = self.get_parent_header(block.parent_hash())?;

        self.consensus.validate_header_against_parent(block.sealed_header(), &parent_header)?;
        let parent_header_hash = parent_header.hash();

        let request_cache = self.cached_reads(parent_header_hash).await;

        // Runs EVM execution and inclusion-list validation synchronously (no `.await` inside),
        // since the state types involved borrow from a `!Sync` state provider and must not be
        // held across an await point for this function's future to remain `Send`.
        let ExecutedBlock { output, request_cache, state_provider, block_access_list_hash } =
            self.execute_block(&block, apply_blacklist, inclusion_list.as_ref(), request_cache)?;

        self.update_cached_reads(parent_header_hash, request_cache).await;

        self.consensus.validate_block_post_execution(
            &block,
            &output,
            None,
            block_access_list_hash,
        )?;

        match base_payment_tx_index {
            Some(ix) => self.ensure_merged_payment(&block, &output, &message, ix)?,
            None => self.ensure_payment(&block, &output, &message)?,
        }

        let state_root =
            state_provider.state_root(state_provider.hashed_post_state(&output.state)?)?;

        if state_root != block.header().state_root() {
            return Err(ConsensusError::BodyStateRootDiff(
                GotExpected { got: state_root, expected: block.header().state_root() }.into(),
            )
            .into());
        }

        Ok(())
    }

    /// Executes the block against the parent state and validates the inclusion list constraint,
    /// entirely synchronously. Kept free of `.await` because the state types involved borrow
    /// from a `!Sync` state provider and a `!Sync` cached-reads database.
    fn execute_block(
        &self,
        block: &RecoveredBlock<Block>,
        apply_blacklist: bool,
        inclusion_list: Option<&InclusionListWithMetadata>,
        mut request_cache: CachedReads,
    ) -> Result<ExecutedBlock, ValidationApiError> {
        let state_provider = self.provider.state_by_block_hash(block.parent_hash())?;

        let cached_db = request_cache
            .as_db_mut(StateProviderDatabase::new((&state_provider).into_evm_state_provider()));
        let mut executor = self.evm_config.batch_executor(cached_db);

        let mut accessed_blacklisted = None;

        let result = executor.execute_one(block)?;

        // The executor rebuilds the block access list when the header commits to one. The
        // post-execution check rejects a submitted list that differs from it, so the EIP-7928 size
        // limit is checked on the rebuilt list.
        let block_access_list = executor.take_bal().map(Bal::from);
        if let Some(block_access_list) = &block_access_list {
            block_access_list
                .validate_gas_limit(block.gas_limit())
                .map_err(ConsensusError::from)?;
        }
        let block_access_list_hash = block_access_list.map(|list| list.compute_hash());

        let state = executor.into_state();

        if !self.disallow.is_empty() && apply_blacklist {
            // Check whether the submission interacted with any blacklisted account by scanning
            // the `State`'s cache that records everything read from database during execution.
            for account in state.cache.accounts.keys() {
                if self.disallow.contains(account) {
                    accessed_blacklisted = Some(*account);
                }
            }
        }

        if let Some(account) = accessed_blacklisted {
            return Err(ValidationApiError::Blacklist(account));
        }

        let output = BlockExecutionOutput { state: state.bundle_state.clone(), result };

        self.validate_inclusion_list_constraint(block, state, inclusion_list)?;

        Ok(ExecutedBlock { output, request_cache, state_provider, block_access_list_hash })
    }

    /// Fails when the block leaves out a transaction of the inclusion list that would still
    /// succeed on top of the block.
    fn validate_inclusion_list_constraint<DB>(
        &self,
        block: &RecoveredBlock<Block>,
        post_state: State<DB>,
        inclusion_list: Option<&InclusionListWithMetadata>,
    ) -> Result<(), ValidationApiError>
    where
        DB: Database + Debug,
        <DB as Database>::Error: Send + Sync + 'static,
    {
        let Some(inclusion_list) = inclusion_list else {
            return Ok(());
        };
        if inclusion_list.txs.is_empty() {
            return Ok(());
        }

        let mut included_hashes = FxHashSet::default();
        for tx in block.body().transactions() {
            if let Some(req) = inclusion_list.txs.iter().find(|t| t.hash == *tx.tx_hash()) {
                included_hashes.insert(req.hash);
            }
        }

        if included_hashes.len() == inclusion_list.txs.len() {
            return Ok(());
        }

        // set up a fresh EVM on top of a cache wrapping the post-block state
        let mut evm = self
            .evm_config
            .evm_for_block(post_state, block.header())
            .map_err(|_| ValidationApiError::InclusionList)?;

        for req in &inclusion_list.txs {
            if included_hashes.contains(&req.hash) {
                continue;
            }

            let bytes_slice = req.bytes.as_ref();
            let transaction: Recovered<TransactionSigned> = recover_raw_transaction(bytes_slice)
                .map_err(|_| ValidationApiError::InclusionList)?;

            let outcome = evm.transact(transaction);

            // if it succeeded, then this tx *could* have been included but wasn’t → reject
            if outcome.is_ok() {
                return Err(ValidationApiError::InclusionList);
            }
        }

        // every missing tx failed in simulation, so constraint is satisfied
        Ok(())
    }

    /// Ensures that fields of [`BidTrace`] match the fields of the [`SealedHeaderFor`].
    fn validate_message_against_header(
        &self,
        header: &SealedHeaderFor<EthPrimitives>,
        message: &BidTrace,
    ) -> Result<(), ValidationApiError> {
        if header.hash() != message.block_hash {
            error!("Block hash mismatch: {message:?}, {header:?}");
            Err(ValidationApiError::BlockHashMismatch(GotExpected {
                got: message.block_hash,
                expected: header.hash(),
            }))
        } else if header.parent_hash() != message.parent_hash {
            Err(ValidationApiError::ParentHashMismatch(GotExpected {
                got: message.parent_hash,
                expected: header.parent_hash(),
            }))
        } else if header.gas_limit() != message.gas_limit {
            Err(ValidationApiError::GasLimitMismatch(GotExpected {
                got: message.gas_limit,
                expected: header.gas_limit(),
            }))
        } else if header.gas_used() != message.gas_used {
            Err(ValidationApiError::GasUsedMismatch(GotExpected {
                got: message.gas_used,
                expected: header.gas_used(),
            }))
        } else {
            Ok(())
        }
    }

    /// Ensures that the proposer has received [`BidTrace::value`] for this block.
    ///
    /// Firstly attempts to verify the payment by checking the state changes, otherwise falls
    /// back to requiring the last transaction in the block to pay it directly. Shared by every
    /// externally-submitted builder block (v4/v5) -- regular submissions always pay in a single
    /// trailing transaction, so this deliberately doesn't scan the rest of the block. A merged
    /// block's payment can legitimately span two transactions (see helix
    /// `crates/builder/src/engine/session.rs`); that's handled by `ensure_merged_payment`, on the
    /// merged-block-only validation endpoint, not here.
    fn ensure_payment(
        &self,
        block: &SealedBlock<Block>,
        output: &BlockExecutionOutput<Receipt>,
        message: &BidTrace,
    ) -> Result<(), ValidationApiError> {
        let (mut balance_before, balance_after) = if let Some(acc) =
            output.state.state.get(&message.proposer_fee_recipient)
        {
            let balance_before = acc.original_info.as_ref().map(|i| i.balance).unwrap_or_default();
            let balance_after = acc.info.as_ref().map(|i| i.balance).unwrap_or_default();

            (balance_before, balance_after)
        } else {
            // account might have balance but considering it zero is fine as long as we know
            // that balance have not changed
            (U256::ZERO, U256::ZERO)
        };

        if let Some(withdrawals) = block.body().withdrawals() {
            for withdrawal in withdrawals {
                if withdrawal.address == message.proposer_fee_recipient {
                    balance_before += withdrawal.amount_wei();
                }
            }
        }

        if balance_after >= balance_before + message.value {
            return Ok(());
        }

        let (receipt, tx) = output
            .receipts
            .last()
            .zip(block.body().transactions().last())
            .ok_or(ValidationApiError::ProposerPayment)?;

        if !receipt.status() {
            return Err(ValidationApiError::ProposerPayment);
        }

        if tx.chain_id() != Some(self.evm_config.chain_spec().chain().id()) {
            return Err(ValidationApiError::ProposerPayment);
        }

        // Either a direct transfer, or one routed through the PaymentForwarder,
        // which sends the full value on to the recipient in its calldata. The
        // forwarder is only trusted where its runtime is actually deployed,
        // since a value call to a codeless address succeeds and keeps the value.
        let paid_directly =
            tx.to() == Some(message.proposer_fee_recipient) && tx.input().is_empty();
        let paid_via_forwarder = tx.to() == Some(PAYMENT_FORWARDER) &&
            payment_forwarder_recipient(tx.input()) == Some(message.proposer_fee_recipient) &&
            self.provider
                .latest()
                .and_then(|state| state.basic_account(&PAYMENT_FORWARDER))
                .is_ok_and(|account| {
                    account.is_some_and(|account| {
                        account.bytecode_hash == Some(PAYMENT_FORWARDER_CODE_HASH)
                    })
                });
        if !paid_directly && !paid_via_forwarder {
            return Err(ValidationApiError::ProposerPayment);
        }

        if tx.value() != message.value {
            return Err(ValidationApiError::ProposerPayment);
        }

        if let Some(block_base_fee) = block.header().base_fee_per_gas() &&
            tx.effective_tip_per_gas(block_base_fee).unwrap_or_default() != 0
        {
            return Err(ValidationApiError::ProposerPayment);
        }

        Ok(())
    }

    /// Merged-block counterpart of `ensure_payment`, for the merged-block-only validation
    /// endpoint. A merged block's payment is legitimately split across exactly two
    /// transactions: the base block's own payment tx (kept at `base_payment_tx_index`, paying
    /// `base_value`) and a newly appended distribution tx at the very end of the block (paying
    /// the incremental `proposer_added_value`) -- see helix
    /// `crates/builder/src/engine/session.rs`. Checking only these two positions (as opposed to
    /// `ensure_payment`'s regular-submission single-last-tx check, or scanning every tx in the
    /// block) is safe here because `base_payment_tx_index` is derived and trusted upstream, at
    /// the relay (`BlockMergeResponse::base_payment_tx_index`) -- if it's wrong, this fails
    /// closed (the payment isn't found), it never lets an underpaid block through.
    fn ensure_merged_payment(
        &self,
        block: &SealedBlock<Block>,
        output: &BlockExecutionOutput<Receipt>,
        message: &BidTrace,
        base_payment_tx_index: usize,
    ) -> Result<(), ValidationApiError> {
        let (mut balance_before, balance_after) = if let Some(acc) =
            output.state.state.get(&message.proposer_fee_recipient)
        {
            let balance_before = acc.original_info.as_ref().map(|i| i.balance).unwrap_or_default();
            let balance_after = acc.info.as_ref().map(|i| i.balance).unwrap_or_default();

            (balance_before, balance_after)
        } else {
            (U256::ZERO, U256::ZERO)
        };

        if let Some(withdrawals) = block.body().withdrawals() {
            for withdrawal in withdrawals {
                if withdrawal.address == message.proposer_fee_recipient {
                    balance_before += withdrawal.amount_wei();
                }
            }
        }

        if balance_after >= balance_before + message.value {
            return Ok(());
        }

        let Some(last_ix) = block.body().transactions().count().checked_sub(1) else {
            return Err(ValidationApiError::ProposerPayment);
        };

        let mut total =
            self.recognized_payment_at(block, output, message.proposer_fee_recipient, last_ix);
        if base_payment_tx_index != last_ix {
            total += self.recognized_payment_at(
                block,
                output,
                message.proposer_fee_recipient,
                base_payment_tx_index,
            );
        }

        if total >= message.value {
            return Ok(());
        }

        Err(ValidationApiError::ProposerPayment)
    }

    /// The recognized payment to `recipient` from the transaction at `ix`: a direct transfer,
    /// one routed through the [`PAYMENT_FORWARDER`], or a batched entry in a Gnosis Safe
    /// `execTransaction` -> `multiSend` delegatecall (the merge-builder payment engine's
    /// mechanism, see helix `crates/builder/src/engine/payment.rs`). Zero for an out-of-range
    /// index, an unsuccessful receipt, or a transaction that fails the same anti-MEV-extraction
    /// checks (chain id, zero priority fee) the payment-recognition checks above apply -- never
    /// an error, since a single unrecognized/invalid position shouldn't itself fail validation.
    fn recognized_payment_at(
        &self,
        block: &SealedBlock<Block>,
        output: &BlockExecutionOutput<Receipt>,
        recipient: Address,
        ix: usize,
    ) -> U256 {
        let Some((receipt, tx)) = output.receipts.get(ix).zip(block.body().transactions().nth(ix))
        else {
            return U256::ZERO;
        };
        if !receipt.status() {
            return U256::ZERO;
        }

        let paid_directly = tx.to() == Some(recipient) && tx.input().is_empty();
        let paid_via_forwarder = tx.to() == Some(PAYMENT_FORWARDER) &&
            payment_forwarder_recipient(tx.input()) == Some(recipient) &&
            self.provider
                .latest()
                .and_then(|state| state.basic_account(&PAYMENT_FORWARDER))
                .is_ok_and(|account| {
                    account.is_some_and(|account| {
                        account.bytecode_hash == Some(PAYMENT_FORWARDER_CODE_HASH)
                    })
                });
        let contributed = if paid_directly || paid_via_forwarder {
            tx.value()
        } else {
            multisend_paid_amount(tx.input(), recipient)
        };
        if contributed.is_zero() {
            return U256::ZERO;
        }

        if tx.chain_id() != Some(self.evm_config.chain_spec().chain().id()) {
            return U256::ZERO;
        }
        if let Some(block_base_fee) = block.header().base_fee_per_gas() &&
            tx.effective_tip_per_gas(block_base_fee).unwrap_or_default() != 0
        {
            return U256::ZERO;
        }

        contributed
    }

    /// Validates the given [`BlobsBundleV1`] and returns versioned hashes for blobs.
    pub fn validate_blobs_bundle(
        &self,
        mut blobs_bundle: BlobsBundleV1,
    ) -> Result<Vec<B256>, ValidationApiError> {
        if blobs_bundle.commitments.len() != blobs_bundle.proofs.len() ||
            blobs_bundle.commitments.len() != blobs_bundle.blobs.len()
        {
            return Err(ValidationApiError::InvalidBlobsBundle);
        }

        let versioned_hashes = blobs_bundle
            .commitments
            .iter()
            .map(|c| kzg_to_versioned_hash(c.as_slice()))
            .collect::<Vec<_>>();

        let sidecar = blobs_bundle.pop_sidecar(blobs_bundle.blobs.len());

        sidecar.validate(&versioned_hashes, EnvKzgSettings::default().get())?;

        Ok(versioned_hashes)
    }

    /// Validates the given [`BlobsBundleV2`] and returns versioned hashes for blobs.
    pub fn validate_blobs_bundle_v2(
        &self,
        blobs_bundle: BlobsBundleV2,
    ) -> Result<Vec<B256>, ValidationApiError> {
        let versioned_hashes = blobs_bundle
            .commitments
            .iter()
            .map(|c| kzg_to_versioned_hash(c.as_slice()))
            .collect::<Vec<_>>();

        blobs_bundle
            .try_into_sidecar()
            .map_err(|_| ValidationApiError::InvalidBlobsBundle)?
            .validate(&versioned_hashes, EnvKzgSettings::default().get())?;

        Ok(versioned_hashes)
    }

    /// Core logic for validating the builder submission v4
    async fn _validate_builder_submission_v4(
        &self,
        request: ExtendedValidationRequestV4,
    ) -> Result<(), ValidationApiError> {
        info!(target: "rpc::relay", "Validating builder submission v4 test");
        let block = self.payload_validator.ensure_well_formed_payload(ExecutionData {
            payload: ExecutionPayload::V3(request.base.request.execution_payload),
            sidecar: ExecutionPayloadSidecar::v4(
                CancunPayloadFields {
                    parent_beacon_block_root: request.base.parent_beacon_block_root,
                    versioned_hashes: self
                        .validate_blobs_bundle(request.base.request.blobs_bundle)?,
                },
                PraguePayloadFields {
                    requests: RequestsOrHash::Requests(
                        request.base.request.execution_requests.to_requests(),
                    ),
                },
            ),
        })?;

        self.validate_message_against_block(
            block,
            request.base.request.message,
            request.base.registered_gas_limit,
            request.apply_blacklist,
            request.inclusion_list,
            None,
        )
        .await
    }

    /// Core logic for validating the builder submission v5. `base_payment_tx_index` is `Some`
    /// only for the merged-block-only endpoint, and switches the payment check from
    /// `ensure_payment` to `ensure_merged_payment`.
    async fn _validate_builder_submission_v5(
        &self,
        request: ExtendedValidationRequestV5,
        base_payment_tx_index: Option<usize>,
    ) -> Result<(), ValidationApiError> {
        let ExtendedValidationRequestV5 { base, inclusion_list, apply_blacklist } = request;
        let block = self.payload_into_block(
            ExecutionPayload::V3(base.request.execution_payload),
            base.parent_beacon_block_root,
            base.request.blobs_bundle,
            base.request.execution_requests.to_requests(),
        )?;

        self.validate_message_against_block(
            block,
            base.request.message,
            base.registered_gas_limit,
            apply_blacklist,
            inclusion_list,
            base_payment_tx_index,
        )
        .await
    }

    /// Core logic for validating the builder submission v6, the Amsterdam counterpart of
    /// `_validate_builder_submission_v5`. `execute_block` checks the payload's block access list
    /// against execution.
    async fn _validate_builder_submission_v6(
        &self,
        request: ExtendedValidationRequestV6,
        base_payment_tx_index: Option<usize>,
    ) -> Result<(), ValidationApiError> {
        let ExtendedValidationRequestV6 { base, inclusion_list, apply_blacklist } = request;
        let block = self.payload_into_block(
            ExecutionPayload::V4(base.request.execution_payload),
            base.parent_beacon_block_root,
            base.request.blobs_bundle,
            base.request.execution_requests.to_requests(),
        )?;

        self.validate_message_against_block(
            block,
            base.request.message,
            base.registered_gas_limit,
            apply_blacklist,
            inclusion_list,
            base_payment_tx_index,
        )
        .await
    }

    /// Converts an Osaka or Amsterdam payload into a block, validating its blobs bundle and its
    /// size.
    fn payload_into_block(
        &self,
        payload: ExecutionPayload,
        parent_beacon_block_root: B256,
        blobs_bundle: BlobsBundleV2,
        requests: Requests,
    ) -> Result<RecoveredBlock<Block>, ValidationApiError> {
        let block = self.payload_validator.ensure_well_formed_payload(ExecutionData {
            payload,
            sidecar: ExecutionPayloadSidecar::v4(
                CancunPayloadFields {
                    parent_beacon_block_root,
                    versioned_hashes: self.validate_blobs_bundle_v2(blobs_bundle)?,
                },
                PraguePayloadFields { requests: RequestsOrHash::Requests(requests) },
            ),
        })?;

        // Check block size as per EIP-7934 (only applies when Osaka hardfork is active)
        let chain_spec = self.provider.chain_spec();
        if chain_spec.is_osaka_active_at_timestamp(block.timestamp()) &&
            block.rlp_length() > MAX_RLP_BLOCK_SIZE
        {
            return Err(ValidationApiError::Consensus(ConsensusError::BlockTooLarge {
                rlp_length: block.rlp_length(),
                max_rlp_length: MAX_RLP_BLOCK_SIZE,
            }));
        }

        Ok(block)
    }

    pub(crate) fn get_parent_header(
        &self,
        parent_hash: B256,
    ) -> Result<SealedHeaderFor<EthPrimitives>, GetParentError> {
        let latest_header =
            self.provider.latest_header()?.ok_or_else(|| GetParentError::MissingLatestBlock)?;

        let parent_header = if parent_hash == latest_header.hash() {
            latest_header
        } else {
            // parent is not the latest header so we need to fetch it and ensure it's not too old
            let parent_header = self
                .provider
                .sealed_header_by_hash(parent_hash)?
                .ok_or_else(|| GetParentError::MissingParentBlock)?;

            if latest_header.number().saturating_sub(parent_header.number()) >
                self.validation_window
            {
                return Err(GetParentError::BlockTooOld);
            }
            parent_header
        };
        Ok(parent_header)
    }
}

#[async_trait]
impl BlockSubmissionValidationApiServer for ValidationApi {
    async fn validate_builder_submission_v1(
        &self,
        _request: BuilderBlockValidationRequest,
    ) -> RpcResult<()> {
        warn!(target: "rpc::relay", "Method `relay_validateBuilderSubmissionV1` is not supported");
        Err(internal_rpc_err("unimplemented"))
    }

    async fn validate_builder_submission_v2(
        &self,
        _request: BuilderBlockValidationRequestV2,
    ) -> RpcResult<()> {
        warn!(target: "rpc::relay", "Method `relay_validateBuilderSubmissionV2` is not supported");
        Err(internal_rpc_err("unimplemented"))
    }

    /// Validates a block submitted to the relay
    async fn validate_builder_submission_v3(
        &self,
        _request: BuilderBlockValidationRequestV3,
    ) -> RpcResult<()> {
        warn!(target: "rpc::relay", "Method `relay_validateBuilderSubmissionV3` is not supported");
        Err(internal_rpc_err("unimplemented"))
    }

    /// Validates a block submitted to the relay
    async fn validate_builder_submission_v4(
        &self,
        request: ExtendedValidationRequestV4,
    ) -> RpcResult<()> {
        let this = self.clone();
        self.spawn_validation(async move { this._validate_builder_submission_v4(request).await })
            .await
    }

    /// Validates a block submitted to the relay
    async fn validate_builder_submission_v5(
        &self,
        request: ExtendedValidationRequestV5,
    ) -> RpcResult<()> {
        let this = self.clone();
        self.spawn_validation(
            async move { this._validate_builder_submission_v5(request, None).await },
        )
        .await
    }

    /// Validates a merged block. Relay-internal only -- never reachable by an externally
    /// submitted builder block -- so it can recognise the base block's payment tx by index
    /// instead of requiring it to be the last transaction.
    async fn validate_merged_builder_submission_v5(
        &self,
        request: ExtendedMergedValidationRequestV5,
    ) -> RpcResult<()> {
        let this = self.clone();
        let index = Some(request.base_payment_tx_index as usize);
        self.spawn_validation(async move {
            this._validate_builder_submission_v5(request.base, index).await
        })
        .await
    }

    /// Validates an Amsterdam block submitted to the relay
    async fn validate_builder_submission_v6(
        &self,
        request: ExtendedValidationRequestV6,
    ) -> RpcResult<()> {
        let this = self.clone();
        self.spawn_validation(
            async move { this._validate_builder_submission_v6(request, None).await },
        )
        .await
    }

    /// Validates a merged Amsterdam block. Relay-internal only, as
    /// `validate_merged_builder_submission_v5`.
    async fn validate_merged_builder_submission_v6(
        &self,
        request: ExtendedMergedValidationRequestV6,
    ) -> RpcResult<()> {
        let this = self.clone();
        let index = Some(request.base_payment_tx_index as usize);
        self.spawn_validation(async move {
            this._validate_builder_submission_v6(request.base, index).await
        })
        .await
    }
}

/// Result of [`ValidationApi::execute_block`].
struct ExecutedBlock {
    output: BlockExecutionOutput<Receipt>,
    request_cache: CachedReads,
    state_provider: StateProviderBox,
    block_access_list_hash: Option<B256>,
}

/// The shared state of a [`ValidationApi`].
pub struct ValidationApiInner {
    /// The provider that can interact with the chain.
    pub(crate) provider: EthProvider,
    /// Consensus implementation.
    consensus: Arc<RethConsensus>,
    /// Execution payload validator.
    pub(crate) payload_validator: Arc<RethPayloadValidator>,
    /// Block executor factory.
    pub(crate) evm_config: RethEvmConfig,
    /// Set of disallowed addresses
    disallow: Arc<DashSet<Address>>,
    /// The maximum block distance - parent to latest - allowed for validation
    validation_window: u64,
    /// Cached state reads to avoid redundant disk I/O across multiple validation attempts
    /// targeting the same state. Stores a tuple of (`block_hash`, `cached_reads`) for the
    /// latest head block state. Uses async `RwLock` to safely handle concurrent validation
    /// requests.
    cached_state: RwLock<(B256, CachedReads)>,
    /// Task spawner for blocking operations
    pub(crate) task_spawner: TaskExecutor,
    /// Validation metrics
    metrics: ValidationMetrics,
}

impl fmt::Debug for ValidationApiInner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ValidationApiInner").finish_non_exhaustive()
    }
}

/// Configuration for validation API.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValidationApiConfig {
    /// Blacklist endpoint.
    pub blacklist_endpoint: String,
    /// The maximum block distance - parent to latest - allowed for validation
    pub validation_window: u64,
}

impl ValidationApiConfig {
    /// Default validation blocks window of 3 blocks
    pub const DEFAULT_VALIDATION_WINDOW: u64 = 3;

    /// Creates a configuration with the default validation window.
    pub const fn new(blacklist_endpoint: String) -> Self {
        Self { blacklist_endpoint, validation_window: Self::DEFAULT_VALIDATION_WINDOW }
    }
}

impl Default for ValidationApiConfig {
    fn default() -> Self {
        Self {
            blacklist_endpoint: Default::default(),
            validation_window: Self::DEFAULT_VALIDATION_WINDOW,
        }
    }
}

/// Metrics for the validation endpoint.
#[derive(Metrics)]
#[metrics(scope = "builder.validation")]
pub(crate) struct ValidationMetrics {
    /// The number of entries configured in the builder validation disallow list.
    pub(crate) disallow_size: Gauge,
    /// One once a disallow list has loaded, zero while none is in force.
    pub(crate) disallow_loaded: Gauge,
}

/// One transaction of an [`InclusionListWithMetadata`].
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
pub struct InclusionListTxWithMetadata {
    /// The transaction hash.
    pub hash: B256,
    /// The EIP-2718 encoded transaction.
    pub bytes: Bytes,
}

/// The transactions a submitted block must include, unless they fail on top of it.
#[derive(Clone, Debug, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct InclusionListWithMetadata {
    /// The transactions.
    pub txs: Vec<InclusionListTxWithMetadata>,
}

/// A v4 validation request with the relay's inclusion list and blacklist switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedValidationRequestV4 {
    /// The validation request.
    #[serde(flatten)]
    pub base: BuilderBlockValidationRequestV4,

    /// The inclusion list the block is checked against.
    pub inclusion_list: Option<InclusionListWithMetadata>,

    /// Whether the block is checked against the disallow list.
    #[serde(default)]
    pub apply_blacklist: bool,
}

/// A v5 validation request with the relay's inclusion list and blacklist switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedValidationRequestV5 {
    /// The validation request.
    #[serde(flatten)]
    pub base: BuilderBlockValidationRequestV5,

    /// The inclusion list the block is checked against.
    pub inclusion_list: Option<InclusionListWithMetadata>,

    /// Whether the block is checked against the disallow list.
    #[serde(default)]
    pub apply_blacklist: bool,
}

/// Merged-block counterpart of [`ExtendedValidationRequestV5`], carrying the extra
/// `base_payment_tx_index` the merged-block-only validation endpoint uses to recognise the
/// base block's own payment tx directly.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedMergedValidationRequestV5 {
    /// The validation request.
    #[serde(flatten)]
    pub base: ExtendedValidationRequestV5,

    /// The index of the base block's payment transaction.
    pub base_payment_tx_index: u64,
}

/// A v6 (Amsterdam) validation request with the relay's inclusion list and blacklist switch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedValidationRequestV6 {
    /// The validation request.
    #[serde(flatten)]
    pub base: BuilderBlockValidationRequestV6,

    /// The inclusion list the block is checked against.
    pub inclusion_list: Option<InclusionListWithMetadata>,

    /// Whether the block is checked against the disallow list.
    #[serde(default)]
    pub apply_blacklist: bool,
}

/// Merged-block counterpart of [`ExtendedValidationRequestV6`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExtendedMergedValidationRequestV6 {
    /// The validation request.
    #[serde(flatten)]
    pub base: ExtendedValidationRequestV6,

    /// The index of the base block's payment transaction.
    pub base_payment_tx_index: u64,
}

/// Block validation rpc interface.
#[rpc(server, namespace = "relay")]
pub trait BlockSubmissionValidationApi {
    /// A Request to validate a block submission.
    #[method(name = "validateBuilderSubmissionV1")]
    async fn validate_builder_submission_v1(
        &self,
        request: BuilderBlockValidationRequest,
    ) -> RpcResult<()>;

    /// A Request to validate a block submission.
    #[method(name = "validateBuilderSubmissionV2")]
    async fn validate_builder_submission_v2(
        &self,
        request: BuilderBlockValidationRequestV2,
    ) -> RpcResult<()>;

    /// A Request to validate a block submission.
    #[method(name = "validateBuilderSubmissionV3")]
    async fn validate_builder_submission_v3(
        &self,
        request: BuilderBlockValidationRequestV3,
    ) -> RpcResult<()>;

    /// A Request to validate a block submission.
    #[method(name = "validateBuilderSubmissionV4")]
    async fn validate_builder_submission_v4(
        &self,
        request: ExtendedValidationRequestV4,
    ) -> RpcResult<()>;

    /// A Request to validate a block submission.
    #[method(name = "validateBuilderSubmissionV5")]
    async fn validate_builder_submission_v5(
        &self,
        request: ExtendedValidationRequestV5,
    ) -> RpcResult<()>;

    /// A request to validate a merged block. Relay-internal only.
    #[method(name = "validateMergedBuilderSubmissionV5")]
    async fn validate_merged_builder_submission_v5(
        &self,
        request: ExtendedMergedValidationRequestV5,
    ) -> RpcResult<()>;

    /// A Request to validate an Amsterdam block submission.
    #[method(name = "validateBuilderSubmissionV6")]
    async fn validate_builder_submission_v6(
        &self,
        request: ExtendedValidationRequestV6,
    ) -> RpcResult<()>;

    /// A request to validate a merged Amsterdam block. Relay-internal only.
    #[method(name = "validateMergedBuilderSubmissionV6")]
    async fn validate_merged_builder_submission_v6(
        &self,
        request: ExtendedMergedValidationRequestV6,
    ) -> RpcResult<()>;
}
