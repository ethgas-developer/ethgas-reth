//! Wrapper around the OP node builder that accumulates hooks instead of replacing them.

use std::fmt;

use eyre::Result;
use reth_node_builder::{
    NodeAdapter, NodeComponentsBuilder,
    node::FullNode,
    rpc::{RethRpcAddOns, RpcContext},
};
use reth_node_ethereum::EthereumNode;

use crate::types::{EthAddOns, EthBuilder, EthComponentsBuilder, EthNodeTypes};

/// Convenience alias for the OP node adapter type used by the reth builder.
pub(crate) type EthNodeAdapter =
    NodeAdapter<EthNodeTypes, <EthComponentsBuilder as NodeComponentsBuilder<EthNodeTypes>>::Components>;

/// Convenience alias for the OP Eth API type exposed by the reth RPC add-ons.
type OpEthApi = <EthAddOns as RethRpcAddOns<EthNodeAdapter>>::EthApi;

/// Convenience alias for the full OP node handle produced after launch.
type EthFullNode = FullNode<EthNodeAdapter, EthAddOns>;

/// Alias for the RPC context used by Ethgas extensions.
pub type EthgasRpcContext<'a> = RpcContext<'a, EthNodeAdapter, OpEthApi>;

/// Hook type for extending RPC modules.
type RpcModuleHook = Box<dyn FnMut(&mut EthgasRpcContext<'_>) -> Result<()> + Send + 'static>;

/// Hook type for node-started callbacks.
type NodeStartedHook = Box<dyn FnMut(EthFullNode) -> Result<()> + Send + 'static>;

/// A thin wrapper over [`OpBuilder`] that accumulates RPC and node-start hooks.
pub struct EthgasBuilder {
    builder: EthBuilder,
    rpc_hooks: Vec<RpcModuleHook>,
    node_started_hooks: Vec<NodeStartedHook>,
}

impl EthgasBuilder {
    /// Create a new EthgasBuilder wrapping the provided OP builder.
    pub const fn new(builder: EthBuilder) -> Self {
        Self { builder, rpc_hooks: Vec::new(), node_started_hooks: Vec::new() }
    }

    /// Consumes the wrapper and returns the inner builder after installing the accumulated hooks.
    pub fn build(self) -> EthBuilder {
        let Self { mut builder, mut rpc_hooks, node_started_hooks } = self;

        if !rpc_hooks.is_empty() {
            builder = builder.extend_rpc_modules(move |mut ctx: EthgasRpcContext<'_>| {
                for hook in rpc_hooks.iter_mut() {
                    hook(&mut ctx)?;
                }

                Ok(())
            });
        }

        if !node_started_hooks.is_empty() {
            builder = builder.on_node_started(move |full_node: EthFullNode| {
                let mut hooks = node_started_hooks;
                for hook in hooks.iter_mut() {
                    hook(full_node.clone())?;
                }
                Ok(())
            });
        }

        builder
    }

    /// Adds an RPC hook that will run when RPC modules are configured.
    pub fn add_rpc_module<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(&mut EthgasRpcContext<'_>) -> Result<()> + Send + 'static,
    {
        let mut hook = Some(hook);
        self.rpc_hooks.push(Box::new(move |ctx| {
            if let Some(hook) = hook.take() {
                hook(ctx)?;
            }
            Ok(())
        }));
        self
    }

    /// Adds a node-started hook that will run after the node has started.
    pub fn add_node_started_hook<F>(mut self, hook: F) -> Self
    where
        F: FnOnce(EthFullNode) -> Result<()> + Send + 'static,
    {
        let mut hook = Some(hook);
        self.node_started_hooks.push(Box::new(move |node| {
            if let Some(hook) = hook.take() {
                hook(node)?;
            }
            Ok(())
        }));
        self
    }

    /// Launches the node after applying accumulated hooks, delegating to the provided closure.
    pub fn launch_with_fn<L, R>(self, launcher: L) -> R
    where
        L: FnOnce(EthBuilder) -> R,
    {
        launcher(self.build())
    }

    /// Maps the add-ons with the given closure.
    pub fn map_add_ons<F>(mut self, f: F) -> Self
    where
        F: FnOnce(EthAddOns) -> EthAddOns,
    {
        self.builder = self.builder.map_add_ons(f);
        self
    }
}

impl fmt::Debug for EthgasBuilder {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EthgasBuilder").finish_non_exhaustive()
    }
}
