//! Contains the [`EthgasNodeRunner`], which is responsible for configuring and launching a Ethgas node.

use eyre::Result;
use reth_node_builder::{EngineNodeLauncher, Node, NodeHandleFor, TreeConfig};
use reth_node_ethereum::EthereumNode;
use reth_provider::providers::BlockchainProvider;
use tracing::info;

use crate::{builder::EthgasBuilder, extension::{EthgasNodeExtension, FromExtensionConfig}, handle::EthgasNodeHandle, types::EthgasNodeBuilder};


/// Wraps the Ethgas node configuration and orchestrates builder wiring.
#[derive(Debug)]
pub struct EthgasNodeRunner {
    /// Registered builder extensions.
    extensions: Vec<Box<dyn EthgasNodeExtension>>,
}

impl EthgasNodeRunner {
    pub fn new() -> Self {
        Self { extensions: Vec::new() }
    }

    /// Registers a new builder extension.
    pub fn install_ext<T: FromExtensionConfig + 'static>(&mut self, config: T::Config) {
        self.extensions.push(Box::new(T::from_config(config)));
    }

    /// Applies all Ethgas-specific wiring to the supplied builder, launches the node, and returns a
    /// handle that can be awaited.
    pub fn run(self, builder: EthgasNodeBuilder) -> EthgasNodeHandle {
        let Self { extensions } = self;
        EthgasNodeHandle::new(Self::launch_node( extensions, builder))
    }

    async fn launch_node(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        builder: EthgasNodeBuilder,
    ) -> Result<NodeHandleFor<EthereumNode>> {
        info!(target: "Ethgas-runner", "starting custom Ethgas node");

        let ethgas_node = EthereumNode::default();

        let builder = builder
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(ethgas_node.components_builder())
            .with_add_ons(ethgas_node.add_ons())
            .on_component_initialized(move |_ctx| Ok(()));

        let builder = extensions
            .into_iter()
            .fold(EthgasBuilder::new(builder), |builder, extension| extension.apply(builder));

        builder
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
