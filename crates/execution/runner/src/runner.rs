//! Contains the [`EthgasNodeRunner`], which is responsible for configuring and launching an Ethgas
//! node.

use eyre::Result;
use reth_node_builder::{EngineNodeLauncher, Node, NodeHandle, NodeHandleFor, TreeConfig};
use reth_provider::providers::BlockchainProvider;
use reth_node_ethereum::EthereumNode;
use tracing::info;

use crate::{EthgasNodeExtension, FromExtensionConfig, NodeHooks, types::EthgasNodeBuilder};

/// Wraps the Ethgas node configuration and orchestrates builder wiring.
#[derive(Debug)]
pub struct EthgasNodeRunner {
    /// Registered builder extensions.
    extensions: Vec<Box<dyn EthgasNodeExtension>>,
}

impl EthgasNodeRunner {
    /// Creates a new runner.
    pub fn new() -> Self {
        Self { extensions: Vec::new() }
    }

    /// Registers a new builder extension.
    pub fn install_ext<T: FromExtensionConfig + 'static>(&mut self, config: T::Config) {
        self.extensions.push(Box::new(T::from_config(config)));
    }

    /// Applies all Ethgas-specific wiring to the supplied builder, launches the node, and waits
    /// for shutdown.
    pub async fn run(self, builder: EthgasNodeBuilder) -> Result<()> {
        let Self { extensions } = self;
        let NodeHandle { node: _node, node_exit_future } =
            Self::launch_node(extensions, builder).await?;
        node_exit_future.await?;
        Ok(())
    }

    async fn launch_node(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        builder: EthgasNodeBuilder,
    ) -> Result<NodeHandleFor<EthereumNode>> {
        info!(target: "ethgas-runner", "starting custom Ethgas node");

        let ethgas_node = EthereumNode::default();

        let builder = builder
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(ethgas_node.components_builder())
            .with_add_ons(ethgas_node.add_ons())
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
                        builder.config().engine.memory_block_buffer_target,
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
