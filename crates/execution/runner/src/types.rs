//! Type aliases for the Ethereum node builder.

use reth::builder::{
    FullNodeTypesAdapter, Node, NodeBuilder, NodeComponentsBuilder, NodeTypesWithDBAdapter,
    WithLaunchContext,
};
use reth::chainspec::ChainSpec;
use reth::providers::providers::BlockchainProvider;
use reth_node_ethereum::EthereumNode;

/// The database environment type used by the node.
type DatabaseEnv = reth_db::DatabaseEnv;

/// Alias for the Ethereum node type adapter used by the runner.
pub type EthNodeTypes =
    FullNodeTypesAdapter<EthereumNode, DatabaseEnv, EthProvider>;

/// A [`BlockchainProvider`] instance.
pub type EthProvider =
    BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, DatabaseEnv>>;

/// Internal alias for the Ethereum node components builder.
pub(crate) type EthComponentsBuilder =
    <EthereumNode as Node<EthNodeTypes>>::ComponentsBuilder;

/// Internal alias for the Ethereum node add-ons.
pub(crate) type EthAddOns = <EthereumNode as Node<EthNodeTypes>>::AddOns;

/// Internal alias for the Ethereum components type.
pub(crate) type EthComponents =
    <EthComponentsBuilder as NodeComponentsBuilder<EthNodeTypes>>::Components;

/// Convenience alias for the Ethereum node builder type.
pub type EthgasNodeBuilder = WithLaunchContext<NodeBuilder<DatabaseEnv, ChainSpec>>;
