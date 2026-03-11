use std::sync::Arc;

use crate::pending::PendingBlocks;
use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types::{Filter, Log};
use alloy_rpc_types_eth::state::StateOverride;
use arc_swap::Guard;
use reth_rpc_eth_api::{RpcBlock, RpcReceipt, RpcTransaction};
use tokio::sync::broadcast;

/// Max configured timeout for `eth_sendRawTransactionSync` in milliseconds.
pub const MAX_TIMEOUT_SEND_RAW_TX_SYNC_MS: u64 = 6_000;

/// Core API for accessing flashblock state and data.
pub trait FlashblocksAPI {
    /// Retrieves the pending blocks.
    fn get_pending_blocks(&self) -> Guard<Option<Arc<PendingBlocks>>>;

    /// Creates a subscription to receive flashblock updates.
    fn subscribe_to_flashblocks(&self) -> broadcast::Receiver<Arc<PendingBlocks>>;
}

pub trait PendingBlocksAPI {
    /// Get the canonical block number on top of which all pending state is built
    fn get_canonical_block_number(&self) -> BlockNumberOrTag;

    /// Get the pending transactions count for an address
    fn get_transaction_count(&self, address: Address) -> U256;

    /// Retrieves the current block. If `full` is true, includes full transaction details.
    fn get_block(&self, full: bool) -> Option<RpcBlock<Ethereum>>;

    /// Gets transaction receipt by hash.
    fn get_transaction_receipt(&self, tx_hash: TxHash) -> Option<RpcReceipt<Ethereum>>;

    /// Gets transaction details by hash.
    fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Option<RpcTransaction<Ethereum>>;

    /// Gets balance for an address. Returns None if address not updated in flashblocks.
    fn get_balance(&self, address: Address) -> Option<U256>;

    fn get_state_overrides(&self) -> Option<StateOverride>;

    /// Gets logs from pending state matching the provided filter.
    fn get_pending_logs(&self, filter: &Filter) -> Vec<Log>;
}
