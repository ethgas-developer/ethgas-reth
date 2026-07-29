//! Unified test harness combining node and engine helpers.

use std::{sync::Arc, time::Duration};

use alloy_eips::{BlockHashOrNumber, eip7685::RequestsOrHash};
use alloy_primitives::{B256, Bytes};
use alloy_provider::{Provider, RootProvider};
use alloy_rpc_client::RpcClient;
use alloy_rpc_types::BlockNumberOrTag;
use alloy_rpc_types_engine::PayloadAttributes;
use eyre::{Result, eyre};
use reth_chainspec::{ChainSpec, ChainSpecProvider};
use reth_ethereum_primitives::Block;
use reth_primitives_traits::{Block as BlockT, RecoveredBlock};
use reth_provider::{BlockNumReader, BlockReader};
use tokio::time::sleep;

use ethgas_test_utils::build_test_genesis;

use crate::{
    EthgasNodeExtension, FromExtensionConfig,
    test_utils::{
        constants::{BLOCK_BUILD_DELAY_MS, BLOCK_TIME_SECONDS, NODE_STARTUP_DELAY_MS},
        engine::EngineApi,
        node::{LocalNode, LocalNodeProvider},
        tracing::init_silenced_tracing,
    },
};

/// Builder for configuring and launching a test harness.
#[derive(Debug, Default)]
pub struct TestHarnessBuilder {
    extensions: Vec<Box<dyn EthgasNodeExtension>>,
    chain_spec: Option<Arc<ChainSpec>>,
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

    /// Build and launch the test harness.
    pub async fn build(self) -> Result<TestHarness> {
        init_silenced_tracing();

        let chain_spec = self.chain_spec.unwrap_or_else(|| {
            let genesis = build_test_genesis();
            Arc::new(ChainSpec::from(genesis))
        });

        let node = LocalNode::new(self.extensions, chain_spec).await?;
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
            withdrawals: Some(vec![]),
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

        let payload_envelope = self.engine.get_payload(payload_id).await?;

        let execution_requests = RequestsOrHash::Requests(payload_envelope.execution_requests);

        let payload_status = self
            .engine
            .new_payload(
                payload_envelope.envelope_inner.execution_payload,
                vec![],
                parent_beacon_block_root,
                execution_requests,
            )
            .await?;

        if payload_status.status.is_invalid() {
            return Err(eyre!("Engine rejected payload: {:?}", payload_status));
        }

        let new_block_hash = payload_status
            .latest_valid_hash
            .ok_or_else(|| eyre!("Payload status missing latest_valid_hash"))?;

        self.engine.update_forkchoice(parent_hash, new_block_hash, None).await?;

        Ok(())
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
