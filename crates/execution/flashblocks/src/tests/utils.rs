use std::sync::Arc;

use reth::api::NodeTypesWithDBAdapter;
use reth_db::{DatabaseEnv, test_utils::TempDatabase};
use reth_provider::{
    ProviderFactory,
    providers::NodeTypesForProvider,
    test_utils::create_test_provider_factory_with_node_types,
};

pub fn create_test_provider_factory<N: NodeTypesForProvider>(
    chain_spec: Arc<N::ChainSpec>,
) -> ProviderFactory<NodeTypesWithDBAdapter<N, Arc<TempDatabase<DatabaseEnv>>>> {
    create_test_provider_factory_with_node_types::<N>(chain_spec)
}
