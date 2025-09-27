#[cfg(test)]
mod tests {
    use crate::{
        payload::{
            ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
        },
        rpc::{EthApiExt, EthApiOverrideServer},
        service::FlashblocksReceiver,
        state::FlashblocksState,
    };
    use alloy_consensus::TxType;
    use alloy_eips::{BlockNumberOrTag, Encodable2718};
    use alloy_genesis::Genesis;
    use alloy_network::Ethereum;
    use alloy_primitives::{
        Address, B256, Bytes, TxHash, U256, address, b256, bytes, map::foldhash::HashMap,
    };
    use alloy_provider::{Provider, RootProvider};
    use alloy_rpc_client::RpcClient;
    use alloy_rpc_types::TransactionRequest;
    use alloy_rpc_types_engine::PayloadId;
    use alloy_rpc_types_eth::TransactionInput;
    use reth::{
        args::{DiscoveryArgs, NetworkArgs, RpcServerArgs},
        builder::{Node, NodeBuilder, NodeConfig, NodeHandle},
        chainspec::{ChainSpecBuilder, MAINNET},
        core::exit::NodeExitFuture,
        revm::context::tx,
        tasks::TaskManager,
    };
    use reth_e2e_test_utils::{transaction::TransactionTestContext, wallet::Wallet};
    use reth_node_ethereum::EthereumNode;
    use reth_primitives::{EthereumHardforks, Receipt};
    use reth_provider::providers::BlockchainProvider;
    use reth_rpc_eth_api::RpcReceipt;
    use serde_json;
    use std::{any::Any, collections::BTreeMap, net::SocketAddr, str::FromStr, sync::Arc};
    use tokio::sync::{mpsc, oneshot};

    pub struct NodeContext {
        sender: mpsc::Sender<(FlashBlock, oneshot::Sender<()>)>,
        http_api_addr: SocketAddr,
        _node_exit_future: NodeExitFuture,
        _node: Box<dyn Any + Sync + Send>,
        _task_manager: TaskManager,
    }

    impl NodeContext {
        pub async fn send_payload(&self, payload: FlashBlock) -> eyre::Result<()> {
            let (tx, rx) = oneshot::channel();
            self.sender.send((payload, tx)).await?;
            rx.await?;
            Ok(())
        }

        pub async fn provider(&self) -> eyre::Result<RootProvider<Ethereum>> {
            let url = format!("http://{}", self.http_api_addr);
            let client = RpcClient::builder().http(url.parse()?);

            Ok(RootProvider::<Ethereum>::new(client))
        }

        pub async fn send_test_payloads(&self) -> eyre::Result<()> {
            let base_payload = create_first_payload_();
            self.send_payload(base_payload).await?;

            let second_payload = create_second_payload();
            self.send_payload(second_payload).await?;

            Ok(())
        }

        pub async fn send_raw_transaction_sync(
            &self,
            tx: Bytes,
            timeout_ms: Option<u64>,
        ) -> eyre::Result<RpcReceipt<Ethereum>> {
            let url = format!("http://{}", self.http_api_addr);
            let client = RpcClient::new_http(url.parse()?);

            let receipt = client
                .request::<_, RpcReceipt<Ethereum>>("eth_sendRawTransactionSync", (tx, timeout_ms))
                .await?;

            Ok(receipt)
        }
    }

    async fn setup_node() -> eyre::Result<NodeContext> {
        let tasks = TaskManager::current();
        let exec = tasks.executor();

        let genesis: Genesis = serde_json::from_str(include_str!("assets/genesis.json")).unwrap();
        let chain_spec = Arc::new(
            ChainSpecBuilder::default()
                .chain(MAINNET.chain)
                .genesis(genesis)
                .prague_activated()
                .build(),
        );

        let network_config = NetworkArgs {
            discovery: DiscoveryArgs { disable_discovery: true, ..DiscoveryArgs::default() },
            ..NetworkArgs::default()
        };

        // Use with_unused_ports() to let Reth allocate random ports and avoid port collisions
        let node_config = NodeConfig::new(chain_spec.clone())
            .with_network(network_config.clone())
            .with_rpc(RpcServerArgs::default().with_unused_ports().with_http())
            .with_unused_ports();

        let node = EthereumNode::default();

        // Start websocket server to simulate the builder and send payloads back to the node
        let (sender, mut receiver) = mpsc::channel::<(FlashBlock, oneshot::Sender<()>)>(100);

        let NodeHandle { node, node_exit_future } = NodeBuilder::new(node_config.clone())
            .testing_node(exec.clone())
            .with_types_and_provider::<EthereumNode, BlockchainProvider<_>>()
            .with_components(node.components_builder())
            .with_add_ons(node.add_ons())
            .extend_rpc_modules(move |ctx| {
                // We are not going to use the websocket connection to send payloads so we use
                // a dummy url.
                let flashblocks_state =
                    Arc::new(FlashblocksState::new(ctx.provider().clone(), chain_spec.clone()));
                flashblocks_state.start();

                let api_ext =
                    EthApiExt::new(ctx.registry.eth_api().clone(), flashblocks_state.clone());

                ctx.modules.replace_configured(api_ext.into_rpc())?;

                tokio::spawn(async move {
                    while let Some((payload, tx)) = receiver.recv().await {
                        flashblocks_state.on_flashblock_received(payload);
                        tx.send(()).unwrap();
                    }
                });

                Ok(())
            })
            .launch()
            .await?;

        let http_api_addr = node
            .rpc_server_handle()
            .http_local_addr()
            .ok_or_else(|| eyre::eyre!("Failed to get http api address"))?;

        Ok(NodeContext {
            sender,
            http_api_addr,
            _node_exit_future: node_exit_future,
            _node: Box::new(node),
            _task_manager: tasks,
        })
    }

    fn create_first_payload(tx_hash: B256, tx_bytes: Bytes) -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 0,
            base: Some(ExecutionPayloadBaseV1 {
                parent_beacon_block_root: B256::default(),
                parent_hash: B256::default(),
                fee_recipient: Address::ZERO,
                prev_randao: B256::default(),
                block_number: 1,
                gas_limit: 30_000_000,
                timestamp: 0,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::ZERO,
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                transactions: vec![tx_bytes],
                ..Default::default()
            },
            metadata: Metadata {
                block_number: 1,
                receipts: {
                    let mut receipts: HashMap<TxHash, Receipt> = HashMap::default();
                    receipts.insert(
                        tx_hash,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true.into(),
                            cumulative_gas_used: 55000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                new_account_balances: HashMap::default(),
            },
        }
    }

    fn create_first_payload_() -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 0,
            base: Some(ExecutionPayloadBaseV1 {
                parent_beacon_block_root: B256::default(),
                parent_hash: B256::default(),
                fee_recipient: Address::ZERO,
                prev_randao: B256::default(),
                block_number: 1,
                gas_limit: 30_000_000,
                timestamp: 0,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::ZERO,
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                transactions: vec![TRANSFER_ETH_TX],
                ..Default::default()
            },
            metadata: Metadata {
                block_number: 1,
                receipts: {
                    let mut receipts: HashMap<TxHash, Receipt> = HashMap::default();
                    receipts.insert(
                        TRANSFER_ETH_HASH,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true.into(),
                            cumulative_gas_used: 24000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                new_account_balances: HashMap::default(),
            },
        }
    }

    const TEST_ADDRESS: Address = address!("0x1234567890123456789012345678901234567890");
    const PENDING_BALANCE: u64 = 4660;

    const TX_SENDER: Address = address!("0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266");

    const TRANSFER_ETH_HASH: TxHash =
        b256!("0xbb079fbde7d12fd01664483cd810e91014113e405247479e5615974ebca93e4a");

    const DEPLOYMENT_HASH: TxHash =
        b256!("0xa9353897b4ab350ae717eefdad4c9cb613e684f5a490c82a44387d8d5a2f8197");

    const INCREMENT_HASH: TxHash =
        b256!("0x993ad6a332752f6748636ce899b3791e4a33f7eece82c0db4556c7339c1b2929");

    const COUNTER_ADDRESS: Address = address!("0xe7f1725E7734CE288F8367e1Bb143E90bb3F0512");

    // NOTE:
    // To create tx use cast mktx/
    // Example: `cast mktx --private-key
    // 0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80 --nonce 1 --gas-limit
    // 100000 --gas-price 1499576 --chain 84532 --value 0 --priority-gas-price 0
    // 0xf39Fd6e51aad88F6F4ce6aB8827279cffFb92266 0x` Create second payload (index 1) with
    // transactions tx1 hash: 0x2be2e6f8b01b03b87ae9f0ebca8bbd420f174bef0fbcc18c7802c5378b78f548
    // (deposit transaction)
    // tx2 hash: 0xbb079fbde7d12fd01664483cd810e91014113e405247479e5615974ebca93e4a
    const TRANSFER_ETH_TX: Bytes = bytes!(
        "0x02f87383014a3480808449504f80830186a094deaddeaddeaddeaddeaddeaddeaddeaddead00018ad3c21bcb3f6efc39800080c0019f5a6fe2065583f4f3730e82e5725f651cbbaf11dc1f82c8d29ba1f3f99e5383a061e0bf5dfff4a9bc521ad426eee593d3653c5c330ae8a65fad3175d30f291d31"
    );

    // NOTE:
    // Following txns deploy a simple Counter contract (Compiled with solc 0.8.13)
    // Only contains a `uin256 public number` and a function increment() { number++ };
    // Following txn calls increment once, so number should be 1
    // Raw Bytecode:
    // 0x608060405234801561001057600080fd5b50610163806100206000396000f3fe608060405234801561001057600080fd5b50600436106100365760003560e01c80638381f58a1461003b578063d09de08a14610059575b600080fd5b610043610063565b604051610050919061009b565b60405180910390f35b610061610069565b005b60005481565b60008081548092919061007b906100e5565b9190505550565b6000819050919050565b61009581610082565b82525050565b60006020820190506100b0600083018461008c565b92915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b60006100f082610082565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8203610122576101216100b6565b5b60018201905091905056fea2646970667358221220a0719cefc3439563ff433fc58f8ffb66e1b639119206276d3bdac5d2e2b6f2fa64736f6c634300080d0033
    const DEPLOYMENT_TX: Bytes = bytes!(
        "0x02f901db83014a3401808449504f8083030d408080b90183608060405234801561001057600080fd5b50610163806100206000396000f3fe608060405234801561001057600080fd5b50600436106100365760003560e01c80638381f58a1461003b578063d09de08a14610059575b600080fd5b610043610063565b604051610050919061009b565b60405180910390f35b610061610069565b005b60005481565b60008081548092919061007b906100e5565b9190505550565b6000819050919050565b61009581610082565b82525050565b60006020820190506100b0600083018461008c565b92915050565b7f4e487b7100000000000000000000000000000000000000000000000000000000600052601160045260246000fd5b60006100f082610082565b91507fffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffffff8203610122576101216100b6565b5b60018201905091905056fea2646970667358221220a0719cefc3439563ff433fc58f8ffb66e1b639119206276d3bdac5d2e2b6f2fa64736f6c634300080d0033c080a034278436b367f7b73ab6dc7c7cc09f8880104513f8b8fb691b498257de97a5bca05cb702ebad2aadf9f225bf5f8685ea03d194bf7a2ea05b1d27a1bd33169f9fe0"
    );
    // Increment tx: call increment()
    const INCREMENT_TX: Bytes = bytes!(
        "0x02f86d83014a3402808449504f8082abe094e7f1725e7734ce288f8367e1bb143e90bb3f05128084d09de08ac080a0a9c1a565668084d4052bbd9bc3abce8555a06aed6651c82c2756ac8a83a79fa2a03427f440ce4910a5227ea0cedb60b06cf0bea2dbbac93bd37efa91a474c29d89"
    );

    fn create_second_payload() -> FlashBlock {
        let payload = FlashBlock {
            payload_id: PayloadId::new([0; 8]),
            index: 1,
            base: None,
            diff: ExecutionPayloadFlashblockDeltaV1 {
                state_root: B256::default(),
                receipts_root: B256::default(),
                gas_used: 0,
                block_hash: B256::default(),
                transactions: vec![DEPLOYMENT_TX, INCREMENT_TX],
                withdrawals: Vec::new(),
                logs_bloom: Default::default(),
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            metadata: Metadata {
                block_number: 1,
                receipts: {
                    let mut receipts: HashMap<TxHash, Receipt> = HashMap::default();

                    receipts.insert(
                        DEPLOYMENT_HASH,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true.into(),
                            cumulative_gas_used: 172279,
                            logs: vec![],
                        },
                    );
                    receipts.insert(
                        INCREMENT_HASH,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true.into(),
                            cumulative_gas_used: 172279 + 44000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                new_account_balances: {
                    let mut map = HashMap::default();
                    map.insert(TEST_ADDRESS, U256::from(PENDING_BALANCE));
                    map.insert(COUNTER_ADDRESS, U256::from(0));
                    map
                },
            },
        };

        payload
    }

    #[tokio::test]
    async fn test_get_pending_block() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;
        let provider = node.provider().await?;

        let latest_block = provider
            .get_block_by_number(alloy_eips::BlockNumberOrTag::Latest)
            .await?
            .expect("latest block expected");
        assert_eq!(latest_block.number(), 0);

        // Querying pending block when it does not exists yet
        let pending_block =
            provider.get_block_by_number(alloy_eips::BlockNumberOrTag::Pending).await?;
        assert_eq!(pending_block.is_none(), true);

        let base_payload = create_first_payload_();
        node.send_payload(base_payload).await?;

        // Query pending block after sending the base payload with an empty delta
        let pending_block = provider
            .get_block_by_number(alloy_eips::BlockNumberOrTag::Pending)
            .await?
            .expect("pending block expected");

        assert_eq!(pending_block.number(), 1);
        assert_eq!(pending_block.transactions.hashes().len(), 1); // L1Info transaction

        let second_payload = create_second_payload();
        node.send_payload(second_payload).await?;

        // Query pending block after sending the second payload with two transactions
        let block = provider
            .get_block_by_number(alloy_eips::BlockNumberOrTag::Pending)
            .await?
            .expect("pending block expected");

        assert_eq!(block.number(), 1);
        assert_eq!(block.transactions.hashes().len(), 3);

        Ok(())
    }

    #[tokio::test]
    async fn test_get_balance_pending() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;
        let provider = node.provider().await?;

        node.send_test_payloads().await?;

        let balance = provider.get_balance(TEST_ADDRESS).await?;
        assert_eq!(balance, U256::ZERO);

        let pending_balance = provider.get_balance(TEST_ADDRESS).pending().await?;
        assert_eq!(pending_balance, U256::from(PENDING_BALANCE));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_transaction_by_hash_pending() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;
        let provider = node.provider().await?;

        assert!(provider.get_transaction_by_hash(TRANSFER_ETH_HASH).await?.is_none());

        node.send_test_payloads().await?;

        let tx2 = provider.get_transaction_by_hash(TRANSFER_ETH_HASH).await?.expect(
            "tx2
    expected",
        );
        assert_eq!(*tx2.inner.tx_hash(), TRANSFER_ETH_HASH);
        assert_eq!(tx2.inner.signer(), TX_SENDER);

        // TODO: Verify more properties of the txns here.

        Ok(())
    }

    #[tokio::test]
    async fn test_get_transaction_receipt_pending() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;
        let provider = node.provider().await?;

        node.send_test_payloads().await?;

        let receipt = provider.get_transaction_receipt(TRANSFER_ETH_HASH).await?.expect(
            "receipt
    expected",
        );
        assert_eq!(receipt.gas_used, 24000); // 45000 - 21000

        // TODO: Add a new payload and validate that the receipts from the previous payload
        // are not returned.

        Ok(())
    }

    #[tokio::test]
    async fn test_get_transaction_count() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;
        let provider = node.provider().await?;

        assert_eq!(provider.get_transaction_count(TX_SENDER).pending().await?, 0);

        node.send_test_payloads().await?;

        assert_eq!(provider.get_transaction_count(TX_SENDER).pending().await?, 3);

        Ok(())
    }

    #[tokio::test]
    async fn test_eth_call() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;

        let provider = node.provider().await?;

        // We ensure that eth_call will succeed because we are on plain state
        let send_eth_call = TransactionRequest::default()
            .from(TX_SENDER)
            .transaction_type(0)
            .gas_limit(200000)
            .nonce(1)
            .to(address!("0xf39635f2adf40608255779ff742afe13de31f577"))
            .value(U256::from(9999999999849942300000u128))
            .input(TransactionInput::new(bytes!("0x")));

        let res =
            provider.call(send_eth_call.clone()).block(BlockNumberOrTag::Pending.into()).await;

        assert!(res.is_ok());

        node.send_test_payloads().await?;

        // We included heavy spending transaction and now don't have enough funds for this
        // request,     // so this eth_call with fail
        let res = provider.call(send_eth_call).block(BlockNumberOrTag::Pending.into()).await;

        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .as_error_resp()
                .unwrap()
                .message
                .contains("insufficient funds for gas")
        );

        // read number from counter contract
        let eth_call = TransactionRequest::default()
            .from(TX_SENDER)
            .transaction_type(0)
            .gas_limit(20000000)
            .nonce(4)
            .to(COUNTER_ADDRESS)
            .value(U256::ZERO)
            .input(TransactionInput::new(bytes!("0x8381f58a")));
        let res = provider.call(eth_call).await;
        assert!(res.is_ok());
        assert_eq!(U256::from_str(res.unwrap().to_string().as_str()).unwrap(), U256::from(1));

        Ok(())
    }

    #[tokio::test]
    async fn test_send_raw_transaction_sync() -> eyre::Result<()> {
        reth_tracing::init_test_tracing();
        let node = setup_node().await?;

        let wallet = Wallet::default();
        let tx_env = TransactionTestContext::transfer_tx(1, wallet.inner).await;
        let tx_hash = tx_env.hash().clone();
        let raw_tx: Bytes = tx_env.encoded_2718().into();

        node.send_payload(create_first_payload_()).await?;

        // run the Tx sync and, in parallel, deliver the payload that contains the Tx
        let (receipt_result, payload_result) =
            tokio::join!(node.send_raw_transaction_sync(raw_tx, None), async {
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                node.send_payload(create_second_payload()).await
            });

        payload_result?;
        let receipt = receipt_result?;

        assert_eq!(receipt.transaction_hash, tx_hash);
        Ok(())
    }
}
