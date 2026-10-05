//! Local node setup for Ethereum integration testing.

use std::{any::Any, fmt, net::SocketAddr, path::PathBuf, sync::Arc, time::Duration};

use alloy_provider::RootProvider;
use alloy_rpc_client::RpcClient;
use eyre::Result;
use reth_chainspec::ChainSpec;
use reth_db::{
    ClientVersion, DatabaseEnv, init_db, mdbx::DatabaseArguments, test_utils::tempdir_path,
};
use reth_network_p2p::sync::{NetworkSyncUpdater, SyncState};
use reth_node_builder::{
    Node, NodeBuilder, NodeConfig, NodeHandle,
    rpc::{BasicEngineApiBuilder, BasicEngineValidatorBuilder, Identity, RpcAddOns},
};
use reth_node_core::{
    args::{DatadirArgs, DebugArgs, DiscoveryArgs, NetworkArgs, RpcServerArgs},
    dirs::{DataDirPath, MaybePlatformPath},
    exit::NodeExitFuture,
};
use reth_node_ethereum::{
    EthereumNode,
    node::{EthereumAddOns, EthereumEngineValidatorBuilder},
};
use reth_provider::{ChainSpecProvider, providers::BlockchainProvider};
use reth_tasks::Runtime;

use crate::{
    EthgasNodeExtension, NodeHooks, PendingStateSource, eth_api::EthgasEthApiBuilder,
    test_utils::engine::EngineApi, types::EthProvider,
};

/// Convenience alias for the local blockchain provider type.
pub type LocalNodeProvider = EthProvider;

/// Execution cache of a test node, in MB. reth's default, 4096, makes every node that executes a
/// block allocate 2,176 MiB; reth's own test nodes use 1.
const CROSS_BLOCK_CACHE_SIZE_MB: usize = 1;

/// Options for a [`LocalNode`] beyond the defaults.
#[derive(Debug, Default, Clone)]
pub struct LocalNodeOptions {
    /// The HTTP and WS modules, as `--http.api` chooses them: a comma-separated list such as
    /// `"eth,net,web3,ots"`. `None` keeps reth's standard set.
    pub rpc_modules: Option<String>,
    /// `--engine.persistence-threshold`: how many canonical blocks stay in memory before the
    /// engine persists them. `None` keeps reth's default.
    pub persistence_threshold: Option<u64>,
    /// `--rpc.send-raw-transaction-sync-timeout`: the longest `eth_sendRawTransactionSync`
    /// waits, which a requested timeout is clamped to. `None` keeps reth's default of 30 s.
    pub send_raw_transaction_sync_timeout: Option<Duration>,
}

/// Longest wait for each of the two shutdown steps when a [`LocalNode`] is dropped.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Handle to a launched local node along with the resources required to keep it alive.
/// Dropping it shuts the node down and removes its datadir.
pub struct LocalNode {
    pub(crate) http_api_addr: SocketAddr,
    engine_ipc_path: String,
    ipc_path: Option<String>,
    pub(crate) ws_api_addr: SocketAddr,
    provider: LocalNodeProvider,
    network: Arc<dyn NetworkSyncUpdater>,
    _node_exit_future: NodeExitFuture,
    node: Option<Box<dyn Any + Sync + Send>>,
    runtime: Runtime,
    db_path: PathBuf,
}

impl Drop for LocalNode {
    fn drop(&mut self) {
        let runtime = self.runtime.clone();
        let node = self.node.take();
        let shutdown = std::thread::spawn(move || {
            runtime.graceful_shutdown_with_timeout(SHUTDOWN_TIMEOUT);
            drop(node);
            runtime.shutdown_timeout(SHUTDOWN_TIMEOUT);
        });
        let _ = shutdown.join();
        let _ = std::fs::remove_dir_all(&self.db_path);
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
        pending_state: Option<Arc<dyn PendingStateSource>>,
    ) -> Result<Self> {
        Self::with_rpc_modules(extensions, chain_spec, pending_state, None).await
    }

    /// Like [`Self::new`], with the HTTP and WS modules chosen as `--http.api` chooses them: a
    /// comma-separated list such as `"eth,net,web3,ots"`. `None` keeps reth's standard set.
    pub async fn with_rpc_modules(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        chain_spec: Arc<ChainSpec>,
        pending_state: Option<Arc<dyn PendingStateSource>>,
        rpc_modules: Option<&str>,
    ) -> Result<Self> {
        let options =
            LocalNodeOptions { rpc_modules: rpc_modules.map(str::to_owned), ..Default::default() };
        Self::with_options(extensions, chain_spec, pending_state, options).await
    }

    /// Like [`Self::new`], with [`LocalNodeOptions`].
    pub async fn with_options(
        extensions: Vec<Box<dyn EthgasNodeExtension>>,
        chain_spec: Arc<ChainSpec>,
        pending_state: Option<Arc<dyn PendingStateSource>>,
        options: LocalNodeOptions,
    ) -> Result<Self> {
        let exec = std::thread::spawn(Runtime::test)
            .join()
            .map_err(|_| eyre::eyre!("failed to build the node's runtime"))?;

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
        if let Some(modules) = options.rpc_modules.as_deref() {
            let selection = modules
                .parse()
                .ok()
                .ok_or_else(|| eyre::eyre!("invalid rpc module list: {modules}"))?;
            rpc_args = rpc_args.with_api(selection);
        }
        rpc_args.auth_ipc_path = unique_ipc_path;
        if let Some(timeout) = options.send_raw_transaction_sync_timeout {
            rpc_args.rpc_send_raw_transaction_sync_timeout = timeout;
        }

        let eth_node = EthereumNode::default();

        let (db, db_path) = Self::create_test_database()?;

        let debug_args = DebugArgs { startup_sync_state_idle: true, ..DebugArgs::default() };

        let mut node_config = NodeConfig::new(Arc::clone(&chain_spec))
            .with_network(network_config)
            .with_rpc(rpc_args)
            .with_debug(debug_args)
            .with_unused_ports();

        let datadir_path = MaybePlatformPath::<DataDirPath>::from(db_path.clone());
        node_config = node_config
            .with_datadir_args(DatadirArgs { datadir: datadir_path, ..Default::default() });
        node_config.engine.cross_block_cache_size = CROSS_BLOCK_CACHE_SIZE_MB;
        if let Some(persistence_threshold) = options.persistence_threshold {
            node_config.engine.persistence_threshold = persistence_threshold;
        }

        let builder = NodeBuilder::new(node_config.clone())
            .with_database(db)
            .with_launch_context(exec.clone())
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(eth_node.components_builder())
            .with_add_ons(EthereumAddOns::new(RpcAddOns::new(
                EthgasEthApiBuilder::new(pending_state).with_send_raw_transaction_sync_timeout(
                    node_config.rpc.rpc_send_raw_transaction_sync_timeout,
                ),
                EthereumEngineValidatorBuilder::default(),
                BasicEngineApiBuilder::default(),
                BasicEngineValidatorBuilder::default(),
                Default::default(),
                Identity::new(),
            )))
            .on_component_initialized(move |_ctx| Ok(()));

        let NodeHandle { node: node_handle, node_exit_future } = extensions
            .into_iter()
            .fold(NodeHooks::new(), |b, ext| ext.apply(b))
            .apply_to(builder)
            .launch_with_debug_capabilities()
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
        let ipc_path = node_handle.rpc_server_handle().ipc_endpoint();
        let provider = node_handle.provider().clone();
        let network = Arc::new(node_handle.network.clone());

        Ok(Self {
            http_api_addr,
            ws_api_addr,
            engine_ipc_path,
            ipc_path,
            provider,
            network,
            _node_exit_future: node_exit_future,
            node: Some(Box::new(node_handle)),
            runtime: exec,
            db_path,
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
        EngineApi::new(self.engine_ipc_path.clone(), self.provider.chain_spec())
    }

    /// Clone the underlying blockchain provider.
    pub fn blockchain_provider(&self) -> LocalNodeProvider {
        self.provider.clone()
    }

    /// Set the sync state the node reports through `eth_syncing`.
    pub fn set_sync_state(&self, state: SyncState) {
        self.network.update_sync_state(state);
    }

    /// Websocket URL for the local node.
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.ws_api_addr)
    }

    /// Path of the IPC endpoint, which serves every enabled module.
    pub fn ipc_path(&self) -> Option<&str> {
        self.ipc_path.as_deref()
    }
}
