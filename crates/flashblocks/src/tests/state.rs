#[cfg(test)]
mod tests {
    use crate::{
        payload::{
            ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
        }, rpc::FlashblocksAPI, service::FlashblocksReceiver, state::FlashblocksState
    };
    use alloy_consensus::{crypto::secp256k1::public_key_to_address, BlockBody, Header, Transaction, TxType};
    use alloy_eips::{BlockHashOrNumber, Encodable2718};
    use alloy_genesis::{Genesis, GenesisAccount};
    use alloy_primitives::{
        Address, B256, BlockNumber, Bytes, TxHash, U256, b256, map::foldhash::HashMap,
    };
    use alloy_provider::network::BlockResponse;
    use alloy_rpc_types::TransactionReceipt;
    use alloy_rpc_types_engine::PayloadId;
    use reth::{
        builder::NodeTypesWithDBAdapter,
        chainspec::{Chain, ChainSpecBuilder},
        providers::{AccountReader, BlockNumReader, BlockReader},
        transaction_pool::test_utils::TransactionBuilder,
    };
    use reth_db::{DatabaseEnv, test_utils::TempDatabase};

    use reth_ethereum_primitives::{Block as EthBlock, Receipt};
    use reth_node_ethereum::EthereumNode;
    use reth_primitives::TransactionSigned;
    use reth_primitives_traits::{Account, Block, RecoveredBlock, SealedHeader};
    use reth_provider::{
        providers::BlockchainProvider, test_utils::create_test_provider_factory_with_node_types,
    };
    use std::{collections::BTreeMap, sync::Arc};

    const TRANSFER_ETH_HASH: TxHash =
        b256!("0xbb079fbde7d12fd01664483cd810e91014113e405247479e5615974ebca93e4a");

    #[derive(Eq, PartialEq, Debug, Hash, Clone, Copy)]
    enum User {
        Alice,
        Bob,
        Charlie,
    }

    struct TestHarness {
        flashblocks: FlashblocksState<
            BlockchainProvider<
                NodeTypesWithDBAdapter<EthereumNode, Arc<TempDatabase<DatabaseEnv>>>,
            >,
        >,
        provider: BlockchainProvider<
            NodeTypesWithDBAdapter<EthereumNode, Arc<TempDatabase<DatabaseEnv>>>,
        >,

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
            self.provider
                .basic_account(&self.address(u))
                .expect("can lookup account state")
                .expect("should be existing account state")
        }

        fn build_transaction_to_send_eth(
            &self,
            from: User,
            to: User,
            amount: u128,
        ) -> TransactionSigned {
            TransactionBuilder::default()
                .signer(self.signer(from))
                .to(self.address(to))
                .nonce(self.account_state(from).nonce)
                .value(amount)
                .gas_limit(21_000)
                .into_eip1559()
        }

        fn new() -> Arc<Self> {
            let keys = reth_testing_utils::generators::generate_keys(&mut rand::rng(), 3);
            let alice_signer = keys[0];
            let bob_signer = keys[1];
            let charli_signer = keys[2];

            let alice = public_key_to_address(alice_signer.public_key());
            let bob = public_key_to_address(bob_signer.public_key());
            let charlie = public_key_to_address(charli_signer.public_key());

            let items = vec![
                (alice, GenesisAccount::default().with_balance(U256::from(100_000_000))),
                (bob, GenesisAccount::default().with_balance(U256::from(100_000_000))),
                (charlie, GenesisAccount::default().with_balance(U256::from(100_000_000))),
            ];

            let genesis = Genesis::default().with_gas_limit(100_000_000).extend_accounts(items);
            let chain_spec =
                Arc::new(ChainSpecBuilder::default().chain(Chain::dev()).genesis(genesis).prague_activated().build());
            let factory =
                create_test_provider_factory_with_node_types::<EthereumNode>(chain_spec.clone());

            assert!(reth_db_common::init::init_genesis(&factory).is_ok());

            let provider = BlockchainProvider::new(factory).expect("able to setup provider");

            let block = provider
                .block(BlockHashOrNumber::Number(0))
                .expect("able to load block")
                .expect("block exists")
                .try_into_recovered()
                .expect("able to recover block");

            let flashblocks = FlashblocksState::new(provider.clone(), chain_spec.clone());

            flashblocks.on_canonical_block_received(&block);

            Arc::new(Self {
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
                    res.insert(User::Alice, alice_signer.secret_bytes().into());
                    res.insert(User::Bob, bob_signer.secret_bytes().into());
                    res.insert(User::Charlie, charli_signer.secret_bytes().into());
                    res
                },
            })
        }
    }

    struct FlashblockBuilder {
        transactions: Vec<Bytes>,
        receipts: BTreeMap<B256, Receipt>,
        harness: Arc<TestHarness>,
        canonical_block_number: Option<BlockNumber>,
        index: u64,
    }

    impl FlashblockBuilder {
        pub fn new_base(harness: &Arc<TestHarness>) -> Self {
            Self {
                canonical_block_number: None,
                transactions: vec![],
                receipts: {
                    let mut receipts = BTreeMap::default();
                    receipts.insert(
                        TRANSFER_ETH_HASH,
                        Receipt {
                            tx_type: TxType::Eip1559,
                            success: true,
                            cumulative_gas_used: 24000,
                            logs: vec![],
                        },
                    );
                    receipts
                },
                index: 0,
                harness: harness.clone(),
            }
        }
        pub fn new(harness: &Arc<TestHarness>, index: u64) -> Self {
            Self {
                canonical_block_number: None,
                transactions: Vec::new(),
                receipts: BTreeMap::default(),
                harness: harness.clone(),
                index,
            }
        }

        pub fn with_receipts(&mut self, receipts: BTreeMap<B256, Receipt>) -> &mut Self {
            self.receipts = receipts;
            self
        }

        pub fn with_transactions(&mut self, transactions: Vec<TransactionSigned>) -> &mut Self {
            assert_ne!(self.index, 0, "Cannot set txns for initial flashblock");
            self.transactions.clear();

            let mut cumulative_gas_used = 0;
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

        pub fn build(&self) -> FlashBlock {
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

            FlashBlock {
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
                    new_account_balances: BTreeMap::default(),
                },
            }
        }
    }

    #[test]
    fn test_state_overrides_persisted_across_flashblocks() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

        assert_eq!(test.flashblocks.get_block(true).expect("block is built").transactions.len(), 1);

        assert!(test.flashblocks.get_state_overrides().is_some());
        assert!(
            !test
                .flashblocks
                .get_state_overrides()
                .unwrap()
                .contains_key(&test.address(User::Alice))
        );

        test.flashblocks.on_flashblock_received(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100_000,
                )])
                .build(),
        );

        let pending = test.flashblocks.get_block(true);
        assert!(pending.is_some());
        let pending = pending.unwrap();
        assert_eq!(pending.transactions.len(), 2);

        let overrides =
            test.flashblocks.get_state_overrides().expect("should be set from txn execution");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from(100_100_000)
        );

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new(&test, 2).build());

        let overrides = test
            .flashblocks
            .get_state_overrides()
            .expect("should be set from txn execution in flashblock index 1");

        assert!(overrides.get(&test.address(User::Alice)).is_some());
        assert_eq!(
            overrides
                .get(&test.address(User::Bob))
                .expect("should be set as txn receiver")
                .balance
                .expect("should be changed due to receiving funds"),
            U256::from(100_100_000)
        );
    }

    #[test]
    fn test_missing_receipts_will_not_process() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

        let current_block = test.flashblocks.get_block(true);

        test.flashblocks.on_flashblock_received(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100,
                )])
                .with_receipts(BTreeMap::default()) // Clear the receipts
                .build(),
        );

        let pending_block = test.flashblocks.get_block(true);

        // When the flashblock is invalid, the chain doesn't progress
        assert_eq!(pending_block.unwrap().hash(), current_block.unwrap().hash());
    }

    #[test]
    fn test_flashblock_for_new_canonical_block_clears_older_flashblocks() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

        let current_block = test.flashblocks.get_block(true).expect("should be a block");

        assert_eq!(current_block.header().number, 1);
        assert_eq!(current_block.transactions.len(), 1);

        test.flashblocks.on_flashblock_received(
            FlashblockBuilder::new(&test, 1).with_canonical_block_number(100).build(),
        );

        let current_block = test.flashblocks.get_block(true);
        assert!(current_block.is_none());
    }

    #[test]
    fn test_non_sequential_payload_ignored() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        assert!(test.flashblocks.get_block(true).is_none());

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

        // Just the block info transaction
        assert_eq!(test.flashblocks.get_block(true).expect("should be set").transactions.len(), 1);

        test.flashblocks.on_flashblock_received(
            FlashblockBuilder::new(&test, 3)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    100,
                )])
                .build(),
        );

        // Still the block info transaction, the txns in the third payload are ignored as it's
        // missing a Flashblock
        assert_eq!(test.flashblocks.get_block(true).expect("should be set").transactions.len(), 1);
    }

    // #[test]
    // fn test_canonical_block_clears_older_flashblocks() {
    //     reth_tracing::init_test_tracing();
    //     let test = TestHarness::new();

    //     test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

    //     assert_eq!(
    //         test.flashblocks.get_block(true).expect("should have pending block one").number(),
    //         1
    //     );

    //     let block = Block::new_sealed(
    //         SealedHeader::new_unhashed(Header {
    //             number: 3, // Greater than the current Flashblock block number of 1
    //             ..Header::default()
    //         }),
    //         BlockBody::default(),
    //     )
    //     .try_recover()
    //     .expect("able to recover block");

    //     test.flashblocks.on_canonical_block_received(&block);

    //     // Flashblock is cleared, as canonical data is more recent
    //     assert!(test.flashblocks.get_block(true).is_none());
    // }

    #[test]
    fn test_duplicate_flashblock_ignored() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        test.flashblocks.on_flashblock_received(FlashblockBuilder::new_base(&test).build());

        let fb = FlashblockBuilder::new(&test, 1)
            .with_transactions(vec![test.build_transaction_to_send_eth(
                User::Alice,
                User::Bob,
                100_000,
            )])
            .build();

        test.flashblocks.on_flashblock_received(fb.clone());
        let block = test.flashblocks.get_block(true);

        test.flashblocks.on_flashblock_received(fb.clone());
        let block_two = test.flashblocks.get_block(true);

        assert_eq!(block, block_two);
    }
}
