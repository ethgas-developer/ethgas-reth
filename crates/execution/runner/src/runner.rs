//! Contains the [`EthgasNodeRunner`], which is responsible for configuring and launching an Ethgas
//! node.

use eyre::Result;
use reth_node_builder::{EngineNodeLauncher, Node, NodeHandle, TreeConfig};
use reth_node_ethereum::{
    EthereumNode,
    node::{EthereumAddOns, EthereumEngineValidatorBuilder},
};
use reth_provider::providers::BlockchainProvider;
use tracing::info;

use reth_node_builder::rpc::{
    BasicEngineApiBuilder, BasicEngineValidatorBuilder, Identity, RpcAddOns,
};

use std::sync::Arc;

use crate::{
    EthgasNodeExtension, FromExtensionConfig, NodeHooks, PendingStateSource,
    builder::EthNodeAdapter,
    eth_api::EthgasEthApiBuilder,
    types::{EthAddOns, EthgasNodeBuilder},
};

/// Wraps the Ethgas node configuration and orchestrates builder wiring.
#[derive(Debug)]
pub struct EthgasNodeRunner {
    /// Registered builder extensions.
    extensions: Vec<Box<dyn EthgasNodeExtension>>,
    /// Supplies the pending state the `eth` API answers the `pending` tag from.
    pending_state: Option<Arc<dyn PendingStateSource>>,
}

impl EthgasNodeRunner {
    /// Creates a new runner.
    pub fn new() -> Self {
        Self { extensions: Vec::new(), pending_state: None }
    }

    /// Sets the source the `eth` API answers the `pending` tag from.
    ///
    /// Without it, `pending` resolves to the canonical tip for every method the node does not
    /// override
    pub fn set_pending_state(&mut self, pending_state: Option<Arc<dyn PendingStateSource>>) {
        self.pending_state = pending_state;
    }

    /// Registers a new builder extension.
    pub fn install_ext<T: FromExtensionConfig + 'static>(&mut self, config: T::Config) {
        self.extensions.push(Box::new(T::from_config(config)));
    }

    /// Applies all Ethgas-specific wiring to the supplied builder, launches the node, and waits
    /// for shutdown.
    pub async fn run(self, builder: EthgasNodeBuilder) -> Result<()> {
        let Self { extensions, pending_state } = self;
        let NodeHandle { node: _node, node_exit_future } =
            Self::launch_node(extensions, pending_state, builder).await?;
        node_exit_future.await?;
        Ok(())
    }

    async fn launch_node(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        pending_state: Option<Arc<dyn PendingStateSource>>,
        builder: EthgasNodeBuilder,
    ) -> Result<NodeHandle<EthNodeAdapter, EthAddOns>> {
        info!(target: "ethgas-runner", "starting custom Ethgas node");

        let ethgas_node = EthereumNode::default();

        let builder = builder
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(ethgas_node.components_builder())
            .with_add_ons(EthereumAddOns::new(RpcAddOns::new(
                EthgasEthApiBuilder::new(pending_state),
                EthereumEngineValidatorBuilder::default(),
                BasicEngineApiBuilder::default(),
                BasicEngineValidatorBuilder::default(),
                Default::default(),
                Identity::new(),
            )))
            .on_component_initialized(move |_ctx| Ok(()));

        extensions
            .into_iter()
            .fold(NodeHooks::new(), |hooks, ext| ext.apply(hooks))
            .add_node_started_hook(|_| {
                ethgas_cli_utils::register_version_metrics!();
                Ok(())
            })
            .apply_to(builder)
            .launch_with_fn(|builder| {
                let engine_tree_config = TreeConfig::default()
                    .with_persistence_threshold(builder.config().engine.persistence_threshold)
                    .with_memory_block_buffer_target(
                        builder.config().engine.memory_block_buffer_target(),
                    );

                let launcher = EngineNodeLauncher::new(
                    builder.task_executor().clone(),
                    builder.config().datadir(),
                    engine_tree_config,
                );

                builder.launch_with(launcher)
            })
            .await
    }
}

impl Default for EthgasNodeRunner {
    fn default() -> Self {
        Self::new()
    }
}
