//! Configuration for the flashblocks feature.

use std::{sync::Arc, time::Duration};

use url::Url;

use crate::FlashblocksState;

pub const DEFAULT_INCLUSION_FEE_MAX_AGE: Duration = Duration::from_secs(3);

/// Flashblocks-specific configuration knobs.
#[derive(Debug, Clone)]
pub struct FlashblocksConfig {
    /// The websocket endpoint that streams flashblock updates.
    pub websocket_url: Url,
    /// Maximum number of pending flashblocks to retain in memory.
    pub max_pending_blocks_depth: u64,
    /// Interval between upstream websocket ping frames.
    pub subscriber_ping_interval: Duration,
    /// Age after which a received inclusion fee gives way to the node's oracle.
    pub inclusion_fee_max_age: Duration,
    /// Shared Flashblocks state.
    pub state: Arc<FlashblocksState>,
}

impl FlashblocksConfig {
    /// Create a new Flashblocks configuration.
    pub fn new(websocket_url: Url, max_pending_blocks_depth: u64) -> Self {
        let state = Arc::new(FlashblocksState::new(max_pending_blocks_depth));
        Self {
            websocket_url,
            max_pending_blocks_depth,
            subscriber_ping_interval: Duration::from_secs(30),
            inclusion_fee_max_age: DEFAULT_INCLUSION_FEE_MAX_AGE,
            state,
        }
    }

    /// Set the maximum age of a received inclusion fee.
    pub fn with_inclusion_fee_max_age(mut self, inclusion_fee_max_age: Duration) -> Self {
        assert!(!inclusion_fee_max_age.is_zero(), "fee inclusion_fee max age must be positive");
        self.inclusion_fee_max_age = inclusion_fee_max_age;
        self
    }

    /// Set the interval between upstream websocket ping frames.
    pub fn with_subscriber_ping_interval(mut self, subscriber_ping_interval: Duration) -> Self {
        assert!(!subscriber_ping_interval.is_zero(), "ping interval must be positive");
        self.subscriber_ping_interval = subscriber_ping_interval;
        self
    }
}
