use std::sync::Arc;

use reth::{chainspec::ChainSpec, providers::providers::BlockchainProvider};
use reth_db::DatabaseEnv;
use reth_node_builder::{FullNodeTypesAdapter, Node, NodeBuilder, NodeBuilderWithComponents, NodeTypesWithDBAdapter, WithLaunchContext};
use reth_node_ethereum::EthereumNode;

/// Internal alias for the eth node type adapter.
pub type EthNodeTypes = FullNodeTypesAdapter<
    EthereumNode,
    Arc<DatabaseEnv>,
    BlockchainProvider<NodeTypesWithDBAdapter<EthereumNode, Arc<DatabaseEnv>>>,
>;
/// Internal alias for the eth node components builder.
pub(crate) type EthComponentsBuilder = <EthereumNode as Node<EthNodeTypes>>::ComponentsBuilder;
/// Internal alias for the eth node add-ons.
pub(crate) type EthAddOns = <EthereumNode as Node<EthNodeTypes>>::AddOns;


pub type EthBuilder =
    WithLaunchContext<NodeBuilderWithComponents<EthNodeTypes, EthComponentsBuilder, EthAddOns>>;


pub type EthgasNodeBuilder = WithLaunchContext<NodeBuilder<Arc<DatabaseEnv>, ChainSpec>>;