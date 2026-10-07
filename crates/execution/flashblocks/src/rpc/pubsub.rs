//! `eth_` `PubSub` RPC extension for flashblocks and standard subscriptions
//!
//! This module provides an extended `eth_subscribe` implementation that supports both
//! standard Ethereum subscription types (newHeads, logs, newPendingTransactions, syncing)
//! and flashblocks-specific subscriptions (newFlashblocks, pendingLogs, newFlashblockTransactions).

use std::{
    collections::HashMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use alloy_primitives::B256;
use alloy_rpc_types_eth::{Filter, Log, pubsub::Params};
use futures::stream;
use jsonrpsee::{
    ConnectionId, PendingSubscriptionSink, SubscriptionSink,
    core::{SubscriptionResult, async_trait},
    proc_macros::rpc,
    server::SubscriptionMessage,
};
use jsonrpsee_types::{ErrorObject, error::TOO_MANY_SUBSCRIPTIONS_CODE};
use reth_rpc::eth::EthPubSub as RethEthPubSub;
use reth_rpc_eth_api::{
    EthApiTypes, RpcNodeCore, RpcTransaction, pubsub::EthPubSubApiServer as RethEthPubSubApiServer,
};
use reth_tasks::Runtime;
use serde::Serialize;
use tokio_stream::{Stream, StreamExt, wrappers::BroadcastStream};
use tracing::{debug, error};

use crate::{
    FlashblocksAPI, PendingBlocks, TransactionWithLogs,
    metrics::Metrics,
    rpc::types::{ExtendedSubscriptionKind, FlashblocksSubscriptionKind},
};

/// The most `pendingLogs` and `newFlashblockTransactions` subscriptions one connection may hold
/// together. Each subscription renders and queues its own copy of every message, so this bounds
/// the work and the memory one connection can cause.
pub const MAX_FLASHBLOCKS_SUBSCRIPTIONS_PER_CONNECTION: usize = 32;

/// The most `newFlashblocks` subscriptions one connection may hold. Every message of that kind is
/// the whole pending block, the largest message the node sends, and each one replaces the one
/// before, so a connection has no use for many.
pub const MAX_NEW_FLASHBLOCKS_SUBSCRIPTIONS_PER_CONNECTION: usize = 4;

/// How often a held-back subscription checks whether its connection has drained. Short, so that
/// a burst of more than `max_queued` messages to a client that reads goes out as fast as the
/// connection drains.
const DRAIN_POLL_INTERVAL: Duration = Duration::from_millis(1);

/// What a subscription does when its client does not read.
#[derive(Debug, Clone, Copy)]
struct SlowSubscriberLimits {
    /// Unwritten messages the connection may hold before a per-item subscription holds back.
    max_queued: usize,
    /// How long the subscription holds back before it ends.
    timeout: Duration,
}

impl Default for SlowSubscriberLimits {
    fn default() -> Self {
        Self { max_queued: 32, timeout: Duration::from_secs(10) }
    }
}

/// The cap a flashblocks subscription counts against.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotKind {
    /// `newFlashblocks`: every message is the whole pending block.
    NewFlashblocks,
    /// `pendingLogs` and `newFlashblockTransactions`: one message per log or transaction.
    PerItem,
}

impl SlotKind {
    const fn of(kind: &FlashblocksSubscriptionKind) -> Self {
        match kind {
            FlashblocksSubscriptionKind::NewFlashblocks => Self::NewFlashblocks,
            FlashblocksSubscriptionKind::PendingLogs |
            FlashblocksSubscriptionKind::NewFlashblockTransactions => Self::PerItem,
        }
    }

    const fn cap(self) -> usize {
        match self {
            Self::NewFlashblocks => MAX_NEW_FLASHBLOCKS_SUBSCRIPTIONS_PER_CONNECTION,
            Self::PerItem => MAX_FLASHBLOCKS_SUBSCRIPTIONS_PER_CONNECTION,
        }
    }

    const fn name(self) -> &'static str {
        match self {
            Self::NewFlashblocks => "newFlashblocks",
            Self::PerItem => "pendingLogs and newFlashblockTransactions",
        }
    }
}

/// The flashblocks subscriptions one connection holds, by the cap they count against.
#[derive(Debug, Default)]
struct HeldSubscriptions {
    new_flashblocks: usize,
    per_item: usize,
    /// Every message these subscriptions have queued on the connection, counted once queued.
    queued_total: Arc<AtomicU64>,
}

impl HeldSubscriptions {
    const fn count_mut(&mut self, kind: SlotKind) -> &mut usize {
        match kind {
            SlotKind::NewFlashblocks => &mut self.new_flashblocks,
            SlotKind::PerItem => &mut self.per_item,
        }
    }

    const fn is_empty(&self) -> bool {
        self.new_flashblocks == 0 && self.per_item == 0
    }
}

/// Flashblocks subscriptions held per connection.
type SubscriptionCounts = Arc<Mutex<HashMap<ConnectionId, HeldSubscriptions>>>;

/// One connection's slot for a flashblocks subscription, given back when the subscription ends.
#[derive(Debug)]
struct ConnectionSlot {
    counts: SubscriptionCounts,
    connection: ConnectionId,
    kind: SlotKind,
    queued_total: Arc<AtomicU64>,
}

impl ConnectionSlot {
    /// Takes a slot of `kind` on `connection`, or `None` when it holds the most it may.
    fn take(counts: &SubscriptionCounts, connection: ConnectionId, kind: SlotKind) -> Option<Self> {
        let mut held = counts.lock().unwrap_or_else(|err| err.into_inner());
        let subscriptions = held.entry(connection).or_default();
        let count = subscriptions.count_mut(kind);
        if *count >= kind.cap() {
            return None;
        }
        *count += 1;
        let queued_total = Arc::clone(&subscriptions.queued_total);
        Some(Self { counts: Arc::clone(counts), connection, kind, queued_total })
    }
}

impl Drop for ConnectionSlot {
    fn drop(&mut self) {
        let mut held = self.counts.lock().unwrap_or_else(|err| err.into_inner());
        if let Some(subscriptions) = held.get_mut(&self.connection) {
            *subscriptions.count_mut(self.kind) -= 1;
            if subscriptions.is_empty() {
                held.remove(&self.connection);
            }
        }
    }
}

/// Eth pub-sub RPC extension for flashblocks and standard subscriptions.
///
/// This trait defines the `eth_subscribe` and `eth_unsubscribe` methods that handle
/// both standard Ethereum subscriptions and flashblocks-specific subscriptions.
#[rpc(server, namespace = "eth")]
pub trait EthPubSubApi {
    /// Create an Eth subscription for the given kind.
    ///
    /// Supports standard subscription types (newHeads, logs, newPendingTransactions, syncing)
    /// as well as flashblocks-specific subscriptions (newFlashblocks, pendingLogs,
    /// newFlashblockTransactions).
    #[subscription(
        name = "subscribe" => "subscription",
        unsubscribe = "unsubscribe",
        item = serde_json::Value
    )]
    async fn subscribe(
        &self,
        kind: ExtendedSubscriptionKind,
        params: Option<Params>,
    ) -> SubscriptionResult;
}

/// `Eth` pubsub RPC implementation that extends reth's standard implementation
/// with flashblocks support.
///
/// This handles `eth_subscribe` RPC calls for both standard Ethereum subscriptions
/// and flashblocks-specific subscriptions.
#[derive(Clone, Debug)]
pub struct EthPubSub<Eth, FB> {
    /// Reth's standard `EthPubSub` for handling standard subscription types
    inner: RethEthPubSub<Eth>,
    /// Flashblocks state for accessing pending blocks stream
    flashblocks_state: Arc<FB>,
    subscriptions_per_connection: SubscriptionCounts,
    limits: SlowSubscriberLimits,
    metrics: Metrics,
}

impl<Eth, FB> EthPubSub<Eth, FB> {
    /// Creates a new instance with the given eth API and flashblocks state.
    ///
    /// `subscription_task_spawner` is the runtime reth's [`RethEthPubSub`] uses to spawn
    /// subscription handler tasks (added as a required argument in reth v2).
    pub fn new(
        eth_api: Eth,
        subscription_task_spawner: Runtime,
        flashblocks_state: Arc<FB>,
    ) -> Self {
        Self {
            inner: RethEthPubSub::new(eth_api, subscription_task_spawner),
            flashblocks_state,
            subscriptions_per_connection: SubscriptionCounts::default(),
            limits: SlowSubscriberLimits::default(),
            metrics: Metrics::default(),
        }
    }

    /// Counts an opened subscription by kind.
    fn count_subscription(&self, kind: &FlashblocksSubscriptionKind) {
        match kind {
            FlashblocksSubscriptionKind::NewFlashblocks => {
                self.metrics.subscriptions_new_flashblocks.increment(1)
            }
            FlashblocksSubscriptionKind::PendingLogs => {
                self.metrics.subscriptions_pending_logs.increment(1)
            }
            FlashblocksSubscriptionKind::NewFlashblockTransactions => {
                self.metrics.subscriptions_new_flashblock_transactions.increment(1)
            }
        }
    }

    /// Returns a stream that yields every published snapshot; the subscription renders its latest
    /// block only when it sends one.
    fn new_flashblocks_stream(flashblocks_state: Arc<FB>) -> impl Stream<Item = Arc<PendingBlocks>>
    where
        FB: FlashblocksAPI + Send + Sync + 'static,
    {
        BroadcastStream::new(flashblocks_state.subscribe_to_flashblocks()).filter_map(|result| {
            match result {
                Ok(pending_blocks) => Some(pending_blocks),
                Err(err) => {
                    error!(
                        message = "Error in flashblocks stream",
                        error = %err
                    );
                    None
                }
            }
        })
    }

    /// Returns a stream that yields individual logs from only the latest flashblock matching the
    /// filter.
    ///
    /// Each matching log is emitted as a separate stream item (one log per WebSocket message).
    /// Only logs from the most recent flashblock are emitted to avoid duplicates.
    fn pending_logs_stream(flashblocks_state: Arc<FB>, filter: Filter) -> impl Stream<Item = Log>
    where
        FB: FlashblocksAPI + Send + Sync + 'static,
    {
        futures::StreamExt::flat_map(
            StreamExt::filter_map(
                BroadcastStream::new(flashblocks_state.subscribe_to_flashblocks()),
                move |result| {
                    let pending_blocks = match result {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            error!(
                                message = "Error in flashblocks stream for pending logs",
                                error = %err
                            );
                            return None;
                        }
                    };
                    let logs = pending_blocks.get_latest_flashblock_logs(&filter);
                    if logs.is_empty() { None } else { Some(logs) }
                },
            ),
            stream::iter,
        )
    }

    /// Returns a stream that yields individual full transactions with logs from only the latest
    /// flashblock.
    ///
    /// Each transaction (with its associated logs) is emitted as a separate stream item
    /// (one transaction per WebSocket message). Only transactions from the most recent
    /// flashblock are emitted to avoid duplicates.
    fn new_flashblock_transactions_full_stream(
        flashblocks_state: Arc<FB>,
    ) -> impl Stream<Item = TransactionWithLogs>
    where
        FB: FlashblocksAPI + Send + Sync + 'static,
    {
        futures::StreamExt::flat_map(
            StreamExt::filter_map(
                BroadcastStream::new(flashblocks_state.subscribe_to_flashblocks()),
                |result| {
                    let pending_blocks = match result {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            error!(
                                message = "Error in flashblocks stream for transactions",
                                error = %err
                            );
                            return None;
                        }
                    };
                    let txs = pending_blocks.get_latest_flashblock_transactions_with_logs();
                    if txs.is_empty() { None } else { Some(txs) }
                },
            ),
            stream::iter,
        )
    }

    /// Returns a stream that yields full transactions with logs from only the latest flashblock,
    /// filtered to include only transactions where at least one log matches the filter.
    fn new_flashblock_transactions_filtered_stream(
        flashblocks_state: Arc<FB>,
        filter: Filter,
    ) -> impl Stream<Item = TransactionWithLogs>
    where
        FB: FlashblocksAPI + Send + Sync + 'static,
    {
        futures::StreamExt::flat_map(
            StreamExt::filter_map(
                BroadcastStream::new(flashblocks_state.subscribe_to_flashblocks()),
                move |result| {
                    let pending_blocks = match result {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            error!(
                                message = "Error in flashblocks stream for filtered transactions",
                                error = %err
                            );
                            return None;
                        }
                    };
                    let txs = pending_blocks
                        .get_latest_flashblock_transactions_with_logs_filtered(&filter);
                    if txs.is_empty() { None } else { Some(txs) }
                },
            ),
            stream::iter,
        )
    }

    /// Returns a stream that yields individual transaction hashes from only the latest flashblock.
    ///
    /// Each hash is emitted as a separate stream item (one hash per WebSocket message).
    /// Only hashes from the most recent flashblock are emitted to avoid duplicates.
    fn new_flashblock_transactions_hash_stream(
        flashblocks_state: Arc<FB>,
    ) -> impl Stream<Item = B256>
    where
        FB: FlashblocksAPI + Send + Sync + 'static,
    {
        futures::StreamExt::flat_map(
            StreamExt::filter_map(
                BroadcastStream::new(flashblocks_state.subscribe_to_flashblocks()),
                |result| {
                    let pending_blocks = match result {
                        Ok(blocks) => blocks,
                        Err(err) => {
                            error!(
                                message = "Error in flashblocks stream for transaction hashes",
                                error = %err
                            );
                            return None;
                        }
                    };
                    let hashes = pending_blocks.get_latest_flashblock_transaction_hashes();
                    if hashes.is_empty() { None } else { Some(hashes) }
                },
            ),
            stream::iter,
        )
    }
}

#[async_trait]
impl<Eth, FB> EthPubSubApiServer for EthPubSub<Eth, FB>
where
    Eth: RpcNodeCore + EthApiTypes + Clone + Send + Sync + 'static,
    RethEthPubSub<Eth>: RethEthPubSubApiServer<RpcTransaction<Eth::NetworkTypes>>,
    FB: FlashblocksAPI + Send + Sync + 'static,
{
    /// Handler for `eth_subscribe`
    ///
    /// Routes standard subscription types to reth's implementation and handles
    /// flashblocks subscriptions directly.
    async fn subscribe(
        &self,
        pending: PendingSubscriptionSink,
        kind: ExtendedSubscriptionKind,
        params: Option<Params>,
    ) -> SubscriptionResult {
        // For standard subscription types, delegate to reth's implementation
        if let Some(standard_kind) = kind.as_standard() {
            return RethEthPubSubApiServer::subscribe(&self.inner, pending, standard_kind, params)
                .await;
        }

        // Handle flashblocks-specific subscriptions
        let ExtendedSubscriptionKind::Flashblocks(fb_kind) = kind else {
            unreachable!("Standard subscription types should be delegated to inner");
        };

        let slot_kind = SlotKind::of(&fb_kind);
        let Some(slot) = ConnectionSlot::take(
            &self.subscriptions_per_connection,
            pending.connection_id(),
            slot_kind,
        ) else {
            pending
                .reject(ErrorObject::owned(
                    TOO_MANY_SUBSCRIPTIONS_CODE,
                    format!(
                        "too many {} subscriptions on this connection: max {}",
                        slot_kind.name(),
                        slot_kind.cap()
                    ),
                    None::<()>,
                ))
                .await;
            return Ok(());
        };

        let sink = pending.accept().await?;
        self.count_subscription(&fb_kind);
        let limits = self.limits;

        match fb_kind {
            FlashblocksSubscriptionKind::NewFlashblocks => {
                let stream = Self::new_flashblocks_stream(Arc::clone(&self.flashblocks_state));

                tokio::spawn(async move {
                    let render =
                        |pending_blocks: &Arc<PendingBlocks>| pending_blocks.get_latest_block(true);
                    pipe_latest(sink, stream, render, limits, &slot.queued_total).await;
                    drop(slot);
                });
            }
            FlashblocksSubscriptionKind::PendingLogs => {
                // Extract filter from params, default to empty filter (match all)
                let filter = match params {
                    Some(Params::Logs(filter)) => *filter,
                    _ => Filter::default(),
                };

                let stream = Self::pending_logs_stream(Arc::clone(&self.flashblocks_state), filter);

                tokio::spawn(async move {
                    pipe_from_stream(sink, stream, limits, &slot.queued_total).await;
                    drop(slot);
                });
            }
            FlashblocksSubscriptionKind::NewFlashblockTransactions => match params {
                Some(Params::Logs(filter)) => {
                    let stream = Self::new_flashblock_transactions_filtered_stream(
                        Arc::clone(&self.flashblocks_state),
                        *filter,
                    );
                    tokio::spawn(async move {
                        pipe_from_stream(sink, stream, limits, &slot.queued_total).await;
                        drop(slot);
                    });
                }
                Some(Params::Bool(true)) => {
                    let stream = Self::new_flashblock_transactions_full_stream(Arc::clone(
                        &self.flashblocks_state,
                    ));
                    tokio::spawn(async move {
                        pipe_from_stream(sink, stream, limits, &slot.queued_total).await;
                        drop(slot);
                    });
                }
                _ => {
                    let stream = Self::new_flashblock_transactions_hash_stream(Arc::clone(
                        &self.flashblocks_state,
                    ));
                    tokio::spawn(async move {
                        pipe_from_stream(sink, stream, limits, &slot.queued_total).await;
                        drop(slot);
                    });
                }
            },
        }

        Ok(())
    }
}

/// Messages the subscription's connection holds and has not written to the client yet. The buffer
/// is the connection's, so every subscription on it sees the same count.
fn queued(sink: &SubscriptionSink) -> usize {
    sink.max_capacity().saturating_sub(sink.capacity())
}

/// Waits until the connection holds at most `limits.max_queued` messages. `false` when it does not
/// drain within `limits.timeout`, or the subscription closes.
async fn wait_for_room(sink: &SubscriptionSink, limits: SlowSubscriberLimits) -> bool {
    let deadline = Instant::now() + limits.timeout;
    while queued(sink) > limits.max_queued {
        if sink.is_closed() || Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DRAIN_POLL_INTERVAL).await;
    }
    true
}

/// Serializes and sends one message, and counts it in the connection's `queued_total` once it is
/// queued. Returns the total with this message counted, or `None` when the subscription must end.
async fn send<T: Serialize>(
    sink: &SubscriptionSink,
    item: &T,
    queued_total: &AtomicU64,
) -> Option<u64> {
    let msg = match SubscriptionMessage::new(sink.method_name(), sink.subscription_id(), item) {
        Ok(msg) => msg,
        Err(err) => {
            error!(
                target: "flashblocks_rpc::pubsub",
                %err,
                "Failed to serialize subscription message"
            );
            return None;
        }
    };

    // An error means the client disconnected.
    sink.send(msg).await.ok()?;
    Some(queued_total.fetch_add(1, Ordering::AcqRel) + 1)
}

/// Whether a message has left the connection's buffer, given how many messages the buffer holds
/// and how many, at least, were queued after it. The buffer is first in, first out, so the message
/// has left once the buffer holds no more messages than came after it, empty or not.
const fn has_left(queued_now: usize, queued_after: u64) -> bool {
    queued_now as u64 <= queued_after
}

/// Pipes every stream item to the subscription sink.
///
/// Ends when the stream ends, the client disconnects, a message fails to serialize, or the client
/// stops reading: an item waits while the connection holds more than `limits.max_queued`
/// messages, and the subscription ends after `limits.timeout`, so a client that never reads cannot
/// make the node hold a full connection buffer of rendered messages.
async fn pipe_from_stream<T, St>(
    sink: SubscriptionSink,
    mut stream: St,
    limits: SlowSubscriberLimits,
    queued_total: &AtomicU64,
) where
    St: Stream<Item = T> + Unpin,
    T: Serialize,
{
    loop {
        let item = tokio::select! {
            _ = sink.closed() => return,
            maybe_item = stream.next() => match maybe_item {
                Some(item) => item,
                None => return,
            },
        };

        if !wait_for_room(&sink, limits).await {
            debug!(
                target: "flashblocks_rpc::pubsub",
                method = sink.method_name(),
                "client does not read, ending the subscription"
            );
            return;
        }

        if send(&sink, &item, queued_total).await.is_none() {
            return;
        }
    }
}

/// Pipes the newest stream item to the subscription sink, rendered with `render`.
///
/// For a stream whose every message replaces the one before, as `newFlashblocks` does: an item is
/// skipped, not rendered or queued, while the subscription's previous message is still in the
/// connection's buffer, so a client that does not read holds one message of the subscription.
/// Messages that the connection's other subscriptions queued after it do not hold it back. After
/// `limits.timeout` of skipping the subscription ends.
async fn pipe_latest<T, U, St, F>(
    sink: SubscriptionSink,
    mut stream: St,
    render: F,
    limits: SlowSubscriberLimits,
    queued_total: &AtomicU64,
) where
    St: Stream<Item = T> + Unpin,
    U: Serialize,
    F: Fn(&T) -> U,
{
    let mut skipping_since: Option<Instant> = None;
    // The connection's `queued_total` with the subscription's previous message counted.
    let mut previous_at: Option<u64> = None;
    loop {
        let item = tokio::select! {
            _ = sink.closed() => return,
            maybe_item = stream.next() => match maybe_item {
                Some(item) => item,
                None => return,
            },
        };

        // The total is read before the buffer, so a message queued between the two reads can
        // only make the buffer look fuller. Messages of the connection's other subscriptions are
        // not counted, which errs the same way.
        let previous_queued = previous_at.is_some_and(|at| {
            let queued_after = queued_total.load(Ordering::Acquire).saturating_sub(at);
            !has_left(queued(&sink), queued_after)
        });
        if previous_queued {
            let since = *skipping_since.get_or_insert_with(Instant::now);
            if since.elapsed() >= limits.timeout {
                debug!(
                    target: "flashblocks_rpc::pubsub",
                    method = sink.method_name(),
                    "client does not read, ending the subscription"
                );
                return;
            }
            continue;
        }
        skipping_since = None;

        let Some(queued_at) = send(&sink, &render(&item), queued_total).await else {
            return;
        };
        previous_at = Some(queued_at);
    }
}

#[cfg(test)]
mod tests {
    use jsonrpsee::{RpcModule, core::server::Subscription, rpc_params};
    use tokio::sync::mpsc;
    use tokio_stream::wrappers::ReceiverStream;

    use super::*;

    const LIMITS: SlowSubscriberLimits =
        SlowSubscriberLimits { max_queued: 2, timeout: Duration::from_millis(200) };

    /// A module whose one subscription, `sub`, pipes the items of `items`: through `pipe_latest`
    /// when `latest`, else through `pipe_from_stream`.
    fn module(
        items: mpsc::Receiver<u64>,
        latest: bool,
    ) -> RpcModule<Mutex<Option<ReceiverStream<u64>>>> {
        let mut module = RpcModule::new(Mutex::new(Some(ReceiverStream::new(items))));
        module
            .register_subscription(
                "sub",
                "sub_notification",
                "unsub",
                move |_, pending, ctx, _| async move {
                    let stream = ctx.lock().unwrap().take().expect("one subscription per module");
                    let sink = pending.accept().await?;
                    tokio::spawn(async move {
                        let queued_total = AtomicU64::new(0);
                        if latest {
                            pipe_latest(sink, stream, |item: &u64| *item, LIMITS, &queued_total)
                                .await;
                        } else {
                            pipe_from_stream(sink, stream, LIMITS, &queued_total).await;
                        }
                    });
                    SubscriptionResult::Ok(())
                },
            )
            .unwrap();
        module
    }

    /// The cumulative stream's items and the second sender's bursts, for the one subscription.
    type SharedInputs = Mutex<Option<(ReceiverStream<u64>, mpsc::Receiver<u64>)>>;

    /// A module whose one subscription pipes `items` through `pipe_latest`, while a second sender
    /// on the same connection queues as many messages of `u64::MAX` as each number of `bursts`
    /// says, counted in the same total, as the connection's per-item subscriptions do.
    fn shared_module(
        items: mpsc::Receiver<u64>,
        bursts: mpsc::Receiver<u64>,
    ) -> RpcModule<SharedInputs> {
        let mut module = RpcModule::new(Mutex::new(Some((ReceiverStream::new(items), bursts))));
        module
            .register_subscription(
                "sub",
                "sub_notification",
                "unsub",
                move |_, pending, ctx, _| async move {
                    let (stream, mut bursts) =
                        ctx.lock().unwrap().take().expect("one subscription per module");
                    let sink = pending.accept().await?;
                    let other = sink.clone();
                    let queued_total = Arc::new(AtomicU64::new(0));
                    let total = Arc::clone(&queued_total);
                    tokio::spawn(async move {
                        while let Some(count) = bursts.recv().await {
                            for _ in 0..count {
                                send(&other, &u64::MAX, &total).await;
                            }
                        }
                    });
                    tokio::spawn(async move {
                        pipe_latest(sink, stream, |item: &u64| *item, LIMITS, &queued_total).await;
                    });
                    SubscriptionResult::Ok(())
                },
            )
            .unwrap();
        module
    }

    /// Reads the messages the subscription holds now, until none arrives for 100 ms.
    async fn read_queued(subscription: &mut Subscription) -> Vec<u64> {
        let mut received = Vec::new();
        while let Ok(Some(Ok((item, _)))) =
            tokio::time::timeout(Duration::from_millis(100), subscription.next::<u64>()).await
        {
            received.push(item);
        }
        received
    }

    /// Reads every queued message until the subscription ends. Panics if it is still open.
    async fn read_until_closed(subscription: &mut Subscription) -> Vec<u64> {
        let mut received = Vec::new();
        loop {
            match tokio::time::timeout(Duration::from_secs(2), subscription.next::<u64>()).await {
                Ok(Some(Ok((item, _)))) => received.push(item),
                Ok(Some(Err(err))) => panic!("{err}"),
                Ok(None) => return received,
                Err(_) => panic!("the subscription is still open after {received:?}"),
            }
        }
    }

    /// A client that stops reading loses its subscription after the timeout, so the node holds no
    /// more than the connection's buffer for it.
    #[tokio::test]
    async fn a_subscription_whose_client_does_not_read_ends() {
        let (items, rx) = mpsc::channel(16);
        let module = module(rx, false);
        let mut subscription = module.subscribe("sub", rpc_params![], 4).await.unwrap();

        for item in 0..10 {
            items.send(item).await.unwrap();
        }
        tokio::time::sleep(LIMITS.timeout * 3).await;

        let received = read_until_closed(&mut subscription).await;
        assert!(received.len() < 10, "every item was queued: {received:?}");
    }

    /// A cumulative stream skips items while its last message is unread, then ends.
    #[tokio::test]
    async fn the_latest_stream_skips_while_the_connection_is_full_then_ends() {
        let (items, rx) = mpsc::channel(16);
        let module = module(rx, true);
        let mut subscription = module.subscribe("sub", rpc_params![], 4).await.unwrap();

        for item in 0..10 {
            items.send(item).await.unwrap();
        }
        tokio::time::sleep(LIMITS.timeout + Duration::from_millis(100)).await;
        items.send(10).await.unwrap();
        tokio::time::sleep(Duration::from_millis(100)).await;

        let received = read_until_closed(&mut subscription).await;
        assert!(received.len() < 10, "the skipped items were queued: {received:?}");
        assert_eq!(
            received,
            (0..received.len() as u64).collect::<Vec<_>>(),
            "in order, none skipped before the buffer filled"
        );
    }

    /// A cumulative stream queues a message only once its previous one has left the buffer, so a
    /// client that does not read holds one of its messages, and a client that reads gets the next.
    #[tokio::test]
    async fn the_latest_stream_waits_for_its_previous_message_to_leave_the_buffer() {
        let (items, rx) = mpsc::channel(16);
        let module = module(rx, true);
        let mut subscription = module.subscribe("sub", rpc_params![], 8).await.unwrap();

        for item in 0..6 {
            items.send(item).await.unwrap();
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(read_queued(&mut subscription).await, vec![0]);

        items.send(6).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(read_queued(&mut subscription).await, vec![6]);
    }

    /// A burst of another subscription's messages, queued after the previous message, does not
    /// hold a cumulative stream back, however long it is.
    #[tokio::test]
    async fn the_latest_stream_sends_beside_a_burst_of_later_messages() {
        let (items, rx) = mpsc::channel(16);
        let (bursts, bursts_rx) = mpsc::channel(16);
        let module = shared_module(rx, bursts_rx);
        let mut subscription = module.subscribe("sub", rpc_params![], 64).await.unwrap();

        items.send(0).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(read_queued(&mut subscription).await, vec![0]);

        bursts.send(10).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        items.send(1).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let received = read_queued(&mut subscription).await;
        assert_eq!(received.last(), Some(&1), "after the burst of ten: {received:?}");
    }

    /// Other subscriptions of the connection queue messages after the previous one; once the
    /// buffer holds only those, the previous message has been written.
    #[test]
    fn a_message_has_left_the_buffer_once_only_later_messages_remain() {
        assert!(has_left(0, 0));
        assert!(has_left(3, 3), "the three queued messages all came after it");
        assert!(!has_left(4, 3), "one queued message came before the three later ones");
    }
}
