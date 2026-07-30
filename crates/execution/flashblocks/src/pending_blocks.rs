//! Pending blocks built from flashblocks.

use std::{sync::Arc, time::Instant};

use alloy_consensus::Header;
use alloy_eips::{BlockNumberOrTag, eip4895::Withdrawal};
use alloy_network::Ethereum;
use alloy_primitives::{
    Address, B256, BlockNumber, Sealed, TxHash, U256,
    map::foldhash::{HashMap, HashMapExt},
};
use alloy_provider::network::{TransactionResponse, primitives::BlockTransactions};
use alloy_rpc_types::{Filter, Log, Transaction, TransactionReceipt};
use alloy_rpc_types_engine::PayloadId;
use alloy_rpc_types_eth::{Header as RPCHeader, state::StateOverride};
use arc_swap::Guard;
use reth_revm::{db::BundleState, state::EvmState};
use reth_rpc_convert::RpcTransaction;
use reth_rpc_eth_api::{RpcBlock, RpcReceipt};

use crate::{
    error::{BuildError, StateProcessorError},
    metrics::Metrics,
    payload::FlashBlock,
    traits::PendingBlocksAPI,
};

/// A full transaction object with its associated logs and gas usage.
///
/// This is returned by `newFlashblockTransactions` subscription when `full = true`
/// or when a log filter is provided, giving both the transaction details, logs emitted
/// by its execution, and gas accounting fields.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TransactionWithLogs {
    /// The full transaction object.
    #[serde(flatten)]
    pub transaction: Transaction,
    /// Logs emitted by this transaction.
    pub logs: Vec<Log>,
    /// Gas consumed by this transaction's execution.
    pub gas_used: Option<u64>,
}

/// Builder for [`PendingBlocks`].
#[derive(Debug)]
pub struct PendingBlocksBuilder {
    flashblocks: Vec<FlashBlock>,
    headers: Vec<Sealed<Header>>,

    transactions: Vec<Transaction>,
    account_balances: HashMap<Address, U256>,
    transaction_count: HashMap<Address, U256>,
    transaction_receipts: HashMap<B256, TransactionReceipt>,
    transactions_by_hash: HashMap<B256, Transaction>,
    transaction_state: HashMap<B256, EvmState>,
    transaction_senders: HashMap<B256, Address>,
    state_overrides: Option<StateOverride>,

    bundle_state: BundleState,
}

impl Default for PendingBlocksBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl PendingBlocksBuilder {
    /// Creates a new empty builder.
    pub fn new() -> Self {
        Self {
            flashblocks: Vec::new(),
            headers: Vec::new(),
            transactions: Vec::new(),
            account_balances: HashMap::new(),
            transaction_count: HashMap::new(),
            transaction_receipts: HashMap::new(),
            transactions_by_hash: HashMap::new(),
            transaction_state: HashMap::new(),
            transaction_senders: HashMap::new(),
            state_overrides: None,
            bundle_state: BundleState::default(),
        }
    }

    /// Adds flashblocks to the builder.
    #[inline]
    pub fn with_flashblocks(&mut self, flashblocks: impl IntoIterator<Item = FlashBlock>) -> &Self {
        self.flashblocks.extend(flashblocks);
        self
    }

    /// Adds a header to the builder.
    #[inline]
    pub fn with_header(&mut self, header: Sealed<Header>) -> &Self {
        self.headers.push(header);
        self
    }

    /// Stores a transaction in the builder.
    #[inline]
    pub fn with_transaction(&mut self, transaction: Transaction) -> &Self {
        self.transactions_by_hash.insert(transaction.tx_hash(), transaction.clone());
        self.transactions.push(transaction);
        self
    }

    /// Stores the EVM state changes produced by a transaction.
    #[inline]
    pub fn with_transaction_state(&mut self, hash: B256, state: EvmState) -> &Self {
        self.transaction_state.insert(hash, state);
        self
    }

    /// Records the sender of a transaction.
    #[inline]
    pub fn with_transaction_sender(&mut self, hash: B256, sender: Address) -> &Self {
        self.transaction_senders.insert(hash, sender);
        self
    }

    /// Increments the pending nonce for an account.
    #[inline]
    pub fn increment_nonce(&mut self, sender: Address) -> &Self {
        let zero = U256::from(0);
        let current_count = self.transaction_count.get(&sender).unwrap_or(&zero);
        _ = self.transaction_count.insert(sender, *current_count + U256::from(1));
        self
    }

    /// Stores the receipt for a transaction.
    #[inline]
    pub fn with_receipt(&mut self, hash: B256, receipt: TransactionReceipt) -> &Self {
        self.transaction_receipts.insert(hash, receipt);
        self
    }

    /// Records the balance of an account after execution.
    #[inline]
    pub fn with_account_balance(&mut self, address: Address, balance: U256) -> &Self {
        self.account_balances.insert(address, balance);
        self
    }

    /// Sets state overrides for the pending blocks.
    #[inline]
    pub fn with_state_overrides(&mut self, state_overrides: StateOverride) -> &Self {
        self.state_overrides = Some(state_overrides);
        self
    }

    /// Sets the accumulated bundle state.
    #[inline]
    pub fn with_bundle_state(&mut self, bundle_state: BundleState) -> &Self {
        self.bundle_state = bundle_state;
        self
    }

    /// Builds the pending blocks.
    pub fn build(self) -> Result<PendingBlocks, StateProcessorError> {
        let earliest_header = self.headers.first().cloned().ok_or(BuildError::MissingHeaders)?;
        let latest_header = self.headers.last().cloned().ok_or(BuildError::MissingHeaders)?;

        let latest_flashblock_index =
            self.flashblocks.last().map(|fb| fb.index).ok_or(BuildError::NoFlashblocks)?;

        Ok(PendingBlocks {
            earliest_header,
            latest_header,
            latest_flashblock_index,
            flashblocks: self.flashblocks,
            transactions: self.transactions,
            account_balances: self.account_balances,
            transaction_count: self.transaction_count,
            transaction_receipts: self.transaction_receipts,
            transactions_by_hash: self.transactions_by_hash,
            transaction_state: self.transaction_state,
            transaction_senders: self.transaction_senders,
            state_overrides: self.state_overrides,
            bundle_state: self.bundle_state,
        })
    }
}

/// Aggregated pending block state from flashblocks.
#[derive(Debug, Clone)]
pub struct PendingBlocks {
    earliest_header: Sealed<Header>,
    latest_header: Sealed<Header>,
    latest_flashblock_index: u64,
    flashblocks: Vec<FlashBlock>,
    transactions: Vec<Transaction>,

    account_balances: HashMap<Address, U256>,
    transaction_count: HashMap<Address, U256>,
    transaction_receipts: HashMap<B256, TransactionReceipt>,
    transactions_by_hash: HashMap<B256, Transaction>,
    transaction_state: HashMap<B256, EvmState>,
    transaction_senders: HashMap<B256, Address>,
    state_overrides: Option<StateOverride>,

    bundle_state: BundleState,
}

impl PendingBlocks {
    /// Returns the latest block number in the pending state.
    #[inline]
    pub fn latest_block_number(&self) -> BlockNumber {
        self.latest_header.number
    }

    /// Returns the canonical block number (the block before pending).
    #[inline]
    pub fn canonical_block_number(&self) -> BlockNumberOrTag {
        BlockNumberOrTag::Number(self.earliest_header.number - 1)
    }

    /// Returns the earliest block number in the pending state.
    #[inline]
    pub fn earliest_block_number(&self) -> BlockNumber {
        self.earliest_header.number
    }

    /// Returns the payload ID for the current build attempt.
    #[inline]
    pub fn payload_id(&self) -> PayloadId {
        self.flashblocks.first().map(|fb| fb.payload_id).unwrap_or_default()
    }

    /// Returns the index of the latest flashblock.
    #[inline]
    pub const fn latest_flashblock_index(&self) -> u64 {
        self.latest_flashblock_index
    }

    /// Returns the latest header.
    #[inline]
    pub fn latest_header(&self) -> Sealed<Header> {
        self.latest_header.clone()
    }

    /// Returns all flashblocks.
    pub fn get_flashblocks(&self) -> Vec<FlashBlock> {
        self.flashblocks.clone()
    }

    /// Returns the EVM state for a transaction.
    pub fn get_transaction_state(&self, hash: &B256) -> Option<EvmState> {
        self.transaction_state.get(hash).cloned()
    }

    /// Returns the sender of a transaction.
    pub fn get_transaction_sender(&self, tx_hash: &B256) -> Option<Address> {
        self.transaction_senders.get(tx_hash).copied()
    }

    /// Returns a clone of the bundle state.
    ///
    /// NOTE: This clones the entire `BundleState`, which contains a `HashMap` of all touched
    /// accounts and their storage slots. The cost scales with the number of accounts and
    /// storage slots modified in the flashblock. Monitor `bundle_state_clone_duration` and
    /// `bundle_state_clone_size` metrics to track if this becomes a bottleneck.
    pub fn get_bundle_state(&self) -> BundleState {
        let metrics = Metrics::default();
        let size = self.bundle_state.state.len();
        let start = Instant::now();
        let cloned = self.bundle_state.clone();
        metrics.bundle_state_clone_duration.record(start.elapsed());
        metrics.bundle_state_clone_size.record(size as f64);
        cloned
    }

    /// Returns all transactions for a specific block number.
    pub fn get_transactions_for_block(
        &self,
        block_number: BlockNumber,
    ) -> impl Iterator<Item = &Transaction> {
        self.transactions.iter().filter(move |tx| tx.block_number.unwrap_or(0) == block_number)
    }

    /// Returns all withdrawals collected from flashblocks.
    fn get_withdrawals(&self) -> Vec<Withdrawal> {
        self.flashblocks.iter().flat_map(|fb| fb.diff.withdrawals.clone()).collect()
    }

    /// Returns the latest block, optionally with full transaction details.
    pub fn get_latest_block(&self, full: bool) -> RpcBlock<Ethereum> {
        let header = self.latest_header();
        let block_number = header.number;
        let block_transactions: Vec<Transaction> =
            self.get_transactions_for_block(block_number).cloned().collect();

        let transactions = if full {
            BlockTransactions::Full(block_transactions)
        } else {
            let tx_hashes: Vec<B256> = block_transactions.iter().map(|tx| tx.tx_hash()).collect();
            BlockTransactions::Hashes(tx_hashes)
        };

        RpcBlock::<Ethereum> {
            header: RPCHeader::from_consensus(header, None, None),
            transactions,
            uncles: Vec::new(),
            withdrawals: Some(self.get_withdrawals().into()),
        }
    }

    /// Returns the receipt for a transaction.
    pub fn get_receipt(&self, tx_hash: TxHash) -> Option<&TransactionReceipt> {
        self.transaction_receipts.get(&tx_hash)
    }

    /// Returns a transaction by its hash.
    pub fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Option<&Transaction> {
        self.transactions_by_hash.get(&tx_hash)
    }

    /// Returns true if the transaction hash is in the pending blocks.
    pub fn has_transaction_hash(&self, tx_hash: &B256) -> bool {
        self.transactions_by_hash.contains_key(tx_hash)
    }

    /// Returns the transaction count for an address in pending state.
    pub fn get_transaction_count(&self, address: Address) -> U256 {
        self.transaction_count.get(&address).copied().unwrap_or_else(|| U256::from(0))
    }

    /// Returns the balance for an address in pending state.
    pub fn get_balance(&self, address: Address) -> Option<U256> {
        self.account_balances.get(&address).copied()
    }

    /// Returns the state overrides for the pending state.
    pub fn get_state_overrides(&self) -> Option<StateOverride> {
        self.state_overrides.clone()
    }

    /// Returns logs matching the filter from pending state.
    pub fn get_pending_logs(&self, filter: &Filter) -> Vec<Log> {
        let mut logs = Vec::new();

        for tx in &self.transactions {
            if let Some(receipt) = self.transaction_receipts.get(&tx.tx_hash()) {
                for log in receipt.inner.logs() {
                    if filter.matches(&log.inner) {
                        logs.push(log.clone());
                    }
                }
            }
        }

        logs
    }

    /// Returns all pending transactions from flashblocks.
    pub fn get_pending_transactions(&self) -> Vec<Transaction> {
        self.transactions.clone()
    }

    /// Returns transactions with their associated logs from only the latest flashblock (delta).
    ///
    /// Unlike `get_pending_transactions_with_logs`, this returns only transactions
    /// that were added in the most recent flashblock, avoiding duplicates
    /// when streaming via WebSocket subscriptions.
    pub fn get_pending_transactions_with_logs(&self) -> Vec<TransactionWithLogs> {
        let prev_count = self.previous_flashblocks_tx_count();

        self.transactions
            .iter()
            .skip(prev_count)
            .map(|tx| {
                let tx_hash = tx.tx_hash();
                let (logs, gas_used) = self
                    .transaction_receipts
                    .get(&tx_hash)
                    .map(|receipt| (receipt.inner.logs().to_vec(), Some(receipt.gas_used)))
                    .unwrap_or_default();
                TransactionWithLogs { transaction: tx.clone(), logs, gas_used }
            })
            .collect()
    }

    /// Returns the hashes of all pending transactions from flashblocks.
    pub fn get_pending_transaction_hashes(&self) -> Vec<B256> {
        self.transactions.iter().map(|tx| tx.tx_hash()).collect()
    }

    /// Returns the number of transactions in all flashblocks except the latest one.
    /// This is used to compute the delta (transactions only in the latest flashblock).
    fn previous_flashblocks_tx_count(&self) -> usize {
        if self.flashblocks.len() <= 1 {
            return 0;
        }
        self.flashblocks[..self.flashblocks.len() - 1]
            .iter()
            .map(|fb| fb.diff.transactions.len())
            .sum()
    }

    /// Returns logs matching the filter from only the latest flashblock (delta).
    ///
    /// Unlike `get_pending_logs`, this returns only logs from transactions
    /// that were added in the most recent flashblock, avoiding duplicates
    /// when streaming via WebSocket subscriptions.
    pub fn get_latest_flashblock_logs(&self, filter: &Filter) -> Vec<Log> {
        let prev_count = self.previous_flashblocks_tx_count();
        let mut logs = Vec::new();

        for tx in self.transactions.iter().skip(prev_count) {
            if let Some(receipt) = self.transaction_receipts.get(&tx.tx_hash()) {
                for log in receipt.inner.logs() {
                    if filter.matches(&log.inner) {
                        logs.push(log.clone());
                    }
                }
            }
        }

        logs
    }

    /// Returns transactions with their associated logs from only the latest flashblock (delta).
    ///
    /// Unlike `get_pending_transactions_with_logs`, this returns only transactions
    /// that were added in the most recent flashblock, avoiding duplicates
    /// when streaming via WebSocket subscriptions. Transactions without a known receipt are
    /// skipped (rather than emitted with empty logs / no gas) so every entry is complete —
    /// matching `get_latest_flashblock_transactions_with_logs_filtered`.
    pub fn get_latest_flashblock_transactions_with_logs(&self) -> Vec<TransactionWithLogs> {
        let prev_count = self.previous_flashblocks_tx_count();

        self.transactions
            .iter()
            .skip(prev_count)
            .filter_map(|tx| {
                let receipt = self.transaction_receipts.get(&tx.tx_hash())?;
                Some(TransactionWithLogs {
                    transaction: tx.clone(),
                    logs: receipt.inner.logs().to_vec(),
                    gas_used: Some(receipt.gas_used),
                })
            })
            .collect()
    }

    /// Returns transactions with their associated logs from only the latest flashblock (delta),
    /// filtered to include only transactions where at least one log matches the given filter.
    ///
    /// When a transaction matches, all of its logs are returned (not just the matching ones).
    /// This preserves full transaction context for subscribers who need complete log sets.
    pub fn get_latest_flashblock_transactions_with_logs_filtered(
        &self,
        filter: &Filter,
    ) -> Vec<TransactionWithLogs> {
        let prev_count = self.previous_flashblocks_tx_count();

        self.transactions
            .iter()
            .skip(prev_count)
            .filter_map(|tx| {
                let tx_hash = tx.tx_hash();
                let receipt = self.transaction_receipts.get(&tx_hash)?;
                let logs = receipt.inner.logs();

                let has_match = logs.iter().any(|log| filter.matches(&log.inner));
                if !has_match {
                    return None;
                }

                Some(TransactionWithLogs {
                    transaction: tx.clone(),
                    logs: logs.to_vec(),
                    gas_used: Some(receipt.gas_used),
                })
            })
            .collect()
    }

    /// Returns the hashes of transactions from only the latest flashblock (delta).
    ///
    /// Unlike `get_pending_transaction_hashes`, this returns only hashes
    /// of transactions that were added in the most recent flashblock,
    /// avoiding duplicates when streaming via WebSocket subscriptions.
    pub fn get_latest_flashblock_transaction_hashes(&self) -> Vec<B256> {
        let prev_count = self.previous_flashblocks_tx_count();
        self.transactions.iter().skip(prev_count).map(|tx| tx.tx_hash()).collect()
    }
}

impl PendingBlocksAPI for Guard<Option<Arc<PendingBlocks>>> {
    fn get_canonical_block_number(&self) -> BlockNumberOrTag {
        self.as_ref().map(|pb| pb.canonical_block_number()).unwrap_or(BlockNumberOrTag::Latest)
    }

    fn get_transaction_count(&self, address: Address) -> U256 {
        self.as_ref().map(|pb| pb.get_transaction_count(address)).unwrap_or_else(|| U256::from(0))
    }

    fn get_block(&self, full: bool) -> Option<RpcBlock<Ethereum>> {
        self.as_ref().map(|pb| pb.get_latest_block(full))
    }

    fn get_transaction_receipt(
        &self,
        tx_hash: alloy_primitives::TxHash,
    ) -> Option<RpcReceipt<Ethereum>> {
        self.as_ref().and_then(|pb| pb.get_receipt(tx_hash).cloned())
    }

    fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Option<RpcTransaction<Ethereum>> {
        self.as_ref().and_then(|pb| pb.get_transaction_by_hash(tx_hash).cloned())
    }

    fn get_balance(&self, address: Address) -> Option<U256> {
        self.as_ref().and_then(|pb| pb.get_balance(address))
    }

    fn get_state_overrides(&self) -> Option<StateOverride> {
        self.as_ref().map(|pb| pb.get_state_overrides()).unwrap_or_default()
    }

    fn get_pending_logs(&self, filter: &Filter) -> Vec<Log> {
        self.as_ref().map(|pb| pb.get_pending_logs(filter)).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::{
        Eip658Value, Header, Receipt, ReceiptEnvelope, ReceiptWithBloom, Sealed, Signed,
        TxEnvelope, TxLegacy, transaction::Recovered,
    };
    use alloy_network::TransactionResponse;
    use alloy_primitives::{
        Address, B256, Bloom, Bytes, Log as PrimitiveLog, LogData, Signature, TxKind, U256,
    };
    use alloy_rpc_types::{Filter, Log, Transaction, TransactionReceipt};
    use alloy_rpc_types_engine::PayloadId;

    use super::{PendingBlocks, PendingBlocksBuilder};
    use crate::payload::{ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata};

    fn test_flashblock() -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::default(),
            index: 0,
            base: None,
            diff: ExecutionPayloadFlashblockDeltaV1::default(),
            metadata: Metadata {
                block_number: 1,
                new_account_balances: Default::default(),
                receipts: Default::default(),
            },
        }
    }

    fn test_transaction_with_hash(hash: B256) -> Transaction {
        let legacy = TxLegacy {
            chain_id: Some(1),
            nonce: 0,
            gas_price: 1_000_000_000,
            gas_limit: 21_000,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            input: Bytes::new(),
        };
        let envelope =
            TxEnvelope::Legacy(Signed::new_unchecked(legacy, Signature::test_signature(), hash));
        Transaction {
            inner: Recovered::new_unchecked(envelope, Address::ZERO),
            block_hash: Some(B256::ZERO),
            block_number: Some(1),
            block_timestamp: None,
            transaction_index: Some(0),
            effective_gas_price: Some(1_000_000_000),
        }
    }

    fn receipt_with_topics(
        tx_hash: B256,
        log_address: Address,
        topics: Vec<B256>,
    ) -> TransactionReceipt {
        let log = Log {
            inner: PrimitiveLog {
                address: log_address,
                data: LogData::new_unchecked(topics, Bytes::new()),
            },
            block_hash: Some(B256::ZERO),
            block_number: Some(1),
            block_timestamp: None,
            transaction_hash: Some(tx_hash),
            transaction_index: Some(0),
            log_index: Some(0),
            removed: false,
        };
        TransactionReceipt {
            inner: ReceiptEnvelope::Legacy(ReceiptWithBloom {
                receipt: Receipt {
                    status: Eip658Value::Eip658(true),
                    cumulative_gas_used: 21_000,
                    logs: vec![log],
                },
                logs_bloom: Bloom::default(),
            }),
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: Some(B256::ZERO),
            block_number: Some(1),
            gas_used: 21_000,
            effective_gas_price: 1_000_000_000,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::ZERO,
            to: None,
            contract_address: None,
        }
    }

    fn receipt_with_log(tx_hash: B256, log_address: Address) -> TransactionReceipt {
        receipt_with_topics(tx_hash, log_address, vec![])
    }

    fn build_pending_blocks_with_logs(entries: &[(B256, Address)]) -> PendingBlocks {
        let mut builder = PendingBlocksBuilder::new();
        builder.with_flashblocks([test_flashblock()]);
        builder.with_header(Sealed::new_unchecked(Header::default(), B256::ZERO));
        for &(hash, addr) in entries {
            builder.with_transaction(test_transaction_with_hash(hash));
            builder.with_receipt(hash, receipt_with_log(hash, addr));
        }
        builder.build().expect("build should succeed")
    }

    fn build_pending_blocks_with_topics(entries: &[(B256, Address, B256)]) -> PendingBlocks {
        let mut builder = PendingBlocksBuilder::new();
        builder.with_flashblocks([test_flashblock()]);
        builder.with_header(Sealed::new_unchecked(Header::default(), B256::ZERO));
        for &(hash, addr, topic) in entries {
            builder.with_transaction(test_transaction_with_hash(hash));
            builder.with_receipt(hash, receipt_with_topics(hash, addr, vec![topic]));
        }
        builder.build().expect("build should succeed")
    }

    #[test]
    fn filtered_transactions_returns_only_matching_by_address() {
        let (ha, hb, hc) =
            (B256::with_last_byte(0xAA), B256::with_last_byte(0xBB), B256::with_last_byte(0xCC));
        let (aa, ab, ac) = (
            Address::with_last_byte(0x0A),
            Address::with_last_byte(0x0B),
            Address::with_last_byte(0x0C),
        );
        let pending = build_pending_blocks_with_logs(&[(ha, aa), (hb, ab), (hc, ac)]);

        let filter = Filter::new().address(ab);
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].transaction.tx_hash(), hb);
        assert_eq!(txs[0].logs.len(), 1);
        assert_eq!(txs[0].logs[0].address(), ab);
    }

    #[test]
    fn filtered_transactions_returns_only_matching_by_topic0() {
        let (ha, hb) = (B256::with_last_byte(0xAA), B256::with_last_byte(0xBB));
        let (aa, ab) = (Address::with_last_byte(0x0A), Address::with_last_byte(0x0B));
        let (ta, tb) = (B256::with_last_byte(0x11), B256::with_last_byte(0x22));
        let pending = build_pending_blocks_with_topics(&[(ha, aa, ta), (hb, ab, tb)]);

        let filter = Filter::new().event_signature(tb);
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].transaction.tx_hash(), hb);
    }

    #[test]
    fn filtered_transactions_returns_none_when_no_match() {
        let ha = B256::with_last_byte(0xAA);
        let aa = Address::with_last_byte(0x0A);
        let pending = build_pending_blocks_with_logs(&[(ha, aa)]);

        let filter = Filter::new().address(Address::with_last_byte(0xFF));
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert!(txs.is_empty());
    }

    #[test]
    fn filtered_transactions_populates_gas_used() {
        let ha = B256::with_last_byte(0xAA);
        let aa = Address::with_last_byte(0x0A);
        let pending = build_pending_blocks_with_logs(&[(ha, aa)]);

        let filter = Filter::new().address(aa);
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].gas_used, Some(21_000));
    }

    #[test]
    fn unfiltered_transactions_populate_logs_and_gas_used() {
        let (ha, hb) = (B256::with_last_byte(0xAA), B256::with_last_byte(0xBB));
        let (aa, ab) = (Address::with_last_byte(0x0A), Address::with_last_byte(0x0B));
        let pending = build_pending_blocks_with_logs(&[(ha, aa), (hb, ab)]);

        let txs = pending.get_pending_transactions_with_logs();

        assert_eq!(txs.len(), 2);
        assert_eq!(txs[0].gas_used, Some(21_000));
        assert_eq!(txs[0].logs.len(), 1);
        assert_eq!(txs[1].logs[0].address(), ab);
    }

    #[test]
    fn get_pending_logs_returns_logs_in_transaction_order() {
        let (ha, hb, hc) =
            (B256::with_last_byte(0xAA), B256::with_last_byte(0xBB), B256::with_last_byte(0xCC));
        let (aa, ab, ac) = (
            Address::with_last_byte(0x0A),
            Address::with_last_byte(0x0B),
            Address::with_last_byte(0x0C),
        );
        let pending = build_pending_blocks_with_logs(&[(ha, aa), (hb, ab), (hc, ac)]);

        let logs = pending.get_pending_logs(&Filter::default());

        assert_eq!(logs.len(), 3, "should return one log per transaction");
        assert_eq!(logs[0].address(), aa);
        assert_eq!(logs[1].address(), ab);
        assert_eq!(logs[2].address(), ac);
    }

    /// A receipt carrying two logs (each with empty topics) at distinct addresses.
    fn receipt_with_two_logs(
        tx_hash: B256,
        addr_a: Address,
        addr_b: Address,
    ) -> TransactionReceipt {
        let log = |index: u64, address: Address| Log {
            inner: PrimitiveLog { address, data: LogData::new_unchecked(vec![], Bytes::new()) },
            block_hash: Some(B256::ZERO),
            block_number: Some(1),
            block_timestamp: None,
            transaction_hash: Some(tx_hash),
            transaction_index: Some(0),
            log_index: Some(index),
            removed: false,
        };
        TransactionReceipt {
            inner: ReceiptEnvelope::Legacy(ReceiptWithBloom {
                receipt: Receipt {
                    status: Eip658Value::Eip658(true),
                    cumulative_gas_used: 42_000,
                    logs: vec![log(0, addr_a), log(1, addr_b)],
                },
                logs_bloom: Bloom::default(),
            }),
            transaction_hash: tx_hash,
            transaction_index: Some(0),
            block_hash: Some(B256::ZERO),
            block_number: Some(1),
            gas_used: 42_000,
            effective_gas_price: 1_000_000_000,
            blob_gas_used: None,
            blob_gas_price: None,
            from: Address::ZERO,
            to: None,
            contract_address: None,
        }
    }

    #[test]
    fn filtered_transactions_returns_all_logs_when_any_matches() {
        let hash_a = B256::with_last_byte(0xAA);
        let addr_match = Address::with_last_byte(0x0A);
        let addr_other = Address::with_last_byte(0x0B);

        let mut builder = PendingBlocksBuilder::new();
        builder.with_flashblocks([test_flashblock()]);
        builder.with_header(Sealed::new_unchecked(Header::default(), B256::ZERO));
        builder.with_transaction(test_transaction_with_hash(hash_a));
        builder.with_receipt(hash_a, receipt_with_two_logs(hash_a, addr_match, addr_other));
        let pending = builder.build().expect("build should succeed");

        let filter = Filter::new().address(addr_match);
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert_eq!(txs.len(), 1);
        // A matching tx returns ALL of its logs, not just the ones matching the filter.
        assert_eq!(txs[0].logs.len(), 2, "should return all logs, not just matching");
        assert_eq!(txs[0].logs[0].address(), addr_match);
        assert_eq!(txs[0].logs[1].address(), addr_other);
    }

    #[test]
    fn filtered_transactions_with_combined_address_and_topic() {
        let (hash_a, hash_b, hash_c) =
            (B256::with_last_byte(0xAA), B256::with_last_byte(0xBB), B256::with_last_byte(0xCC));
        let (addr_x, addr_y) = (Address::with_last_byte(0x0A), Address::with_last_byte(0x0B));
        let (topic_transfer, topic_approval) =
            (B256::with_last_byte(0x01), B256::with_last_byte(0x02));

        // Only (hash_a, addr_x, topic_transfer) matches both the address and the topic.
        let pending = build_pending_blocks_with_topics(&[
            (hash_a, addr_x, topic_transfer),
            (hash_b, addr_x, topic_approval),
            (hash_c, addr_y, topic_transfer),
        ]);

        let filter = Filter::new().address(addr_x).event_signature(topic_transfer);
        let txs = pending.get_latest_flashblock_transactions_with_logs_filtered(&filter);

        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].transaction.tx_hash(), hash_a);
    }
}
