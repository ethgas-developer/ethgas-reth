//! Builds the inclusion list from the transaction pool after every canonical block.

use std::{cmp::Ordering, collections::BTreeSet, sync::Arc};

use alloy_consensus::Transaction;
use alloy_primitives::{Bytes, TxHash};
use futures::StreamExt;
use reth_provider::{CanonStateNotification, CanonStateNotificationStream};
use reth_transaction_pool::{
    FullTransactionEvent, PoolTransaction, TransactionPool, ValidPoolTransaction,
};
use rustc_hash::FxHashMap;
use tokio::sync::watch::Sender;
use tracing::{error, info, warn};

const MAX_LIST_BYTES: usize = 8 * 1024;

/// Tracks the pool's pending transactions and publishes a new inclusion list on every canonical
/// commit, until the pool's event stream or the canonical state stream closes.
pub async fn inclusion_producer<P: TransactionPool>(
    pool: P,
    mut notifications: CanonStateNotificationStream,
    published: Sender<Option<Vec<Bytes>>>,
) {
    // Maintain 2 collections - 1 ordered by Inclusion List score, with `TxHash` as value. The other
    // mapping `TxHash` to tx.
    let mut ordered_txs = BTreeSet::new();
    let mut pending_txs = FxHashMap::default();

    let mut tx_event_listener = pool.all_transactions_event_listener();

    loop {
        tokio::select! {
            event = tx_event_listener.next() => {
                match event {
                    Some(event) => handle_tx_event(&pool, event, &mut ordered_txs, &mut pending_txs),
                    None => {
                        warn!("tx pool event listener closed - exiting");
                        break;
                    }
                }
            },
            result = notifications.next() => {
                match result {
                    Some(update) => {
                        if let CanonStateNotification::Commit { new: _ } = update {
                            build_inclusion_list::<P>(&mut ordered_txs, &pending_txs, &published);
                        }
                    }
                    None => {
                        error!("state notifications closed - exiting");
                        break;
                    }
                }

            },
        }
    }
}

fn handle_tx_event<P: TransactionPool>(
    pool: &P,
    event: FullTransactionEvent<P::Transaction>,
    ordered_tx: &mut BTreeSet<OrderedTx>,
    pending_txs: &mut FxHashMap<TxHash, Arc<ValidPoolTransaction<P::Transaction>>>,
) {
    match event {
        FullTransactionEvent::Pending(tx_hash) => {
            if let Some(pending_tx) = pool.get(&tx_hash) &&
                matches!(pending_tx.transaction.blob_count(), Some(0) | None)
            {
                let tx_hash = *pending_tx.hash();
                let score = score_tx(&pending_tx);
                ordered_tx.insert(OrderedTx { score, tx_hash });
                pending_txs.insert(tx_hash, pending_tx);
            }
        }
        FullTransactionEvent::Queued(tx_hash, _) |
        FullTransactionEvent::Mined { tx_hash, .. } |
        FullTransactionEvent::Discarded(tx_hash) |
        FullTransactionEvent::Invalid(tx_hash) => {
            pending_txs.remove(&tx_hash);
        }
        FullTransactionEvent::Replaced { transaction, replaced_by: _ } => {
            let tx_hash = transaction.hash();
            pending_txs.remove(tx_hash);
        }
        FullTransactionEvent::Propagated(_) => {}
    }
}

fn build_inclusion_list<P: TransactionPool>(
    ordered_txs: &mut BTreeSet<OrderedTx>,
    pending_txs: &FxHashMap<TxHash, Arc<ValidPoolTransaction<P::Transaction>>>,
    published: &Sender<Option<Vec<Bytes>>>,
) {
    info!("building new inclusion list");

    let mut inclusion_list = vec![];
    let mut to_add = vec![];
    let mut total_encoded_size = 0;

    while let Some(ordered_tx) = ordered_txs.pop_first() {
        if let Some(transaction) = pending_txs.get(&ordered_tx.tx_hash) {
            to_add.push(ordered_tx); // will add back to the ordered txs.
            if total_encoded_size + transaction.encoded_length() > MAX_LIST_BYTES {
                continue;
            }
            total_encoded_size += transaction.encoded_length();
            let rlp_encoded_tx = transaction.to_consensus().into_encoded();
            inclusion_list.push(rlp_encoded_tx.into_encoded_bytes());
        }
    }
    let _ = published.send(Some(inclusion_list));
    ordered_txs.extend(to_add);
}

#[derive(Debug)]
struct OrderedTx {
    score: f64,
    tx_hash: TxHash,
}

impl PartialEq for OrderedTx {
    fn eq(&self, other: &Self) -> bool {
        self.score == other.score
    }
}

impl Eq for OrderedTx {}

impl PartialOrd for OrderedTx {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for OrderedTx {
    fn cmp(&self, other: &Self) -> Ordering {
        // Reverse ordering b/c highest scores come first.
        self.score.total_cmp(&other.score).reverse()
    }
}

fn score_tx<T: PoolTransaction>(tx: &Arc<ValidPoolTransaction<T>>) -> f64 {
    let tx_added_ts = tx.timestamp;
    let tx_gas_limit = tx.gas_limit();
    let tx_priority_fee = tx.priority_fee_or_price();

    (tx_added_ts.elapsed().as_micros() as f64 * tx_priority_fee as f64) / tx_gas_limit as f64
}
