use ethgas_reth_flashblocks::{
    service::FlashblocksSubscriber,
    state::FlashblocksState,
};
use futures_util::TryStreamExt;
use once_cell::sync::OnceCell;
use reth::{
    builder::NodeHandle,
    chainspec::{ChainSpecProvider, EthereumChainSpecParser},
    cli::Cli,
    version::{RethCliVersionConsts, default_reth_version_metadata, try_init_version_metadata},
};
use reth_exex::ExExEvent;
use reth_node_ethereum::EthereumNode;
use std::sync::Arc;
use ethgas_reth_rpc::{EthApiExt, EthApiOverrideServer};

use clap::Parser;
use reth::{
    builder::{EngineNodeLauncher, Node, TreeConfig},
    providers::providers::BlockchainProvider,
};
use tracing::{debug, info};
use url::Url;

#[global_allocator]
static ALLOC: reth_cli_util::allocator::Allocator = reth_cli_util::allocator::new_allocator();

pub const NODE_RETH_CLIENT_VERSION: &str = concat!("ethgas-reth/v", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone, PartialEq, Eq, clap::Args)]
struct FlashblocksArgs {
    #[arg(long = "websocket-url", value_name = "WEBSOCKET_URL")]
    pub websocket_url: Option<String>,

    #[arg(
        long = "max-pending-blocks-depth",
        value_name = "MAX_PENDING_BLOCKS_DEPTH",
        default_value = "3"
    )]
    pub max_pending_blocks_depth: u64,
}

impl FlashblocksArgs {
    fn flashblocks_enabled(&self) -> bool {
        self.websocket_url.is_some()
    }
}

fn main() {
    let default_version_metadata = default_reth_version_metadata();
    try_init_version_metadata(RethCliVersionConsts {
        name_client: "ETHGAS-RETH".to_string().into(),
        cargo_pkg_version: format!(
            "{}/{}",
            default_version_metadata.cargo_pkg_version,
            env!("CARGO_PKG_VERSION")
        )
        .into(),
        p2p_client_version: format!(
            "{}/{}",
            default_version_metadata.p2p_client_version, NODE_RETH_CLIENT_VERSION
        )
        .into(),
        extra_data: format!("{}/{}", default_version_metadata.extra_data, NODE_RETH_CLIENT_VERSION)
            .into(),
        ..default_version_metadata
    })
    .expect("Unable to init version metadata");

    Cli::<EthereumChainSpecParser, FlashblocksArgs>::parse()
        .run(|builder, flashblocks_args| async move {
            info!(message = "starting ethgas-reth rpc node");

            let flashblocks_enabled = flashblocks_args.flashblocks_enabled();
            debug!("Flashblocks enabled: {}", flashblocks_enabled);
            let node = EthereumNode::default();

            let fb_cell: Arc<OnceCell<Arc<FlashblocksState<_>>>> = Arc::new(OnceCell::new());

            let NodeHandle { node: _node, node_exit_future } = builder
                .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
                .with_components(node.components_builder())
                .with_add_ons(node.add_ons())
                .on_component_initialized(move |_ctx| Ok(()))
                .install_exex_if(flashblocks_enabled, "flashblocks-canon", {
                    let fb_cell = fb_cell.clone();
                    move |mut ctx| async move {
                        let provider = ctx.provider().clone();
                        let chain_spec = provider.chain_spec();
                        let fb = fb_cell
                            .get_or_init(|| {
                                Arc::new(FlashblocksState::new(
                                    provider,
                                    chain_spec,
                                    flashblocks_args.max_pending_blocks_depth,
                                ))
                            })
                            .clone();

                        Ok(async move {
                            while let Some(note) = ctx.notifications.try_next().await? {
                                if let Some(committed) = note.committed_chain() {
                                    for b in committed.blocks_iter() {
                                        fb.on_canonical_block_received(b);
                                    }
                                    let _ = ctx.events.send(ExExEvent::FinishedHeight(
                                        committed.tip().num_hash(),
                                    ));
                                }
                            }
                            Ok(())
                        })
                    }
                })
                .extend_rpc_modules(move |ctx| {
                    if flashblocks_enabled {
                        info!(message = "Starting Flashblocks RPC");

                        let ws_url = Url::parse(
                            flashblocks_args
                                .websocket_url
                                .expect("WEBSOCKET_URL must be set when Flashblocks is enabled")
                                .as_str(),
                        )?;

                        let provider = ctx.provider().clone();
                        let chain_spec = provider.chain_spec();
                        let fb = fb_cell
                            .get_or_init(|| {
                                Arc::new(FlashblocksState::new(
                                    provider,
                                    chain_spec,
                                    flashblocks_args.max_pending_blocks_depth,
                                ))
                            })
                            .clone();
                        fb.start();

                        let mut flashblocks_client = FlashblocksSubscriber::new(fb.clone(), ws_url);
                        flashblocks_client.start();

                        let api_ext = EthApiExt::new(
                            ctx.registry.eth_api().clone(),
                            ctx.registry.eth_handlers().filter.clone(),
                            fb,
                        );
                        ctx.modules.replace_configured(api_ext.into_rpc())?;
                    } else {
                        info!(message = "flashblocks integration is disabled");
                    }
                    Ok(())
                })
                .launch_with_fn(|builder| {
                    let engine_tree_config = TreeConfig::default()
                        .with_persistence_threshold(builder.config().engine.persistence_threshold)
                        .with_memory_block_buffer_target(
                            builder.config().engine.memory_block_buffer_target,
                        );

                    let launcher = EngineNodeLauncher::new(
                        builder.task_executor().clone(),
                        builder.config().datadir(),
                        engine_tree_config,
                    );

                    builder.launch_with(launcher)
                })
                .await?;

            node_exit_future.await
        })
        .unwrap();
}
