//! Metrics for flashblocks.

use metrics::{Counter, Gauge, Histogram};
use metrics_derive::Metrics;

/// Metrics for the `reth_flashblocks` component.
/// Conventions:
/// - Durations are recorded in seconds (histograms).
/// - Counters are monotonic event counts.
/// - Gauges reflect the current value/state.
#[derive(Metrics, Clone)]
#[metrics(scope = "reth_flashblocks")]
pub struct Metrics {
    /// Count of times upstream receiver was closed/errored.
    #[metric(describe = "Count of times upstream receiver was closed/errored")]
    pub upstream_errors: Counter,

    /// Count of messages received from the upstream source.
    #[metric(describe = "Count of messages received from the upstream source")]
    pub upstream_messages: Counter,

    /// Time taken to process a message.
    #[metric(describe = "Time taken to process a message")]
    pub block_processing_duration: Histogram,

    /// Time spent on parallel sender recovery (ECDSA operations).
    #[metric(describe = "Time spent on parallel sender recovery")]
    pub sender_recovery_duration: Histogram,

    /// Number of Flashblocks that arrive in an unexpected order.
    #[metric(describe = "Number of Flashblocks that arrive in an unexpected order")]
    pub unexpected_block_order: Counter,

    /// Number of flashblocks contained within a single block.
    #[metric(describe = "Number of flashblocks in a block")]
    pub flashblocks_in_block: Histogram,

    /// Count of times flashblocks are unable to be converted to blocks.
    #[metric(describe = "Count of times flashblocks are unable to be converted to blocks")]
    pub block_processing_error: Counter,

    /// Count of times pending snapshot was cleared because canonical caught up.
    #[metric(
        describe = "Number of times pending snapshot was cleared because canonical caught up"
    )]
    pub pending_clear_catchup: Counter,

    /// Number of times pending snapshot was cleared because of reorg.
    #[metric(describe = "Number of times pending snapshot was cleared because of reorg")]
    pub pending_clear_reorg: Counter,

    /// Pending snapshot flashblock index (current).
    #[metric(describe = "Pending snapshot flashblock index (current)")]
    pub pending_snapshot_fb_index: Gauge,

    /// Pending snapshot block number (current).
    #[metric(describe = "Pending snapshot block number (current)")]
    pub pending_snapshot_height: Gauge,

    /// Total number of WebSocket reconnection attempts.
    #[metric(describe = "Total number of WebSocket reconnection attempts")]
    pub reconnect_attempts: Counter,

    // RPC metrics
    /// Count of times flashblocks `get_transaction_count` is called.
    #[metric(describe = "Count of times flashblocks get_transaction_count is called")]
    pub rpc_get_transaction_count: Counter,

    /// Count of times flashblocks `get_transaction_receipt` is called.
    #[metric(describe = "Count of times flashblocks get_transaction_receipt is called")]
    pub rpc_get_transaction_receipt: Counter,

    /// Count of times flashblocks `get_transaction_by_hash` is called.
    #[metric(describe = "Count of times flashblocks get_transaction_by_hash is called")]
    pub rpc_get_transaction_by_hash: Counter,

    /// Count of times flashblocks `get_balance` is called.
    #[metric(describe = "Count of times flashblocks get_balance is called")]
    pub rpc_get_balance: Counter,

    /// Count of times flashblocks `get_block_by_number` is called.
    #[metric(describe = "Count of times flashblocks get_block_by_number is called")]
    pub rpc_get_block_by_number: Counter,

    /// Count of times flashblocks call is called.
    #[metric(describe = "Count of times flashblocks call is called")]
    pub rpc_call: Counter,

    /// Count of times flashblocks `estimate_gas` is called.
    #[metric(describe = "Count of times flashblocks estimate_gas is called")]
    pub rpc_estimate_gas: Counter,

    /// Count of times flashblocks `simulate_v1` is called.
    #[metric(describe = "Count of times flashblocks simulate_v1 is called")]
    pub rpc_simulate_v1: Counter,

    /// Count of times flashblocks `get_logs` is called.
    #[metric(describe = "Count of times flashblocks get_logs is called")]
    pub rpc_get_logs: Counter,

    /// Count of times flashblocks `get_block_transaction_count_by_number` is called.
    #[metric(
        describe = "Count of times flashblocks get_block_transaction_count_by_number is called"
    )]
    pub rpc_get_block_transaction_count_by_number: Counter,

    /// Time taken to clone bundle state.
    #[metric(describe = "Time taken to clone bundle state")]
    pub bundle_state_clone_duration: Histogram,

    /// Size of bundle state being cloned (number of accounts).
    #[metric(describe = "Size of bundle state being cloned (number of accounts)")]
    pub bundle_state_clone_size: Histogram,

    // `build_pending_state` breakdown: splits per-flashblock latency into its components to
    // show whether cold disk reads or compute/cloning dominates.
    /// Total wall time of `build_pending_state`, end to end.
    #[metric(describe = "Total wall time of build_pending_state")]
    pub build_total_duration: Histogram,

    /// Time to open the canonical `StateProvider` and read the parent header.
    #[metric(describe = "Time to open the canonical state provider and parent header")]
    pub build_state_open_duration: Histogram,

    /// Time spent setting up the EVM env and applying pre-execution system calls (4788/2935).
    #[metric(describe = "Time spent on EVM env setup and pre-execution system calls")]
    pub build_evm_setup_duration: Histogram,

    /// Time spent executing newly-seen transactions (the `evm.transact` + commit path).
    #[metric(describe = "Time spent executing new transactions (evm.transact + commit)")]
    pub build_execution_duration: Histogram,

    /// Time building receipts and RPC tx objects for all txs (scales with the whole block,
    /// reused or not — not just the new delta).
    #[metric(describe = "Time spent building receipts and rpc tx objects for all txs")]
    pub build_receipt_duration: Histogram,

    /// Time spent merging transitions and taking the final bundle state.
    #[metric(describe = "Time spent merging transitions and taking the final bundle state")]
    pub build_bundle_finalize_duration: Histogram,

    /// Number of transactions actually executed in a `build_pending_state` call (the new delta).
    #[metric(describe = "Number of transactions executed in a build_pending_state call")]
    pub build_executed_transactions: Histogram,

    /// Total number of transactions iterated in a `build_pending_state` call (whole block).
    #[metric(describe = "Total number of transactions iterated in a build_pending_state call")]
    pub build_total_transactions: Histogram,
}
