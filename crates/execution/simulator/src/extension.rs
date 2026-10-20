//! Contains the [`SimulatorExtension`], which wires the block simulator's RPC surface into the
//! Ethgas node builder.

use std::sync::Arc;

use ethgas_node_runner::{EthgasNodeExtension, FromExtensionConfig, NodeHooks};
use reth_node_api::FullNodeComponents;
use reth_node_ethereum::EthereumEngineValidator;
use reth_provider::CanonStateSubscriptions;
use tokio::sync::watch;
use tracing::info;
use url::Url;

use crate::{
    BlockSubmissionValidationApiServer, InclusionExt, InclusionExtApiServer, ValidationApi,
    ValidationApiConfig, inclusion_producer,
};

/// Configuration of the block simulator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SimulatorConfig {
    /// The endpoint the address disallow list is fetched from.
    pub blacklist_provider: Url,
}

impl SimulatorConfig {
    /// Creates a simulator configuration.
    pub const fn new(blacklist_provider: Url) -> Self {
        Self { blacklist_provider }
    }
}

/// Wires the block simulator (the `relay` namespace's block validation and inclusion list
/// methods) into the node builder.
#[derive(Debug)]
pub struct SimulatorExtension {
    /// Simulator configuration. `None` disables the simulator.
    config: Option<SimulatorConfig>,
}

impl SimulatorExtension {
    /// Creates a new simulator extension.
    pub const fn new(config: Option<SimulatorConfig>) -> Self {
        Self { config }
    }
}

impl EthgasNodeExtension for SimulatorExtension {
    fn apply(self: Box<Self>, hooks: NodeHooks) -> NodeHooks {
        let Some(config) = self.config else {
            info!(message = "block simulator is disabled");
            return hooks;
        };

        hooks.add_rpc_module(move |ctx| {
            info!(message = "Starting block simulator RPC");

            let validation_api = ValidationApi::new(
                ctx.node().provider.clone(),
                Arc::new(ctx.node().consensus().clone()),
                ctx.node().evm_config().clone(),
                ValidationApiConfig::new(config.blacklist_provider.into()),
                ctx.node().task_executor.clone(),
                Arc::new(EthereumEngineValidator::new(ctx.config().chain.clone())),
            );
            ctx.modules.merge_configured(validation_api.into_rpc())?;

            let (publisher, published) = watch::channel(None);
            tokio::spawn(inclusion_producer(
                ctx.pool().clone(),
                ctx.provider().canonical_state_stream(),
                publisher,
            ));
            ctx.modules.merge_configured(InclusionExt { published }.into_rpc())?;

            Ok(())
        })
    }
}

impl FromExtensionConfig for SimulatorExtension {
    type Config = Option<SimulatorConfig>;

    fn from_config(config: Self::Config) -> Self {
        Self::new(config)
    }
}
