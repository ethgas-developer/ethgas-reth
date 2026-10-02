//! Contains the [`EthgasNodeRunner`], which is responsible for configuring and launching an Ethgas
//! node.

use eyre::Result;
use reth_node_builder::{Node, NodeHandle};
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
        let send_raw_transaction_sync_timeout =
            builder.config().rpc.rpc_send_raw_transaction_sync_timeout;

        let builder = builder
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(ethgas_node.components_builder())
            .with_add_ons(EthereumAddOns::new(RpcAddOns::new(
                EthgasEthApiBuilder::new(pending_state)
                    .with_send_raw_transaction_sync_timeout(send_raw_transaction_sync_timeout),
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
            // As `reth node` launches: every `--engine.*` flag reaches the engine tree, and
            // `--dev` mines blocks without a consensus client.
            .launch_with_debug_capabilities()
            .await
    }
}

impl Default for EthgasNodeRunner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        time::{Duration, Instant},
    };

    use reth_chainspec::ChainSpec;
    use reth_db::{ClientVersion, init_db, mdbx::DatabaseArguments, test_utils::tempdir_path};
    use reth_node_builder::{NodeBuilder, NodeConfig};
    use reth_node_core::args::DatadirArgs;
    use reth_provider::{BlockNumReader, HeaderProvider};
    use reth_tasks::Runtime;

    use super::*;
    use crate::test_utils::build_test_genesis_with_amsterdam_at;

    #[tokio::test]
    async fn dev_mode_mines_amsterdam_blocks_without_a_consensus_client() -> Result<()> {
        let chain_spec = Arc::new(ChainSpec::from(build_test_genesis_with_amsterdam_at(Some(0))));
        let datadir = tempdir_path();
        let mut config =
            NodeConfig::new(chain_spec).dev().with_unused_ports().with_datadir_args(DatadirArgs {
                datadir: datadir.clone().into(),
                ..Default::default()
            });
        config.dev.block_time = Some(Duration::from_millis(100));
        let db = init_db(&datadir, DatabaseArguments::new(ClientVersion::default()))?;
        let builder =
            NodeBuilder::new(config).with_database(db).with_launch_context(Runtime::test());

        let NodeHandle { node, .. } = EthgasNodeRunner::launch_node(vec![], None, builder).await?;
        let deadline = Instant::now() + Duration::from_secs(10);
        while node.provider.best_block_number()? < 2 {
            assert!(Instant::now() < deadline, "no block was mined");
            tokio::time::sleep(Duration::from_millis(50)).await;
        }

        // reth's dev miner gives every Amsterdam block slot 0.
        let header = node.provider.header_by_number(1)?.expect("block 1 was mined");
        assert_eq!(header.slot_number, Some(0));
        assert!(header.block_access_list_hash.is_some());
        let _ = fs::remove_dir_all(datadir);
        Ok(())
    }
}
