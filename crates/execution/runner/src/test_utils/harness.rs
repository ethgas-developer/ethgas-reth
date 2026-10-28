//! Unified test harness combining node and engine helpers.

use std::{sync::Arc, time::Duration};

use alloy_eips::{BlockHashOrNumber, eip4895::Withdrawal};
use alloy_primitives::{B256, Bytes};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types::BlockNumberOrTag;
use alloy_rpc_types_engine::PayloadAttributes;
use eyre::{Result, eyre};
use reth_chainspec::{ChainSpec, ChainSpecProvider, EthereumHardforks};
use reth_ethereum_primitives::Block;
use reth_network_p2p::sync::SyncState;
use reth_primitives_traits::{Block as BlockT, RecoveredBlock};
use reth_provider::{BlockNumReader, BlockReader, DatabaseProviderFactory};
use tokio::time::sleep;

use ethgas_test_utils::{build_test_genesis, slot_number_at};

use crate::{
    EthgasNodeExtension, FromExtensionConfig, PendingStateSource,
    test_utils::{
        constants::{BLOCK_BUILD_DELAY_MS, BLOCK_TIME_SECONDS, NODE_STARTUP_DELAY_MS},
        engine::{EngineApi, EnginePayload},
        node::{LocalNode, LocalNodeOptions, LocalNodeProvider},
        tracing::init_silenced_tracing,
    },
};

/// Builder for configuring and launching a test harness.
#[derive(Debug, Default)]
pub struct TestHarnessBuilder {
    extensions: Vec<Box<dyn EthgasNodeExtension>>,
    chain_spec: Option<Arc<ChainSpec>>,
    pending_state: Option<Arc<dyn PendingStateSource>>,
    rpc_modules: Option<String>,
    persistence_threshold: Option<u64>,
    send_raw_transaction_sync_timeout: Option<Duration>,
}

impl TestHarnessBuilder {
    /// Create a new builder with no extensions.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add an extension to be applied during node launch using its config type.
    pub fn with_ext<T: FromExtensionConfig + 'static>(mut self, config: T::Config) -> Self {
        self.extensions.push(Box::new(T::from_config(config)));
        self
    }

    /// Add a pre-constructed extension.
    pub fn with_extension(mut self, ext: impl EthgasNodeExtension + 'static) -> Self {
        self.extensions.push(Box::new(ext));
        self
    }

    /// Set a custom chain spec.
    pub fn with_chain_spec(mut self, chain_spec: Arc<ChainSpec>) -> Self {
        self.chain_spec = Some(chain_spec);
        self
    }

    /// Set the source the `eth` API answers the `pending` tag from.
    pub fn with_pending_state(mut self, pending_state: Arc<dyn PendingStateSource>) -> Self {
        self.pending_state = Some(pending_state);
        self
    }

    /// Choose the HTTP and WS modules, as `--http.api` does: a comma-separated list such as
    /// `"eth,net,web3,ots"`. The default is reth's standard set.
    pub fn with_rpc_modules(mut self, rpc_modules: &str) -> Self {
        self.rpc_modules = Some(rpc_modules.to_owned());
        self
    }

    /// Set `--engine.persistence-threshold`: how many canonical blocks stay in memory before
    /// the engine persists them. Zero persists every block as it turns canonical.
    pub const fn with_persistence_threshold(mut self, persistence_threshold: u64) -> Self {
        self.persistence_threshold = Some(persistence_threshold);
        self
    }

    /// Set `--rpc.send-raw-transaction-sync-timeout`: the longest `eth_sendRawTransactionSync`
    /// waits, which a requested timeout is clamped to.
    pub const fn with_send_raw_transaction_sync_timeout(mut self, timeout: Duration) -> Self {
        self.send_raw_transaction_sync_timeout = Some(timeout);
        self
    }

    /// Build and launch the test harness.
    pub async fn build(self) -> Result<TestHarness> {
        init_silenced_tracing();

        let chain_spec = self.chain_spec.unwrap_or_else(|| {
            let genesis = build_test_genesis();
            Arc::new(ChainSpec::from(genesis))
        });

        let options = LocalNodeOptions {
            rpc_modules: self.rpc_modules,
            persistence_threshold: self.persistence_threshold,
            send_raw_transaction_sync_timeout: self.send_raw_transaction_sync_timeout,
        };
        let node =
            LocalNode::with_options(self.extensions, chain_spec, self.pending_state, options)
                .await?;
        let engine = node.engine_api()?;

        sleep(Duration::from_millis(NODE_STARTUP_DELAY_MS)).await;

        Ok(TestHarness { node, engine })
    }
}

/// High-level façade that bundles a local node, engine API client, and common helpers.
#[derive(Debug)]
pub struct TestHarness {
    node: LocalNode,
    engine: EngineApi,
}

impl TestHarness {
    /// Launch a new harness using the default configuration.
    pub async fn new() -> Result<Self> {
        TestHarnessBuilder::new().build().await
    }

    /// Create a builder for configuring the test harness.
    pub fn builder() -> TestHarnessBuilder {
        TestHarnessBuilder::new()
    }

    /// Create a harness from pre-built parts.
    pub const fn from_parts(node: LocalNode, engine: EngineApi) -> Self {
        Self { node, engine }
    }

    /// Return a JSON-RPC provider connected to the harness node.
    pub fn provider(&self) -> RootProvider {
        self.node.provider().expect("provider should always be available after node initialization")
    }

    /// Access the low-level blockchain provider.
    pub fn blockchain_provider(&self) -> LocalNodeProvider {
        self.node.blockchain_provider()
    }

    /// The Engine API client, for forkchoice updates a helper does not cover.
    pub const fn engine(&self) -> &EngineApi {
        &self.engine
    }

    /// Path of the IPC endpoint, which serves every enabled module.
    pub fn ipc_path(&self) -> Option<&str> {
        self.node.ipc_path()
    }

    /// The highest block number the database holds. Blocks above it are canonical in memory
    /// only, until the engine persists them.
    pub fn persisted_block_number(&self) -> Result<u64> {
        Ok(self.blockchain_provider().database_provider_ro()?.last_block_number()?)
    }

    /// Waits until the database holds block `number`. Persistence runs on its own task, so a
    /// block is canonical before it is persisted even at a threshold of zero.
    pub async fn wait_until_persisted(&self, number: u64) -> Result<()> {
        for _ in 0..PERSISTENCE_POLLS {
            if self.persisted_block_number()? >= number {
                return Ok(());
            }
            sleep(Duration::from_millis(PERSISTENCE_POLL_INTERVAL_MS)).await;
        }
        Err(eyre!("block {number} was not persisted in time"))
    }

    /// Set the sync state the node reports through `eth_syncing`.
    pub fn set_sync_state(&self, state: SyncState) {
        self.node.set_sync_state(state);
    }

    /// HTTP URL for sending JSON-RPC requests.
    pub fn rpc_url(&self) -> String {
        format!("http://{}", self.node.http_api_addr)
    }

    /// Websocket URL for subscribing to JSON-RPC notifications.
    pub fn ws_url(&self) -> String {
        format!("ws://{}", self.node.ws_api_addr)
    }

    /// Return a JSON-RPC client.
    pub fn rpc_client(&self) -> Result<RpcClient> {
        let url = self.rpc_url().parse()?;
        Ok(RpcClient::new_http(url))
    }

    /// Build a block using the provided transactions and push it through the engine.
    ///
    /// Transactions are submitted to the node's transaction pool before triggering
    /// the engine to build a block that will include them.
    pub async fn build_block_from_transactions(&self, transactions: Vec<Bytes>) -> Result<()> {
        self.build_block_with_withdrawals(transactions, vec![]).await?;
        Ok(())
    }

    /// Like [`Self::build_block_from_transactions`], with `withdrawals` in the payload
    /// attributes. Returns the payload the engine built, which the block is made from.
    pub async fn build_block_with_withdrawals(
        &self,
        transactions: Vec<Bytes>,
        withdrawals: Vec<Withdrawal>,
    ) -> Result<EnginePayload> {
        let SubmittedBlock { parent_hash, hash, payload } =
            self.submit_block(transactions, withdrawals).await?;
        self.engine.update_forkchoice(parent_hash, hash, None).await?;
        Ok(payload)
    }

    /// Like [`Self::build_block_from_transactions`] without the forkchoice update: the engine
    /// executes the block and holds it as pending. Returns the block hash.
    pub async fn submit_block_from_transactions(&self, transactions: Vec<Bytes>) -> Result<B256> {
        Ok(self.submit_block(transactions, vec![]).await?.hash)
    }

    async fn submit_block(
        &self,
        transactions: Vec<Bytes>,
        withdrawals: Vec<Withdrawal>,
    ) -> Result<SubmittedBlock> {
        // Submit transactions to the mempool so the payload builder picks them up.
        let provider = self.provider();
        for tx in &transactions {
            let _ = provider.send_raw_transaction(tx).await?;
        }

        let latest_block = self
            .provider()
            .get_block_by_number(BlockNumberOrTag::Latest)
            .await?
            .ok_or_else(|| eyre!("No genesis block found"))?;

        let parent_hash = latest_block.header.hash;
        let parent_beacon_block_root =
            latest_block.header.parent_beacon_block_root.unwrap_or(B256::ZERO);
        let next_timestamp = latest_block.header.timestamp + BLOCK_TIME_SECONDS;

        let payload_attributes = PayloadAttributes {
            timestamp: next_timestamp,
            parent_beacon_block_root: Some(parent_beacon_block_root),
            withdrawals: Some(withdrawals),
            slot_number: self
                .chain_spec()
                .is_amsterdam_active_at_timestamp(next_timestamp)
                .then(|| slot_number_at(next_timestamp)),
            ..Default::default()
        };

        let forkchoice_result = self
            .engine
            .update_forkchoice(parent_hash, parent_hash, Some(payload_attributes))
            .await?;

        let payload_id = forkchoice_result
            .payload_id
            .ok_or_else(|| eyre!("Forkchoice update did not return payload ID"))?;

        sleep(Duration::from_millis(BLOCK_BUILD_DELAY_MS)).await;

        let payload = self.engine.get_payload(payload_id, next_timestamp).await?;
        let payload_status =
            self.engine.new_payload(payload.clone(), parent_beacon_block_root).await?;

        if payload_status.status.is_invalid() {
            return Err(eyre!("Engine rejected payload: {:?}", payload_status));
        }

        let hash = payload_status
            .latest_valid_hash
            .ok_or_else(|| eyre!("Payload status missing latest_valid_hash"))?;

        Ok(SubmittedBlock { parent_hash, hash, payload })
    }

    /// Advance the canonical chain by `n` empty blocks.
    pub async fn advance_chain(&self, n: u64) -> Result<()> {
        for _ in 0..n {
            self.build_block_from_transactions(vec![]).await?;
        }
        Ok(())
    }

    /// Return the latest recovered block.
    pub fn latest_block(&self) -> RecoveredBlock<Block> {
        let provider = self.blockchain_provider();
        let best_number = provider.best_block_number().expect("able to read best block number");
        let block = provider
            .block(BlockHashOrNumber::Number(best_number))
            .expect("able to load canonical block")
            .expect("canonical block exists");
        BlockT::try_into_recovered(block).expect("able to recover canonical block")
    }

    /// Return the chain specification.
    pub fn chain_spec(&self) -> Arc<ChainSpec> {
        self.node.blockchain_provider().chain_spec()
    }

    /// Return the chain ID.
    pub fn chain_id(&self) -> u64 {
        self.chain_spec().chain().id()
    }
}

/// How often, and how long apart, [`TestHarness::wait_until_persisted`] polls the database.
const PERSISTENCE_POLLS: u32 = 200;
const PERSISTENCE_POLL_INTERVAL_MS: u64 = 25;

struct SubmittedBlock {
    parent_hash: B256,
    hash: B256,
    payload: EnginePayload,
}
