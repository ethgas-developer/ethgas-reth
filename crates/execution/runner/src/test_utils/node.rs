//! Local node setup for Ethereum integration testing.

use std::{any::Any, fmt, net::SocketAddr, path::PathBuf, sync::Arc};

use alloy_provider::RootProvider;
use alloy_rpc_client::RpcClient;
use eyre::Result;
use reth::chainspec::ChainSpec;
use reth_db::{
    ClientVersion, DatabaseEnv, init_db, mdbx::DatabaseArguments, test_utils::tempdir_path,
};
use reth_node_builder::{Node, NodeBuilder, NodeConfig, NodeHandle};
use reth_node_core::{
    args::{DatadirArgs, DiscoveryArgs, NetworkArgs, RpcServerArgs},
    dirs::{DataDirPath, MaybePlatformPath},
    exit::NodeExitFuture,
};
use reth_node_ethereum::EthereumNode;
use reth_provider::providers::BlockchainProvider;

use crate::{
    EthgasNodeExtension, NodeHooks,
    types::EthProvider,
    test_utils::engine::EngineApi,
};

/// Convenience alias for the local blockchain provider type.
pub type LocalNodeProvider = EthProvider;

/// Handle to a launched local node along with the resources required to keep it alive.
pub struct LocalNode {
    pub(crate) http_api_addr: SocketAddr,
    engine_ipc_path: String,
    pub(crate) ws_api_addr: SocketAddr,
    provider: LocalNodeProvider,
    _node_exit_future: NodeExitFuture,
    _node: Box<dyn Any + Sync + Send>,
    _runtime: reth_tasks::Runtime,
    _db_path: PathBuf,
}

impl Drop for LocalNode {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self._db_path);
    }
}

impl fmt::Debug for LocalNode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LocalNode")
            .field("http_api_addr", &self.http_api_addr)
            .field("ws_api_addr", &self.ws_api_addr)
            .field("engine_ipc_path", &self.engine_ipc_path)
            .finish_non_exhaustive()
    }
}

impl LocalNode {
    /// Launch a new local Ethereum node with the provided extensions and chain spec.
    pub async fn new(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        chain_spec: Arc<ChainSpec>,
    ) -> Result<Self> {
        let exec = reth_tasks::Runtime::test();

        let network_config = NetworkArgs {
            discovery: DiscoveryArgs { disable_discovery: true, ..DiscoveryArgs::default() },
            ..NetworkArgs::default()
        };

        let unique_ipc_path = format!(
            "/tmp/reth_engine_api_{}_{}_{:?}.ipc",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos(),
            std::process::id(),
            std::thread::current().id()
        );

        let mut rpc_args =
            RpcServerArgs::default().with_unused_ports().with_http().with_auth_ipc().with_ws();
        rpc_args.auth_ipc_path = unique_ipc_path;

        let eth_node = EthereumNode::default();

        let (db, db_path) = Self::create_test_database()?;

        let mut node_config = NodeConfig::new(Arc::clone(&chain_spec))
            .with_network(network_config)
            .with_rpc(rpc_args)
            .with_unused_ports();

        let datadir_path = MaybePlatformPath::<DataDirPath>::from(db_path.clone());
        node_config = node_config
            .with_datadir_args(DatadirArgs { datadir: datadir_path, ..Default::default() });

        let builder = NodeBuilder::new(node_config.clone())
            .with_database(db)
            .with_launch_context(exec.clone())
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(eth_node.components_builder())
            .with_add_ons(eth_node.add_ons())
            .on_component_initialized(move |_ctx| Ok(()));

        let NodeHandle { node: node_handle, node_exit_future } = extensions
            .into_iter()
            .fold(NodeHooks::new(), |b, ext| ext.apply(b))
            .apply_to(builder)
            .launch()
            .await?;

        let http_api_addr = node_handle
            .rpc_server_handle()
            .http_local_addr()
            .ok_or_else(|| eyre::eyre!("HTTP RPC server failed to bind to address"))?;

        let ws_api_addr = node_handle
            .rpc_server_handle()
            .ws_local_addr()
            .ok_or_else(|| eyre::eyre!("Failed to get websocket api address"))?;

        let engine_ipc_path = node_config.rpc.auth_ipc_path;
        let provider = node_handle.provider().clone();

        Ok(Self {
            http_api_addr,
            ws_api_addr,
            engine_ipc_path,
            provider,
            _node_exit_future: node_exit_future,
            _node: Box::new(node_handle),
            _runtime: exec,
            _db_path: db_path,
        })
    }

    fn create_test_database() -> Result<(DatabaseEnv, PathBuf)> {
        let path = tempdir_path();
        let args = DatabaseArguments::new(ClientVersion::default())
            .with_geometry_max_size(Some(100 * 1024 * 1024));
        let db = init_db(&path, args).expect("Failed to create test database");
        Ok((db, path))
    }

    /// Create an HTTP provider pointed at the node's public RPC endpoint.
    pub fn provider(&self) -> Result<RootProvider> {
        let url = format!("http://{}", self.http_api_addr);
        let client = RpcClient::builder().http(url.parse()?);
        Ok(RootProvider::new(client))
    }

    /// Build an Engine API client.
    pub fn engine_api(&self) -> Result<EngineApi> {
        EngineApi::new(self.engine_ipc_path.clone())
    }

    /// Clone the underlying blockchain provider.
    pub fn blockchain_provider(&self) -> LocalNodeProvider {
        self.provider.clone()
    }

    /// Websocket URL for the local node.
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.ws_api_addr)
    }
}
