use crate::payload::FlashBlock;
use alloy_consensus::Header;
use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::{
    Address, B256, BlockNumber, Sealed, TxHash, U256,
    map::foldhash::{HashMap, HashMapExt},
};
use alloy_provider::network::{TransactionResponse, primitives::BlockTransactions};
use alloy_rpc_types::{Filter, Log, Transaction, TransactionReceipt};
use alloy_rpc_types_eth::{Header as RPCHeader, state::StateOverride};
use eyre::eyre;
use reth::revm::{db::Cache, state::EvmState};
use reth_rpc_eth_api::RpcBlock;

pub struct PendingBlocksBuilder {
    flashblocks: Vec<FlashBlock>,
    headers: Vec<Sealed<Header>>,

    transactions: Vec<Transaction>,
    account_balances: HashMap<Address, U256>,
    transaction_count: HashMap<Address, U256>,
    transaction_receipts: HashMap<B256, TransactionReceipt>,
    transactions_by_hash: HashMap<B256, Transaction>,
    transaction_state: HashMap<B256, EvmState>,
    state_overrides: Option<StateOverride>,

    db_cache: Cache,
}

impl PendingBlocksBuilder {
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

            state_overrides: None,
            db_cache: Cache::default(),
        }
    }

    #[inline]
    pub(crate) fn with_flashblocks(&mut self, flashblocks: Vec<FlashBlock>) -> &Self {
        self.flashblocks = flashblocks;
        self
    }

    #[inline]
    pub(crate) fn with_header(&mut self, header: Sealed<Header>) -> &Self {
        self.headers.push(header);
        self
    }

    #[inline]
    pub(crate) fn with_transaction(&mut self, transaction: Transaction) -> &Self {
        self.transactions_by_hash.insert(transaction.tx_hash(), transaction.clone());
        self.transactions.push(transaction);
        self
    }

    #[inline]
    pub(crate) fn with_transaction_state(&mut self, hash: B256, state: EvmState) -> &Self {
        self.transaction_state.insert(hash, state);
        self
    }

    #[inline]
    pub(crate) fn with_db_cache(&mut self, cache: Cache) -> &Self {
        self.db_cache = cache;
        self
    }

    #[inline]
    pub(crate) fn increment_nonce(&mut self, sender: Address) -> &Self {
        let current_count = self.transaction_count.get(&sender).unwrap_or(&U256::ZERO);
        _ = self.transaction_count.insert(sender, *current_count + U256::from(1));
        self
    }

    #[inline]
    pub(crate) fn with_receipt(&mut self, hash: B256, receipt: TransactionReceipt) -> &Self {
        self.transaction_receipts.insert(hash, receipt);
        self
    }

    #[inline]
    pub(crate) fn with_account_balance(&mut self, address: Address, balance: U256) -> &Self {
        self.account_balances.insert(address, balance);
        self
    }

    #[inline]
    pub(crate) fn with_state_overrides(&mut self, state_overrides: StateOverride) -> &Self {
        self.state_overrides = Some(state_overrides);
        self
    }

    pub(crate) fn build(self) -> eyre::Result<PendingBlocks> {
        if self.headers.is_empty() {
            return Err(eyre!("missing headers"));
        }

        if self.flashblocks.is_empty() {
            return Err(eyre!("no flashblocks"));
        }

        Ok(PendingBlocks {
            flashblocks: self.flashblocks,
            headers: self.headers,
            transactions: self.transactions,
            account_balances: self.account_balances,
            transaction_count: self.transaction_count,
            transaction_receipts: self.transaction_receipts,
            transactions_by_hash: self.transactions_by_hash,
            transaction_state: self.transaction_state,
            state_overrides: self.state_overrides,
            db_cache: self.db_cache,
        })
    }
}

#[derive(Debug, Clone)]
pub struct PendingBlocks {
    flashblocks: Vec<FlashBlock>,
    headers: Vec<Sealed<Header>>,
    transactions: Vec<Transaction>,

    account_balances: HashMap<Address, U256>,
    transaction_count: HashMap<Address, U256>,
    transaction_receipts: HashMap<B256, TransactionReceipt>,
    transactions_by_hash: HashMap<B256, Transaction>,
    transaction_state: HashMap<B256, EvmState>,
    state_overrides: Option<StateOverride>,

    db_cache: Cache,
}

impl PendingBlocks {
    pub fn latest_block_number(&self) -> BlockNumber {
        self.headers.last().unwrap().number
    }

    pub fn canonical_block_number(&self) -> BlockNumberOrTag {
        BlockNumberOrTag::Number(self.headers.first().unwrap().number - 1)
    }

    pub fn latest_flashblock_index(&self) -> u64 {
        self.flashblocks.last().unwrap().index
    }

    pub fn latest_header(&self) -> Sealed<Header> {
        self.headers.last().unwrap().clone()
    }

    pub fn get_flashblocks(&self) -> Vec<FlashBlock> {
        self.flashblocks.clone()
    }

    pub fn get_transaction_state(&self, hash: B256) -> Option<EvmState> {
        self.transaction_state.get(&hash).cloned()
    }

    pub fn get_db_cache(&self) -> Cache {
        self.db_cache.clone()
    }

    pub fn get_transactions_for_block(&self, block_number: BlockNumber) -> Vec<Transaction> {
        self.transactions
            .iter()
            .filter(|tx| tx.block_number.unwrap_or(0) == block_number)
            .cloned()
            .collect()
    }

    pub fn get_latest_block(&self, full: bool) -> RpcBlock<Ethereum> {
        let header = self.latest_header();
        let block_number = header.number;
        let block_transactions: Vec<Transaction> = self.get_transactions_for_block(block_number);

        let transactions = if full {
            BlockTransactions::Full(block_transactions)
        } else {
            let tx_hashes: Vec<B256> =
                block_transactions.iter().map(|tx| tx.tx_hash()).collect();
            BlockTransactions::Hashes(tx_hashes.clone())
        };

        RpcBlock::<Ethereum> {
            header: RPCHeader::from_consensus(header.clone(), None, None),
            transactions,
            uncles: Vec::new(),
            withdrawals: None,
        }
    }

    pub fn get_receipt(&self, tx_hash: TxHash) -> Option<TransactionReceipt> {
        self.transaction_receipts.get(&tx_hash).cloned()
    }

    pub fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Option<Transaction> {
        self.transactions_by_hash.get(&tx_hash).cloned()
    }

    pub fn get_transaction_count(&self, address: Address) -> U256 {
        self.transaction_count.get(&address).cloned().unwrap_or(U256::from(0))
    }

    pub fn get_balance(&self, address: Address) -> Option<U256> {
        self.account_balances.get(&address).cloned()
    }

    pub fn get_state_overrides(&self) -> Option<StateOverride> {
        self.state_overrides.clone()
    }

    pub fn get_pending_logs(&self, filter: &Filter) -> Vec<Log> {
        let mut logs = Vec::new();

        // Iterate through all transaction receipts in pending state
        for receipt in self.transaction_receipts.values() {
            for log in receipt.inner.logs() {
                if filter.matches(&log.inner) {
                    logs.push(log.clone());
                }
            }
        }

        logs
    }

    /// Returns all pending transactions from flashblocks.
    pub fn get_pending_transactions(&self) -> Vec<Transaction> {
        self.transactions.clone()
    }

    /// Returns the hashes of all pending transactions from flashblocks.
    pub fn get_pending_transaction_hashes(&self) -> Vec<B256> {
        self.transactions.iter().map(|tx| tx.tx_hash()).collect()
    }
}
