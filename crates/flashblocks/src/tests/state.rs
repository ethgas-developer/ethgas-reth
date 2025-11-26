#[cfg(test)]
mod tests {
    use crate::{
        payload::{
            ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, Flashblock, Metadata,
        },
        traits::{FlashblocksAPI, PendingBlocksAPI},
        service::FlashblocksReceiver,
        state::FlashblocksState,
        tests::utils::create_test_provider_factory,
    };
    use alloy_consensus::{
        BlockBody, BlockHeader, Header, Transaction, TxType,
        crypto::secp256k1::public_key_to_address,
    };
    use alloy_eips::{BlockHashOrNumber, Encodable2718};
    use alloy_genesis::{Genesis, GenesisAccount};
    use alloy_primitives::{
        Address, B256, BlockNumber, Bytes, TxHash, U256, address, b256, bytes,
        map::foldhash::HashMap,
    };
    use alloy_provider::network::BlockResponse;
    use alloy_rpc_types::TransactionReceipt;
    use alloy_rpc_types_engine::PayloadId;
    use reth::{
        builder::NodeTypesWithDBAdapter,
        chainspec::{Chain, ChainSpec, ChainSpecBuilder, EthChainSpec, MAINNET},
        providers::{AccountReader, BlockNumReader, BlockReader},
        revm::database::StateProviderDatabase,
        transaction_pool::test_utils::TransactionBuilder,
    };
    use reth_db::{DatabaseEnv, test_utils::TempDatabase};
    use std::str::FromStr;

    use reth_ethereum_primitives::{Block as EthBlock, Receipt};
    use reth_evm::{ConfigureEvm, execute::Executor};
    use reth_evm_ethereum::EthEvmConfig;
    use reth_node_ethereum::EthereumNode;
    use reth_primitives::TransactionSigned;
    use reth_primitives_traits::{Account, Block, RecoveredBlock, SealedHeader};
    use reth_provider::{
        BlockWriter, ChainSpecProvider, ExecutionOutcome, LatestStateProviderRef, ProviderFactory,
        providers::BlockchainProvider, test_utils::create_test_provider_factory_with_node_types,
    };
    use std::{collections::BTreeMap, sync::Arc, time::Duration};
    use tokio::time::sleep;

    const TRANSFER_ETH_HASH: TxHash =
        b256!("0x706bbbf402a4f55831d250c77be8f368e16d9b63df9d58561cea8d1f2b59030b");

    const TRANSFER_ETH_TX: Bytes = bytes!(
        "0x02f86b0180806482520894deadbeefdeadbeefdeadbeefdeadbeefdeadbeef8902b5e3af16b188000080c001a0c18767bf03c514933cfec05f2c9a354bf4e8eaafe2e4e7c86836bfc0fb62ad42a02b291b32c588337b7b45420076433157a440bb97afebb154988986527a6ef535"
    );

    // The amount of time to wait (in milliseconds) after sending a new flashblock or canonical
    // block so it can be processed by the state processor
    const SLEEP_TIME: u64 = 10;

    #[derive(Eq, PartialEq, Debug, Hash, Clone, Copy)]
    enum User {
        Alice,
        Bob,
        Charlie,
    }

    type NodeTypes = NodeTypesWithDBAdapter<EthereumNode, Arc<TempDatabase<DatabaseEnv>>>;

    #[derive(Debug, Clone)]
    struct TestHarness {
        flashblocks: FlashblocksState<BlockchainProvider<NodeTypes>>,
        provider: BlockchainProvider<NodeTypes>,
        factory: ProviderFactory<NodeTypes>,
        user_to_address: HashMap<User, Address>,
        user_to_private_key: HashMap<User, B256>,
    }

    impl TestHarness {
        fn address(&self, u: User) -> Address {
            assert!(self.user_to_address.contains_key(&u));
            self.user_to_address[&u]
        }

        fn signer(&self, u: User) -> B256 {
            assert!(self.user_to_private_key.contains_key(&u));
            self.user_to_private_key[&u]
        }

        fn current_canonical_block(&self) -> RecoveredBlock<EthBlock> {
            let latest_block_num =
                self.provider.last_block_number().expect("should be a latest block");

            self.provider
                .block(BlockHashOrNumber::Number(latest_block_num))
                .expect("able to load block")
                .expect("block exists")
                .try_into_recovered()
                .expect("able to recover block")
        }

        fn account_state(&self, u: User) -> Account {
            let basic_account = self
                .provider
                .basic_account(&self.address(u))
                .expect("can lookup account state")
                .expect("should be existing account state");

            let nonce = self
                .flashblocks
                .get_pending_blocks()
                .get_transaction_count(self.address(u))
                .to::<u64>();
            let balance = self
                .flashblocks
                .get_pending_blocks()
                .get_balance(self.address(u))
                .unwrap_or(basic_account.balance);

            Account {
                nonce: nonce + basic_account.nonce,
                balance,
                bytecode_hash: basic_account.bytecode_hash,
            }
        }

        fn build_transaction_to_send_eth(
            &self,
            from: User,
            to: User,
            amount: u128,
        ) -> TransactionSigned {
            TransactionBuilder::default()
                .signer(self.signer(from))
                .chain_id(self.provider.chain_spec().chain_id())
                .to(self.address(to))
                .nonce(self.account_state(from).nonce)
                .value(amount)
                .gas_limit(21_000)
                .max_fee_per_gas(2_000_000_000) // 2 gwei
                .into_eip1559()
        }

        fn build_transaction_to_send_eth_with_nonce(
            &self,
            from: User,
            to: User,
            amount: u128,
            nonce: u64,
        ) -> TransactionSigned {
            TransactionBuilder::default()
                .signer(self.signer(from))
                .chain_id(self.provider.chain_spec().chain_id())
                .to(self.address(to))
                .nonce(nonce)
                .value(amount)
                .gas_limit(21_000)
                .max_fee_per_gas(2_000_000_000) // 2 gwei
                .into_eip1559()
        }

        async fn send_flashblock(&self, flashblock: Flashblock) {
            self.flashblocks.on_flashblock_received(flashblock);
            sleep(Duration::from_millis(SLEEP_TIME)).await;
        }

        async fn new_canonical_block_without_processing(
            &mut self,
            mut user_transactions: Vec<TransactionSigned>,
        ) -> RecoveredBlock<EthBlock> {
            let current_tip = self.current_canonical_block();

            let mut transactions: Vec<TransactionSigned> = vec![];
            transactions.append(&mut user_transactions);

            let block: RecoveredBlock<
                alloy_consensus::Block<
                    alloy_consensus::EthereumTxEnvelope<alloy_consensus::TxEip4844>,
                >,
            > = EthBlock::new_sealed(
                SealedHeader::new_unhashed(Header {
                    parent_beacon_block_root: Some(current_tip.hash()),
                    parent_hash: current_tip.hash(),
                    number: current_tip.number() + 1,
                    timestamp: current_tip.header().timestamp() + 2,
                    gas_limit: current_tip.header().gas_limit(),
                    excess_blob_gas: current_tip.header().excess_blob_gas,
                    ..Header::default()
                }),
                BlockBody { transactions, ommers: vec![], withdrawals: None },
            )
            .try_recover()
            .expect("able to recover block");

            let provider = self.factory.provider().unwrap();

            // Execute the block to produce a block execution output
            let mut block_execution_output = EthEvmConfig::ethereum(self.provider.chain_spec())
                .batch_executor(StateProviderDatabase::new(LatestStateProviderRef::new(&provider)))
                .execute(&block)
                .unwrap();

            block_execution_output.state.reverts.sort();

            let execution_outcome = ExecutionOutcome {
                bundle: block_execution_output.state.clone(),
                receipts: vec![block_execution_output.receipts.clone()],
                first_block: block.number,
                requests: vec![block_execution_output.requests.clone()],
            };

            // Commit the block's execution outcome to the database
            let provider_rw = self.factory.provider_rw().unwrap();
            provider_rw
                .append_blocks_with_state(
                    vec![block.clone()],
                    &execution_outcome,
                    Default::default(),
                )
                .unwrap();
            provider_rw.commit().unwrap();

            self.flashblocks.on_canonical_block_received(&block);

            block
        }

        async fn new_canonical_block(&mut self, user_transactions: Vec<TransactionSigned>) {
            let block = self.new_canonical_block_without_processing(user_transactions).await;
            self.flashblocks.on_canonical_block_received(&block);
            sleep(Duration::from_millis(SLEEP_TIME)).await;
        }

        fn new() -> Self {
            // Use correct private keys from standard test mnemonic: "test test test test test test
            // test test test test test junk"
            let alice_signer =
                b256!("0x47e179ec197488593b187f80a00eb0da91f1b9d0b13f8733639f19c30a34926a"); // Account 4: 0x15d34aaf54267db7d7c367839aaf71a00a2c6a65
            let bob_signer =
                b256!("0x47c99abed3324a2707c28affff1267e45918ec8c3f20b8aa892e8b065d2942dd"); // Account 13: 0x1cbd3b2770909d4e10f157cabc84c7264073c9ec
            let charlie_signer =
                b256!("0xdbda1821b80551c9d65939329250298aa3472ba22feea921c0cf5d620ea67b97"); // Account 8: 0x23618e81e3f5cdf7f54c3d65f7fbc0abf5b21e8f

            let alice = address!("15d34aaf54267db7d7c367839aaf71a00a2c6a65");
            let bob = address!("1cbd3b2770909d4e10f157cabc84c7264073c9ec");
            let charlie = address!("23618e81e3f5cdf7f54c3d65f7fbc0abf5b21e8f");

            let genesis: Genesis =
                serde_json::from_str(include_str!("assets/genesis.json")).unwrap();
            let chainspec = Arc::new(ChainSpec::from_genesis(genesis));
            let factory = create_test_provider_factory::<EthereumNode>(chainspec.clone());
            assert!(reth_db_common::init::init_genesis(&factory).is_ok());

            let provider =
                BlockchainProvider::new(factory.clone()).expect("able to setup provider");

            let block = provider
                .block(BlockHashOrNumber::Number(0))
                .expect("able to load block")
                .expect("block exists")
                .try_into_recovered()
                .expect("able to recover block");

            let flashblocks = FlashblocksState::new(provider.clone(), chainspec.clone(), 5);
            flashblocks.start();

            flashblocks.on_canonical_block_received(&block);

            Self {
                factory,
                flashblocks,
                provider,
                user_to_address: {
                    let mut res = HashMap::default();
                    res.insert(User::Alice, alice);
                    res.insert(User::Bob, bob);
                    res.insert(User::Charlie, charlie);
                    res
                },
                user_to_private_key: {
                    let mut res = HashMap::default();
                    res.insert(User::Alice, alice_signer);
                    res.insert(User::Bob, bob_signer);
                    res.insert(User::Charlie, charlie_signer);
                    res
                },
            }
        }
    }

    struct FlashblockBuilder {
        transactions: Vec<Bytes>,
        receipts: HashMap<B256, Receipt>,
        harness: TestHarness,
        canonical_block_number: Option<BlockNumber>,
        index: u64,
    }

    impl FlashblockBuilder {
        pub fn new_base(harness: &TestHarness) -> Self {
            Self {
                canonical_block_number: None,
                transactions: vec![TRANSFER_ETH_TX],
                receipts: {
                    let mut receipts = HashMap::default();
                    receipts.insert(
                        TRANSFER_ETH_HASH,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true,
                            cumulative_gas_used: 21000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                index: 0,
                harness: harness.clone(),
            }
        }
        pub fn new(harness: &TestHarness, index: u64) -> Self {
            Self {
                canonical_block_number: None,
                transactions: Vec::new(),
                receipts: HashMap::default(),
                harness: harness.clone(),
                index,
            }
        }

        pub fn with_receipts(&mut self, receipts: HashMap<B256, Receipt>) -> &mut Self {
            self.receipts = receipts;
            self
        }

        pub fn with_transactions(&mut self, transactions: Vec<TransactionSigned>) -> &mut Self {
            assert_ne!(self.index, 0, "Cannot set txns for initial flashblock");
            self.transactions.clear();

            let mut cumulative_gas_used = 21000;
            for txn in transactions.iter() {
                cumulative_gas_used = cumulative_gas_used + txn.gas_limit();
                self.transactions.push(txn.encoded_2718().into());
                self.receipts.insert(
                    txn.hash().clone(),
                    Receipt {
                        tx_type: TxType::Eip1559,
                        success: true.into(),
                        cumulative_gas_used,
                        logs: vec![],
                    },
                );
            }
            self
        }

        pub fn with_canonical_block_number(&mut self, num: BlockNumber) -> &mut Self {
            self.canonical_block_number = Some(num);
            self
        }

        pub fn build(&self) -> Flashblock {
            let current_block = self.harness.current_canonical_block();
            let canonical_block_num =
                self.canonical_block_number.unwrap_or_else(|| current_block.number) + 1;

            let base = if self.index == 0 {
                Some(ExecutionPayloadBaseV1 {
                    parent_beacon_block_root: current_block.hash(),
                    parent_hash: current_block.hash(),
                    fee_recipient: Address::random(),
                    prev_randao: B256::random(),
                    block_number: canonical_block_num,
                    gas_limit: current_block.gas_limit,
                    timestamp: current_block.timestamp + 2,
                    extra_data: Bytes::new(),
                    base_fee_per_gas: U256::from(100),
                })
            } else {
                None
            };

            Flashblock {
                payload_id: PayloadId::default(),
                index: self.index,
                base,
                diff: ExecutionPayloadFlashblockDeltaV1 {
                    state_root: B256::default(),
                    receipts_root: B256::default(),
                    block_hash: B256::default(),
                    gas_used: 0,
                    withdrawals: Vec::new(),
                    logs_bloom: Default::default(),
                    transactions: self.transactions.clone(),
                    blob_gas_used: 0,
                    excess_blob_gas: 0,
                },
                metadata: Metadata {
                    block_number: canonical_block_num,
                    receipts: self.receipts.clone(),
                    new_account_balances: HashMap::default(),
                },
            }
        }
    }

    #[tokio::test]
    async fn test_state_overrides_persisted_across_flashblocks() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("block is built")
                .transactions
                .len(),
            1
        );

        assert!(test.flashblocks.get_pending_blocks().get_state_overrides().is_some());
        assert!(
            !test
                .flashblocks
                .get_pending_blocks()
                .get_state_overrides()
                .unwrap()
                .contains_key(&test.address(User::Alice))
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        )
        .await;

        let pending = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 2);

        let overrides = test
            .flashblocks
            .get_pending_blocks()
            .get_state_overrides()
            .expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );

        test.send_flashblock(FlashblockBuilder::new(&test, 2).build()).await;

        let overrides = test
            .flashblocks
            .get_pending_blocks()
            .get_state_overrides()
            .expect("should be set from txn execution in flashblock index 1");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );
    }

    #[tokio::test]
    async fn test_state_overrides_persisted_across_blocks() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        let initial_base = FlashblockBuilder::new_base(&test).build();
        let initial_block_number = initial_base.metadata.block_number;
        test.send_flashblock(initial_base).await;
        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("block is built")
                .transactions
                .len(),
            1
        );

        assert!(test.flashblocks.get_pending_blocks().get_state_overrides().is_some());
        assert!(
            !test
                .flashblocks
                .get_pending_blocks()
                .get_state_overrides()
                .unwrap()
                .contains_key(&test.address(User::Alice))
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        )
        .await;

        let pending = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 2);

        let overrides = test
            .flashblocks
            .get_pending_blocks()
            .get_state_overrides()
            .expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );

        test.send_flashblock(
            FlashblockBuilder::new_base(&test)
                .with_canonical_block_number(initial_block_number)
                .build(),
        )
        .await;

        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("block is built")
                .transactions
                .len(),
            1
        );
        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("block is built")
                .header
                .number,
            initial_block_number + 1
        );

        assert!(test.flashblocks.get_pending_blocks().get_state_overrides().is_some());
        assert!(
            test.flashblocks
                .get_pending_blocks()
                .get_state_overrides()
                .unwrap()
                .contains_key(&test.address(User::Alice))
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_canonical_block_number(initial_block_number)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        )
        .await;

        let overrides = test
            .flashblocks
            .get_pending_blocks()
            .get_state_overrides()
            .expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from(1000000000000000000100000u128)
        );
    }

    #[tokio::test]
    async fn test_only_current_pending_state_cleared_upon_canonical_block_reorg() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("block is built")
                .transactions
                .len(),
            1
        );
        assert!(test.flashblocks.get_pending_blocks().get_state_overrides().is_some());
        assert!(
            !test
                .flashblocks
                .get_pending_blocks()
                .get_state_overrides()
                .unwrap()
                .contains_key(&test.address(User::Alice))
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        )
        .await;
        let pending = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 2);

        let overrides = test
            .flashblocks
            .get_pending_blocks()
            .get_state_overrides()
            .expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );

        test.send_flashblock(
            FlashblockBuilder::new_base(&test).with_canonical_block_number(1).build(),
        )
        .await;
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_canonical_block_number(1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        )
        .await;
        let pending = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 1);

        let overrides =
            test.flashblocks.get_pending_blocks().get_state_overrides().expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from(1000000000000000000100000u128)
        );

        test.new_canonical_block(vec![test.build_transaction_to_send_eth_with_nonce(
            User::Alice,
            User::Bob,
            100,
            0,
        )])
        .await;

        let pending = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 1);

        let overrides =
            test.flashblocks.get_pending_blocks().get_state_overrides().expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from(1000000000000000000100000u128)
        );
    }

    #[tokio::test]
    async fn test_missing_receipts_will_not_process() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let current_block = test.flashblocks.get_pending_blocks().get_block(true);

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100,
                )])
                .with_receipts(HashMap::default()) // Clear the receipts
                .build(),
        )
        .await;

        let pending_block = test.flashblocks.get_pending_blocks().get_block(true);

        // When the flashblock is invalid, the chain doesn't progress
        assert_eq!(pending_block.unwrap().hash(), current_block.unwrap().hash());
    }

    #[tokio::test]
    async fn test_flashblock_for_new_canonical_block_clears_older_flashblocks_if_non_zero_index() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let current_block =
            test.flashblocks.get_pending_blocks().get_block(true).expect("should be a block");

        assert_eq!(current_block.header().number, 1);
        assert_eq!(current_block.transactions.len(), 1);

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1).with_canonical_block_number(100).build(),
        )
        .await;

        let current_block = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(current_block.is_none());
    }

    #[tokio::test]
    async fn test_flashblock_for_new_canonical_block_works_if_sequential() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let current_block =
            test.flashblocks.get_pending_blocks().get_block(true).expect("should be a block");

        assert_eq!(current_block.header().number, 1);
        assert_eq!(current_block.transactions.len(), 1);

        test.send_flashblock(
            FlashblockBuilder::new_base(&test).with_canonical_block_number(1).build(),
        )
        .await;

        let current_block =
            test.flashblocks.get_pending_blocks().get_block(true).expect("should be a block");

        assert_eq!(current_block.header().number, 2);
        assert_eq!(current_block.transactions.len(), 1);
    }

    #[tokio::test]
    async fn test_non_sequential_payload_clears_pending_state() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        assert!(test.flashblocks.get_pending_blocks().get_block(true).is_none());

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        // Just the block info transaction
        assert_eq!(
            test.flashblocks
                .get_pending_blocks()
                .get_block(true)
                .expect("should be set")
                .transactions
                .len(),
            1
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 3)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100,
                )])
                .build(),
        )
        .await;

        assert_eq!(test.flashblocks.get_pending_blocks().is_none(), true);
    }

    #[tokio::test]
    async fn test_duplicate_flashblock_ignored() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let fb = FlashblockBuilder::new(&test, 1)
            .with_transactions(vec![test.build_transaction_to_send_eth(
                User::Alice,
                User::Bob,
                100_000,
            )])
            .build();

        test.send_flashblock(fb.clone()).await;
        let block = test.flashblocks.get_pending_blocks().get_block(true);

        test.send_flashblock(fb.clone()).await;
        let block_two = test.flashblocks.get_pending_blocks().get_block(true);

        assert_eq!(block, block_two);
    }

    #[tokio::test]
    async fn test_progress_canonical_blocks_without_flashblocks() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        let genesis_block = test.current_canonical_block();
        assert_eq!(genesis_block.number, 0);
        assert_eq!(genesis_block.transaction_count(), 0);
        assert!(test.flashblocks.get_pending_blocks().get_block(true).is_none());

        test.new_canonical_block(vec![test.build_transaction_to_send_eth(
            User::Alice,
            User::Bob,
            100,
        )])
        .await;

        let block_one = test.current_canonical_block();
        assert_eq!(block_one.number, 1);
        assert_eq!(block_one.transaction_count(), 1);
        assert!(test.flashblocks.get_pending_blocks().get_block(true).is_none());

        test.new_canonical_block(vec![
            test.build_transaction_to_send_eth(User::Bob, User::Charlie, 100),
            test.build_transaction_to_send_eth(User::Charlie, User::Alice, 1000),
        ])
        .await;

        let block_two = test.current_canonical_block();
        assert_eq!(block_two.number, 2);
        assert_eq!(block_two.transaction_count(), 2);
        assert!(test.flashblocks.get_pending_blocks().get_block(true).is_none());
    }
}
