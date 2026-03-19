//! Pending state builder for executing flashblock transactions.

use std::{sync::Arc, time::Instant};

use alloy_consensus::{
    Block, Eip658Value, Header, Receipt, ReceiptEnvelope, TxEnvelope, TxType,
    transaction::{Recovered, TransactionMeta},
};
use alloy_primitives::B256;
use alloy_rpc_types::TransactionTrait;
use alloy_rpc_types_eth::state::StateOverride;
use reth::{
    chainspec::EthChainSpec,
    revm::{
        DatabaseCommit,
        context::result::{ExecutionResult, ResultAndState},
        state::EvmState,
    },
    rpc::server_types::eth::receipt::build_receipt,
};
use reth_ethereum_primitives::TransactionSigned;
use reth_evm::{Database, Evm, FromRecoveredTx, block::{StateDB, SystemCaller}};
use reth_primitives::EthereumHardforks;
use reth_rpc_convert::transaction::ConvertReceiptInput;

use crate::{ExecutionError, PendingBlocks, StateProcessorError};

/// Represents the result of executing or fetching a cached pending transaction.
#[derive(Debug, Clone)]
pub struct ExecutedPendingTransaction {
    /// The RPC transaction.
    pub rpc_transaction: alloy_rpc_types::Transaction,
    /// The receipt of the transaction.
    pub receipt: alloy_rpc_types::TransactionReceipt,
    /// The updated EVM state.
    pub state: EvmState,
    /// The execution result of the transaction.
    pub result: ExecutionResult,
    /// Per-transaction EVM execution time, if known.
    pub execution_time_us: Option<u128>,
}

#[derive(Debug)]
struct CachedTransactionExecution {
    receipt: alloy_rpc_types::TransactionReceipt,
    state: EvmState,
    result: ExecutionResult,
    execution_time_us: Option<u128>,
}

/// Executes or fetches cached values for transactions in a flashblock.
#[derive(Debug)]
pub struct PendingStateBuilder<E, ChainSpec> {
    cumulative_gas_used: u64,
    next_log_index: usize,

    evm: E,
    pending_block: Block<TransactionSigned, Header>,
    chain_spec: ChainSpec,

    prev_pending_blocks: Option<Arc<PendingBlocks>>,
    state_overrides: StateOverride,
}

impl<E, CS, DB> PendingStateBuilder<E, CS>
where
    E: Evm<DB = DB, HaltReason = reth::revm::context::result::HaltReason>,
    DB: Database + DatabaseCommit,
    E::Tx: FromRecoveredTx<TransactionSigned>,
    CS: EthChainSpec + EthereumHardforks + Clone,
{
    /// Creates a new pending state builder.
    pub fn new(
        chain_spec: CS,
        evm: E,
        pending_block: Block<TransactionSigned, Header>,
        prev_pending_blocks: Option<Arc<PendingBlocks>>,
        state_overrides: StateOverride,
    ) -> Self {
        Self {
            pending_block,
            evm,
            cumulative_gas_used: 0,
            next_log_index: 0,
            prev_pending_blocks,
            state_overrides,
            chain_spec,
        }
    }

    /// Consumes the builder and returns the database and state overrides.
    pub fn into_db_and_state_overrides(self) -> (DB, StateOverride) {
        (self.evm.into_db(), self.state_overrides)
    }

    /// Returns a mutable reference to the underlying database.
    pub fn db_mut(&mut self) -> &mut DB {
        self.evm.db_mut()
    }

    /// Executes a single transaction and updates internal state.
    /// Should be called in order for each transaction.
    pub fn execute_transaction(
        &mut self,
        idx: usize,
        transaction: Recovered<TransactionSigned>,
    ) -> Result<ExecutedPendingTransaction, StateProcessorError> {
        let tx_hash = *transaction.tx_hash();

        let effective_gas_price = self
            .pending_block
            .base_fee_per_gas
            .map(|base_fee| {
                transaction.effective_tip_per_gas(base_fee).unwrap_or_default()
                    + base_fee as u128
            })
            .unwrap_or_else(|| transaction.max_fee_per_gas());

        // Check if we have all the data we need to reuse the previous execution.
        let cached_execution = self.prev_pending_blocks.as_ref().and_then(|p| {
            Some(CachedTransactionExecution {
                receipt: p.get_receipt(tx_hash)?.clone(),
                state: p.get_transaction_state(&tx_hash)?,
                result: p.get_transaction_result(&tx_hash)?.clone(),
                execution_time_us: p.get_execution_time(&tx_hash),
            })
        });

        if let Some(cached_execution) = cached_execution {
            self.execute_with_cached_data(transaction, cached_execution, idx, effective_gas_price)
        } else {
            self.execute_with_evm(transaction, idx, effective_gas_price)
        }
    }

    /// Applies EIP-4788 and EIP-2935 pre-execution changes to the EVM.
    ///
    /// Must be called once per block, before executing any transactions.
    pub fn apply_pre_execution_changes(
        &mut self,
        parent_hash: B256,
        parent_beacon_block_root: Option<B256>,
    ) -> Result<(), StateProcessorError>
    where
        DB: Database + StateDB,
        CS: Clone,
    {
        let state_clear_flag =
            self.chain_spec.is_spurious_dragon_active_at_block(self.pending_block.number);
        self.evm.db_mut().set_state_clear_flag(state_clear_flag);

        let mut system_caller = SystemCaller::new(self.chain_spec.clone());
        system_caller
            .apply_blockhashes_contract_call(parent_hash, &mut self.evm)
            .map_err(|e| ExecutionError::EvmEnv(e.to_string()))?;
        system_caller
            .apply_beacon_root_contract_call(parent_beacon_block_root, &mut self.evm)
            .map_err(|e| ExecutionError::EvmEnv(e.to_string()))?;

        Ok(())
    }

    /// Builds transaction result from cached receipt and state data.
    fn execute_with_cached_data(
        &mut self,
        transaction: Recovered<TransactionSigned>,
        cached_execution: CachedTransactionExecution,
        idx: usize,
        effective_gas_price: u128,
    ) -> Result<ExecutedPendingTransaction, StateProcessorError> {
        let CachedTransactionExecution { receipt, state, result, execution_time_us } =
            cached_execution;

        let envelope: Recovered<TxEnvelope> = transaction.convert();

        let rpc_transaction = alloy_rpc_types::Transaction {
            inner: envelope,
            block_hash: None,
            block_number: Some(self.pending_block.number),
            transaction_index: Some(idx as u64),
            effective_gas_price: Some(effective_gas_price),
        };

        self.cumulative_gas_used = self
            .cumulative_gas_used
            .checked_add(receipt.gas_used)
            .ok_or(ExecutionError::GasOverflow)?;
        self.next_log_index += receipt.inner.logs().len();

        Ok(ExecutedPendingTransaction {
            rpc_transaction,
            receipt,
            state,
            result,
            execution_time_us,
        })
    }

    /// Builds an Ethereum receipt envelope from an execution result and transaction type.
    fn build_receipt_envelope(
        &self,
        result: &ExecutionResult,
        tx_type: TxType,
    ) -> ReceiptEnvelope {
        let receipt = Receipt {
            status: Eip658Value::Eip658(result.is_success()),
            cumulative_gas_used: self.cumulative_gas_used,
            logs: result.logs().to_vec(),
        }
        .with_bloom();

        match tx_type {
            TxType::Legacy => ReceiptEnvelope::Legacy(receipt),
            TxType::Eip2930 => ReceiptEnvelope::Eip2930(receipt),
            TxType::Eip1559 => ReceiptEnvelope::Eip1559(receipt),
            TxType::Eip4844 => ReceiptEnvelope::Eip4844(receipt),
            TxType::Eip7702 => ReceiptEnvelope::Eip7702(receipt),
        }
    }

    /// Executes the transaction through the EVM and builds the result from scratch.
    fn execute_with_evm(
        &mut self,
        transaction: Recovered<TransactionSigned>,
        idx: usize,
        effective_gas_price: u128,
    ) -> Result<ExecutedPendingTransaction, StateProcessorError> {
        let tx_hash = *transaction.tx_hash();

        let start = Instant::now();
        let transact_result = self.evm.transact(&transaction);
        let elapsed_us = start.elapsed().as_micros();

        match transact_result {
            Ok(ResultAndState { state, result }) => {
                let gas_used = result.gas_used();
                for (addr, acc) in &state {
                    let existing_override = self.state_overrides.entry(*addr).or_default();
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

                self.cumulative_gas_used = self
                    .cumulative_gas_used
                    .checked_add(gas_used)
                    .ok_or(ExecutionError::GasOverflow)?;

                let tx_type = transaction.tx_type();
                let receipt_envelope: reth_ethereum_primitives::EthereumReceipt =
                    self.build_receipt_envelope(&result, tx_type).into();

                let meta = TransactionMeta {
                    tx_hash,
                    index: idx as u64,
                    block_hash: B256::ZERO,
                    block_number: self.pending_block.number,
                    base_fee: self.pending_block.base_fee_per_gas,
                    excess_blob_gas: self.pending_block.excess_blob_gas,
                    timestamp: self.pending_block.timestamp,
                };

                let sender = transaction.signer();
                let receipt_input: ConvertReceiptInput<'_, reth_primitives::EthPrimitives> =
                    ConvertReceiptInput {
                        receipt: receipt_envelope,
                        tx: Recovered::new_unchecked(transaction.inner(), sender),
                        gas_used,
                        next_log_index: self.next_log_index,
                        meta,
                    };

                let blob_params =
                    self.chain_spec.blob_params_at_timestamp(self.pending_block.timestamp);
                let eth_receipt =
                    build_receipt(receipt_input, blob_params, |receipt, next_log_index, meta| {
                        receipt.into_rpc(next_log_index, meta).into()
                    });

                self.next_log_index += result.logs().len();

                let envelope: Recovered<TxEnvelope> = transaction.convert();

                let rpc_transaction = alloy_rpc_types::Transaction {
                    inner: envelope,
                    block_hash: None,
                    block_number: Some(self.pending_block.number),
                    transaction_index: Some(idx as u64),
                    effective_gas_price: Some(effective_gas_price),
                };
                self.evm.db_mut().commit(state.clone());

                Ok(ExecutedPendingTransaction {
                    rpc_transaction,
                    receipt: eth_receipt,
                    state,
                    result,
                    execution_time_us: Some(elapsed_us),
                })
            }
            Err(e) => Err(ExecutionError::TransactionFailed {
                tx_hash,
                sender: transaction.signer(),
                reason: format!("{e:?}"),
            }
            .into()),
        }
    }
}
