//! Contains the [`FlashblocksExtension`] which wires up the flashblocks feature
//! (canonical block subscription and RPC surface) on the Ethgas node builder.

use std::sync::Arc;

use ethgas_node_runner::{EthgasNodeExtension, FromExtensionConfig, NodeHooks};
use ethgas_reth_flashblocks::{
    EthApiExt, EthApiOverrideServer, EthPubSub, EthPubSubApiServer, FlashblocksConfig,
    FlashblocksSubscriber,
};
use reth_provider::CanonStateSubscriptions;
use tokio_stream::{StreamExt, wrappers::BroadcastStream};
use tracing::info;

/// Helper struct that wires the Flashblocks feature (canonical subscription and RPC) into the node
/// builder.
#[derive(Debug)]
pub struct FlashblocksExtension {
    /// Optional Flashblocks configuration (includes state).
    config: Option<FlashblocksConfig>,
}

impl FlashblocksExtension {
    /// Create a new Flashblocks extension helper.
    pub const fn new(config: Option<FlashblocksConfig>) -> Self {
        Self { config }
    }
}

impl EthgasNodeExtension for FlashblocksExtension {
    /// Applies the extension to the supplied hooks.
    fn apply(self: Box<Self>, hooks: NodeHooks) -> NodeHooks {
        let Some(cfg) = self.config else {
            info!(message = "flashblocks integration is disabled");
            return hooks;
        };

        let state = cfg.state;
        let mut subscriber = FlashblocksSubscriber::new(Arc::clone(&state), cfg.websocket_url);

        let state_for_canonical = Arc::clone(&state);
        let state_for_rpc = Arc::clone(&state);
        let state_for_start = state;

        // Start state processor, subscriber, and canonical subscription after node is started
        let hooks = hooks.add_node_started_hook(move |ctx| {
            info!(message = "Starting Flashblocks state processor");
            state_for_start.start(ctx.provider().clone());
            subscriber.start();

            let mut canonical_stream =
                BroadcastStream::new(ctx.provider().subscribe_to_canonical_state());
            tokio::spawn(async move {
                while let Some(Ok(notification)) = canonical_stream.next().await {
                    let committed = notification.committed();
                    for block in committed.blocks_iter() {
                        state_for_canonical.on_canonical_block_received(block);
                    }
                }
            });

            Ok(())
        });

        // Extend with RPC modules
        hooks.add_rpc_module(move |ctx| {
            info!(message = "Starting Flashblocks RPC");

            let api_ext = EthApiExt::new(
                ctx.registry.eth_api().clone(),
                ctx.registry.eth_handlers().filter.clone(),
                Arc::clone(&state_for_rpc),
            );
            ctx.modules.replace_configured(api_ext.into_rpc())?;

            // Register the flashblocks-aware `eth_subscribe` endpoint. Uses `replace_configured`
            // because `eth_subscribe` already exists from reth's standard module; standard
            // subscription kinds are proxied to reth's `EthPubSub` via the passed `eth_api`.
            let eth_pubsub = EthPubSub::new(
                ctx.registry.eth_api().clone(),
                ctx.node().task_executor.clone(),
                state_for_rpc,
            );
            ctx.modules.replace_configured(eth_pubsub.into_rpc())?;

            Ok(())
        })
    }
}

impl FromExtensionConfig for FlashblocksExtension {
    type Config = Option<FlashblocksConfig>;

    fn from_config(config: Self::Config) -> Self {
        Self::new(config)
    }
}
