//! Traits for the Flashblocks module.

use std::sync::Arc;

use alloy_eips::BlockNumberOrTag;
use alloy_network::Ethereum;
use alloy_primitives::{Address, TxHash, U256};
use alloy_rpc_types_eth::{Filter, Log};
use arc_swap::Guard;
use reth_rpc_convert::RpcTransaction;
use reth_rpc_eth_api::{RpcBlock, RpcReceipt};
use tokio::sync::broadcast;

use crate::{fee::ReceivedInclusionFee, payload::FlashBlock, pending_blocks::PendingBlocks};

/// Trait for receiving flashblock updates.
pub trait FlashblocksReceiver {
    /// Called when a new flashblock is received.
    fn on_flashblock_received(&self, flashblock: FlashBlock);
}

/// Core API for accessing flashblock state and data.
pub trait FlashblocksAPI {
    /// Retrieves the pending blocks.
    fn get_pending_blocks(&self) -> Guard<Option<Arc<PendingBlocks>>>;

    /// Subscribes to flashblock updates.
    fn subscribe_to_flashblocks(&self) -> broadcast::Receiver<Arc<PendingBlocks>>;

    /// `None` until a flashblock carries one, and again after one that carries none.
    fn latest_inclusion_fee(&self) -> Option<Arc<ReceivedInclusionFee>>;
}

/// API for accessing pending blocks data.
pub trait PendingBlocksAPI {
    /// Get the canonical block number on top of which all pending state is built
    fn get_canonical_block_number(&self) -> BlockNumberOrTag;

    /// Retrieves the current block. If `full` is true, includes full transaction details.
    fn get_block(&self, full: bool) -> Option<RpcBlock<Ethereum>>;

    /// Gets transaction receipt by hash.
    fn get_transaction_receipt(&self, tx_hash: TxHash) -> Option<RpcReceipt<Ethereum>>;

    /// Gets the receipts of the current block, in block order.
    fn get_block_receipts(&self) -> Option<Vec<RpcReceipt<Ethereum>>>;

    /// Gets the current block's transaction at `index`.
    fn get_transaction_by_index(&self, index: usize) -> Option<RpcTransaction<Ethereum>>;

    /// Gets transaction details by hash.
    fn get_transaction_by_hash(&self, tx_hash: TxHash) -> Option<RpcTransaction<Ethereum>>;

    /// Gets balance for an address. Returns None if address not updated in flashblocks.
    fn get_balance(&self, address: Address) -> Option<U256>;

    /// Gets logs from pending state matching the provided filter.
    fn get_pending_logs(&self, filter: &Filter) -> Vec<Log>;
}
