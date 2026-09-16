//! Ethgas-specific CLI arguments layered onto reth's node command.

use std::time::Duration;

use ethgas_reth_flashblocks::FlashblocksConfig;
use url::Url;

/// CLI Arguments
#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
pub struct Args {
    /// A URL pointing to a secure websocket subscription that streams out flashblocks.
    ///
    /// If given, the flashblocks are received to build pending block. All request with "pending"
    /// block tag will use the pending state based on flashblocks.
    #[arg(long, alias = "websocket-url")]
    pub flashblocks_url: Option<Url>,

    /// The max pending blocks depth.
    #[arg(
        long = "max-pending-blocks-depth",
        value_name = "MAX_PENDING_BLOCKS_DEPTH",
        default_value = "3"
    )]
    pub max_pending_blocks_depth: u64,

    /// Interval between flashblocks upstream websocket ping frames.
    ///
    /// The same interval is the pong deadline: a reconnect is triggered when a ping goes
    /// unanswered for one interval.
    #[arg(
        long = "flashblocks.ping-interval",
        value_name = "FLASHBLOCKS_PING_INTERVAL",
        default_value = "30s",
        value_parser = humantime::parse_duration,
        requires = "flashblocks_url"
    )]
    pub flashblocks_ping_interval: Duration,
}

impl From<&Args> for Option<FlashblocksConfig> {
    fn from(args: &Args) -> Self {
        args.flashblocks_url.clone().map(|url| {
            FlashblocksConfig::new(url, args.max_pending_blocks_depth)
                .with_subscriber_ping_interval(args.flashblocks_ping_interval)
        })
    }
}
