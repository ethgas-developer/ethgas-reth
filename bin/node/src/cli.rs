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
        default_value = "2s",
        value_parser = humantime::parse_duration,
        requires = "flashblocks_url"
    )]
    pub flashblocks_ping_interval: Duration,

    /// How long the builder's inclusion fee is served after its flashblock arrived.
    #[arg(
        long = "flashblocks.fee-max-age",
        value_name = "FLASHBLOCKS_FEE_MAX_AGE",
        default_value = "15s",
        value_parser = parse_positive_duration,
        requires = "flashblocks_url"
    )]
    pub flashblocks_fee_max_age: Duration,
}

fn parse_positive_duration(value: &str) -> Result<Duration, String> {
    let duration = humantime::parse_duration(value).map_err(|error| error.to_string())?;
    if duration.is_zero() {
        return Err("the duration must be positive".to_owned());
    }
    Ok(duration)
}

impl From<&Args> for Option<FlashblocksConfig> {
    fn from(args: &Args) -> Self {
        args.flashblocks_url.clone().map(|url| {
            FlashblocksConfig::new(url, args.max_pending_blocks_depth)
                .with_subscriber_ping_interval(args.flashblocks_ping_interval)
                .with_inclusion_fee_max_age(args.flashblocks_fee_max_age)
        })
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use ethgas_reth_flashblocks::config::DEFAULT_INCLUSION_FEE_MAX_AGE;

    use super::*;

    /// Wraps [`Args`] so the flattened flags can be parsed on their own, without reth's
    /// full node command.
    #[derive(Debug, Parser)]
    struct CommandParser<T: clap::Args> {
        #[command(flatten)]
        args: T,
    }

    fn parse(argv: &[&str]) -> Args {
        CommandParser::<Args>::parse_from(argv).args
    }

    #[test]
    fn ping_interval_defaults_to_2_seconds() {
        let args = parse(&["ethgas-node", "--flashblocks-url", "wss://example.com/ws"]);

        assert_eq!(args.flashblocks_ping_interval, Duration::from_secs(2));
    }

    /// `requires` must not fire for a value that came from `default_value`, or the node would
    /// refuse to start whenever flashblocks are disabled.
    #[test]
    fn ping_interval_default_does_not_require_flashblocks_url() {
        let args = CommandParser::<Args>::try_parse_from(["ethgas-node"])
            .expect("args should parse with flashblocks disabled")
            .args;

        assert_eq!(args.flashblocks_url, None);
        assert_eq!(args.flashblocks_ping_interval, Duration::from_secs(2));
    }

    #[test]
    fn explicit_ping_interval_requires_flashblocks_url() {
        let error = CommandParser::<Args>::try_parse_from([
            "ethgas-node",
            "--flashblocks.ping-interval",
            "45s",
        ])
        .expect_err("an explicit ping interval should require a flashblocks url");

        assert!(error.to_string().contains("--flashblocks-url"));
    }

    #[test]
    fn ping_interval_flows_into_config() {
        let args = parse(&[
            "ethgas-node",
            "--flashblocks-url",
            "wss://example.com/ws",
            "--flashblocks.ping-interval",
            "45s",
        ]);

        let config = Option::<FlashblocksConfig>::from(&args)
            .expect("a flashblocks url should produce a config");

        assert_eq!(config.subscriber_ping_interval, Duration::from_secs(45));
    }
}
