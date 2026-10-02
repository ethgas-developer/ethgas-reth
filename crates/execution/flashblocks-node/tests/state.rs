//! Canonical-block / flashblock state reconciliation tests.

#[cfg(test)]
mod tests {
    use alloy_consensus::{BlockBody, BlockHeader, Header, Transaction, TxType};
    use alloy_eips::{BlockHashOrNumber, BlockNumberOrTag, Encodable2718};
    use alloy_genesis::Genesis;
    use alloy_primitives::{
        Address, B256, BlockNumber, Bytes, TxKind, U256, address, b256, map::foldhash::HashMap,
    };
    use alloy_provider::network::BlockResponse;
    use alloy_rpc_types_engine::PayloadId;
    use alloy_sol_types::SolCall;
    use ethgas_node_runner::test_utils::{PendingProbe, slot_number_at};
    use ethgas_reth_flashblocks::{
        FlashblocksAPI, FlashblocksReceiver, FlashblocksState, PendingBlocksAPI,
        payload::{
            ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
        },
    };
    use reth_chain_state::{ExecutedBlock, NewCanonicalChain};
    use reth_chainspec::{ChainSpec, EthChainSpec};
    use reth_db::{DatabaseEnv, test_utils::TempDatabase};
    use reth_network_p2p::sync::NoopSyncStateUpdater;
    use reth_node_api::NodeTypesWithDBAdapter;
    use reth_provider::{AccountReader, BlockNumReader, BlockReader};
    use reth_revm::{database::StateProviderDatabase, state::AccountInfo};
    use reth_transaction_pool::test_utils::TransactionBuilder;
    use reth_trie_common::{
        ComputedTrieData, HashedPostState, KeccakKeyHasher, updates::TrieUpdates,
    };
    use std::str::FromStr;

    use reth_ethereum_primitives::{Block as EthBlock, Receipt, TransactionSigned};
    use reth_evm::{ConfigureEvm, execute::Executor};
    use reth_evm_ethereum::EthEvmConfig;
    use reth_node_ethereum::EthereumNode;
    use reth_primitives_traits::{Account, Block, RecoveredBlock, SealedHeader};
    use reth_provider::{
        BlockWriter, ChainSpecProvider, ExecutionOutcome, LatestStateProviderRef, ProviderFactory,
        StateProvider, StateProviderFactory, providers::BlockchainProvider,
        test_utils::create_test_provider_factory_with_node_types,
    };
    use std::{sync::Arc, time::Duration};
    use tokio::time::sleep;

    /// Sign the one transaction every base flashblock carries, a key per block number, so no
    /// transaction repeats across the blocks of a pending span and each stays valid whichever of
    /// those blocks the chain keeps. The first four accounts of the standard test mnemonic, funded
    /// at genesis and never a `User`.
    const BASE_SIGNERS: [B256; 4] = [
        b256!("0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80"),
        b256!("0x59c6995e998f97a5a0044966f0945389dc9e86dae88c7a8412f4603b6b78690d"),
        b256!("0x5de4111afa1a4b94908f83103eb1f1706367c2e68ca870fc3fb9a804cdab365a"),
        b256!("0x7c852118294e51e653712a81e05800f419141751be58f605c371e15141b007a6"),
    ];

    const BASE_TRANSFER_RECIPIENT: Address = address!("0xdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef");

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
        flashblocks: Arc<FlashblocksState>,
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

        fn canonical_account(&self, u: User) -> Account {
            let latest_block_num =
                self.provider.last_block_number().expect("should be a latest block");

            self.provider
                .state_by_block_number_or_tag(BlockNumberOrTag::Number(latest_block_num))
                .expect("can get state at canonical tip")
                .basic_account(&self.address(u))
                .expect("can lookup account state")
                .expect("should be existing account state")
        }

        fn account_state(&self, u: User) -> Account {
            let basic_account = self.canonical_account(u);
            let pending = self.flashblocks.get_pending_blocks();

            let nonce = pending
                .as_ref()
                .and_then(|pending| {
                    pending.bundle_state().account(&self.address(u))?.account_info()
                })
                .map_or(basic_account.nonce, |info| info.nonce);
            let balance = pending.get_balance(self.address(u)).unwrap_or(basic_account.balance);

            Account { nonce, balance, bytecode_hash: basic_account.bytecode_hash }
        }

        /// The account as pending execution left it; `None` when pending execution never touched
        /// it.
        fn pending_account(&self, u: User) -> Option<AccountInfo> {
            let pending = self.flashblocks.get_pending_blocks();
            let bundle = pending.as_ref().expect("a snapshot is published").bundle_state();
            bundle.account(&self.address(u)).and_then(|account| account.account_info())
        }

        fn pending_storage(&self, address: Address, slot: U256) -> Option<U256> {
            let pending = self.flashblocks.get_pending_blocks();
            let bundle = pending.as_ref().expect("a snapshot is published").bundle_state();
            bundle.account(&address).and_then(|account| account.storage_slot(slot))
        }

        fn build_base_transaction(&self, block_number: BlockNumber) -> TransactionSigned {
            TransactionBuilder::default()
                .signer(BASE_SIGNERS[block_number as usize % BASE_SIGNERS.len()])
                .chain_id(self.provider.chain_spec().chain_id())
                .to(BASE_TRANSFER_RECIPIENT)
                .nonce(0)
                .value(1)
                .gas_limit(21_000)
                .max_fee_per_gas(2_000_000_000)
                .into_eip1559()
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

        fn build_deployment_transaction(
            &self,
            from: User,
            bytecode: Bytes,
            nonce: u64,
        ) -> TransactionSigned {
            let mut builder = TransactionBuilder::default()
                .signer(self.signer(from))
                .chain_id(self.provider.chain_spec().chain_id())
                .nonce(nonce)
                .value(0)
                .gas_limit(1_000_000)
                .max_fee_per_gas(2_000_000_000)
                .input(bytecode);
            builder.to = TxKind::Create;
            builder.into_eip1559()
        }

        fn build_contract_call(
            &self,
            from: User,
            to: Address,
            calldata: Bytes,
            nonce: u64,
        ) -> TransactionSigned {
            TransactionBuilder::default()
                .signer(self.signer(from))
                .chain_id(self.provider.chain_spec().chain_id())
                .to(to)
                .nonce(nonce)
                .value(0)
                .gas_limit(1_000_000)
                .max_fee_per_gas(2_000_000_000)
                .input(calldata)
                .into_eip1559()
        }

        async fn send_flashblock(&self, flashblock: FlashBlock) {
            self.flashblocks.on_flashblock_received(flashblock);
            sleep(Duration::from_millis(SLEEP_TIME)).await;
        }

        #[allow(clippy::needless_pass_by_ref_mut)]
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
                .batch_executor(StateProviderDatabase::new(
                    LatestStateProviderRef::new(&provider).into_evm_state_provider(),
                ))
                .execute(&block)
                .unwrap();

            block_execution_output.state.reverts.sort();

            let execution_outcome = ExecutionOutcome {
                bundle: block_execution_output.state.clone(),
                receipts: vec![block_execution_output.receipts.clone()],
                first_block: block.number,
                requests: vec![block_execution_output.requests.clone()],
            };

            let hashed_state = HashedPostState::from_bundle_state::<KeccakKeyHasher>(
                block_execution_output.state.state(),
            );

            // Commit the block's execution outcome to the database
            let provider_rw = self.factory.provider_rw().unwrap();
            provider_rw
                .append_blocks_with_state(
                    vec![block.clone()],
                    &execution_outcome,
                    hashed_state.clone().into_sorted(),
                )
                .unwrap();
            provider_rw.commit().unwrap();

            // Advance the `BlockchainProvider`'s in-memory canonical state so `latest()` (and hence
            // `basic_account`) reflects the new block. Under reth v2 the manual `provider_rw`
            // commit alone does not update the in-memory head, leaving canonical reads
            // stale at genesis.
            let executed = ExecutedBlock::new(
                Arc::new(block.clone()),
                Arc::new(block_execution_output),
                ComputedTrieData::new(
                    Arc::new(hashed_state.into_sorted()),
                    Arc::new(TrieUpdates::default().into_sorted()),
                ),
            );
            self.provider
                .canonical_in_memory_state()
                .update_chain(NewCanonicalChain::Commit { new: vec![executed] });

            // `update_chain` only fills the in-memory block map. The canonical head lives in the
            // chain info tracker, which is what `best_block_number()` reports, so set it too or
            // every tip read stays at genesis.
            self.provider
                .canonical_in_memory_state()
                .set_canonical_head(block.clone_sealed_header());

            // NOTE: this method intentionally does NOT notify the StateProcessor
            // (`on_canonical_block_received`). It only commits the block to the underlying chain so
            // tests can reproduce the race where `latest` advanced but pending state is not yet
            // reconciled. The `new_canonical_block` wrapper performs the notification.
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
            let factory = create_test_provider_factory_with_node_types::<EthereumNode>(chainspec);
            assert!(reth_db_common::init::init_genesis(&factory).is_ok());

            let provider =
                BlockchainProvider::new(factory.clone()).expect("able to setup provider");

            let block = provider
                .block(BlockHashOrNumber::Number(0))
                .expect("able to load block")
                .expect("block exists")
                .try_into_recovered()
                .expect("able to recover block");

            let flashblocks = Arc::new(FlashblocksState::new(3));
            flashblocks.start(provider.clone(), NoopSyncStateUpdater::default());

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
        fn new_base(harness: &TestHarness) -> Self {
            Self::new(harness, 0)
        }
        fn new(harness: &TestHarness, index: u64) -> Self {
            Self {
                canonical_block_number: None,
                transactions: Vec::new(),
                receipts: HashMap::default(),
                harness: harness.clone(),
                index,
            }
        }

        fn with_receipts(&mut self, receipts: HashMap<B256, Receipt>) -> &mut Self {
            self.receipts = receipts;
            self
        }

        fn with_transactions(&mut self, transactions: Vec<TransactionSigned>) -> &mut Self {
            assert_ne!(self.index, 0, "Cannot set txns for initial flashblock");
            self.transactions.clear();

            let mut cumulative_gas_used = 21000;
            for txn in &transactions {
                cumulative_gas_used += txn.gas_limit();
                self.transactions.push(txn.encoded_2718().into());
                self.receipts.insert(
                    *txn.hash(),
                    Receipt {
                        tx_type: TxType::Eip1559,
                        success: true,
                        cumulative_gas_used,
                        logs: vec![],
                    },
                );
            }
            self
        }

        fn with_canonical_block_number(&mut self, num: BlockNumber) -> &mut Self {
            self.canonical_block_number = Some(num);
            self
        }

        fn build(&self) -> FlashBlock {
            let current_block = self.harness.current_canonical_block();
            let canonical_block_num =
                self.canonical_block_number.unwrap_or_else(|| current_block.number) + 1;

            let mut transactions = self.transactions.clone();
            let mut receipts = self.receipts.clone();
            if self.index == 0 {
                let base_tx = self.harness.build_base_transaction(canonical_block_num);
                receipts.insert(
                    *base_tx.tx_hash(),
                    Receipt {
                        tx_type: TxType::Eip1559,
                        success: true,
                        cumulative_gas_used: 21000,
                        logs: vec![],
                    },
                );
                transactions.insert(0, base_tx.encoded_2718().into());
            }

            let timestamp = current_block.timestamp + 2;
            let base = (self.index == 0).then(|| ExecutionPayloadBaseV1 {
                parent_beacon_block_root: current_block.hash(),
                parent_hash: current_block.hash(),
                fee_recipient: Address::random(),
                prev_randao: B256::random(),
                block_number: canonical_block_num,
                gas_limit: current_block.gas_limit,
                timestamp,
                extra_data: Bytes::new(),
                base_fee_per_gas: U256::from(100),
                slot_number: Some(slot_number_at(timestamp)),
            });

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
                    transactions,
                    blob_gas_used: 0,
                    excess_blob_gas: 0,
                    requests: None,
                },
                metadata: Metadata {
                    block_number: canonical_block_num,
                    receipts,
                    new_account_balances: HashMap::default(),
                    inclusion_fee: None,
                },
            }
        }
    }

    #[tokio::test]
    async fn test_pending_state_persisted_across_flashblocks() {
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

        assert!(test.pending_account(User::Alice).is_none());

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

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 1);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );

        test.send_flashblock(FlashblockBuilder::new(&test, 2).build()).await;

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 1);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
            U256::from_str("1000000000000000000100000").unwrap() /* Genesis balance (1M ETH) +
                                                                  * 100k wei received */
        );
    }

    #[tokio::test]
    async fn test_pending_state_persisted_across_blocks() {
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

        assert!(test.pending_account(User::Alice).is_none());

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

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 1);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
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

        assert_eq!(
            test.pending_account(User::Alice).expect("the first block touched the sender").nonce,
            1,
            "the second block's base carries the first block's state"
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

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 2);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
            // Pending blocks stack: Bob received 100k in block N and another 100k in block N+1.
            U256::from(1000000000000000000200000u128)
        );
    }

    // Exercises canonical reconciliation: after a canonical block arrives, only the matching
    // pending state is cleared and the remaining pending flashblock delta is restacked on the
    // new canonical balance. Requires the harness to advance the `BlockchainProvider`'s
    // in-memory canonical head (see `new_canonical_block_without_processing`), since the manual
    // `provider_rw` commit alone does not under reth v2.
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
        assert!(test.pending_account(User::Alice).is_none());

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

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 1);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
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
        assert_eq!(pending.transactions.len(), 2);

        assert_eq!(test.pending_account(User::Alice).expect("the sender is touched").nonce, 2);
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
            // Pending blocks stack: Bob received 100k in block N and another 100k in block N+1.
            U256::from(1000000000000000000200000u128)
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
        assert_eq!(pending.transactions.len(), 2);

        assert_eq!(
            test.pending_account(User::Alice).expect("the sender is touched").nonce,
            2,
            "the canonical transfer and the surviving flashblock transfer"
        );
        assert_eq!(
            test.pending_account(User::Bob).expect("the receiver is touched").balance,
            // After reconciliation Bob's pending balance is his *canonical* balance plus the
            // surviving flashblock delta: genesis + 100 (received in the canonical block) +
            // 100_000 (the still-pending flashblock transfer). Mirrors base's
            // `expected_pending_balance`.
            U256::from(1000000000000000000100100u128)
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
        let current = current_block.unwrap();
        let pending = pending_block.unwrap();
        assert_eq!(pending.header.number, current.header.number);
        assert_eq!(pending.transactions, current.transactions);
    }

    /// A flashblock whose wire receipts report a falling cumulative gas is rejected, and the
    /// processor goes on to apply the next valid flashblock.
    #[tokio::test]
    async fn test_a_falling_cumulative_gas_rejects_the_flashblock() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();
        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let transfer = test.build_transaction_to_send_eth(User::Alice, User::Bob, 100);
        let mut receipts = HashMap::default();
        receipts.insert(
            *transfer.tx_hash(),
            Receipt {
                tx_type: TxType::Eip1559,
                success: true,
                // Below the base transaction's 21000.
                cumulative_gas_used: 20_000,
                logs: vec![],
            },
        );
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_transactions(vec![transfer.clone()])
                .with_receipts(receipts)
                .build(),
        )
        .await;
        let pending = test.flashblocks.get_pending_blocks().get_block(true).expect("block");
        assert_eq!(pending.transactions.len(), 1, "the flashblock must be rejected");

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1).with_transactions(vec![transfer]).build(),
        )
        .await;
        let pending = test.flashblocks.get_pending_blocks().get_block(true).expect("block");
        assert_eq!(pending.transactions.len(), 2, "the next valid flashblock must still apply");
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

        assert!(test.flashblocks.get_pending_blocks().is_none());
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

    /// Verifies that transactions in flashblock N+1 can see state changes from flashblock N.
    ///
    /// This test catches database layering bugs where writes from earlier flashblocks
    /// aren't visible to later flashblock execution. The key is that flashblock 2's
    /// transaction uses nonce=1, which only succeeds if the execution layer sees
    /// flashblock 1's transaction (which used nonce=0).
    #[tokio::test]
    async fn test_sequential_nonces_across_flashblocks() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        // Send base flashblock
        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        // Flashblock 1: Alice sends to Bob with nonce 0
        let tx_nonce_0 =
            test.build_transaction_to_send_eth_with_nonce(User::Alice, User::Bob, 1000, 0);
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1).with_transactions(vec![tx_nonce_0]).build(),
        )
        .await;

        // Verify flashblock 1 was processed - Alice's pending nonce should now be 1
        let alice_state = test.account_state(User::Alice);
        assert_eq!(alice_state.nonce, 1, "After flashblock 1, Alice's pending nonce should be 1");

        // Flashblock 2: Alice sends to Charlie with nonce 1
        // This will FAIL if the execution layer can't see flashblock 1's state change
        let tx_nonce_1 =
            test.build_transaction_to_send_eth_with_nonce(User::Alice, User::Charlie, 2000, 1);
        test.send_flashblock(
            FlashblockBuilder::new(&test, 2).with_transactions(vec![tx_nonce_1]).build(),
        )
        .await;

        // Verify flashblock 2 was processed - Alice's pending nonce should now be 2
        let alice_state_after = test.account_state(User::Alice);
        assert_eq!(
            alice_state_after.nonce, 2,
            "After flashblock 2, Alice's pending nonce should be 2. \
             If this fails, the database layering may be preventing flashblock 2 \
             from seeing flashblock 1's state changes."
        );

        // Also verify Bob and Charlie received their funds
        assert_eq!(
            test.pending_account(User::Bob).expect("Bob received funds in flashblock 1").balance,
            test.canonical_account(User::Bob).balance + U256::from(1000)
        );
        assert_eq!(
            test.pending_account(User::Charlie)
                .expect("Charlie received funds in flashblock 2")
                .balance,
            test.canonical_account(User::Charlie).balance + U256::from(2000)
        );
    }

    #[tokio::test]
    async fn test_flashblock_cached_and_applied_after_canonical_block() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        // Send a flashblock targeting block 2 (canonical_block_number=1) before
        // canonical block 1 exists. This triggers MissingCanonicalHeader and should
        // be cached by the processor.
        test.send_flashblock(
            FlashblockBuilder::new_base(&test).with_canonical_block_number(1).build(),
        )
        .await;

        assert!(
            test.flashblocks.get_pending_blocks().is_none(),
            "pending state should be empty because canonical block 1 does not exist yet"
        );

        // Build canonical block 1 so the processor can replay the cached flashblock.
        test.new_canonical_block(vec![]).await;
        let block_one = test.current_canonical_block();
        assert_eq!(block_one.number, 1);

        let pending = test
            .flashblocks
            .get_pending_blocks()
            .get_block(true)
            .expect("cached flashblock replayed");
        assert_eq!(pending.header.number, 2, "replayed flashblock should produce pending block 2");
    }

    #[tokio::test]
    async fn test_cached_flashblock_with_transactions_applied_after_canonical() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        let transfer_amount = 100_000u128;

        // Cache a base flashblock for block 2 (needs canonical block 1).
        test.send_flashblock(
            FlashblockBuilder::new_base(&test).with_canonical_block_number(1).build(),
        )
        .await;
        assert!(test.flashblocks.get_pending_blocks().is_none());

        // Also cache a second flashblock (index 1) with a transaction.
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_canonical_block_number(1)
                .with_transactions(vec![test.build_transaction_to_send_eth(
                    User::Alice,
                    User::Bob,
                    transfer_amount,
                )])
                .build(),
        )
        .await;
        assert!(test.flashblocks.get_pending_blocks().is_none());

        // Provide canonical block 1 to unlock the cache.
        test.new_canonical_block(vec![]).await;

        let pending = test
            .flashblocks
            .get_pending_blocks()
            .get_block(true)
            .expect("cached flashblocks replayed");
        assert_eq!(pending.header.number, 2);
        assert_eq!(pending.transactions.len(), 2, "base tx + Alice->Bob transfer");

        assert_eq!(
            test.pending_account(User::Alice).expect("the replayed transfer touched Alice").nonce,
            1
        );
        assert_eq!(
            test.pending_account(User::Bob).expect("the replayed transfer touched Bob").balance,
            test.canonical_account(User::Bob).balance + U256::from(transfer_amount)
        );
    }

    #[tokio::test]
    async fn test_flashblock_far_ahead_of_canonical_not_cached() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        // Send a flashblock targeting a block far in the future (canonical_block_number=100).
        // This is more than MAX_CACHE_AHEAD_BLOCKS (5) ahead of genesis, so it should NOT
        // be cached and pending state should remain empty.
        test.send_flashblock(
            FlashblockBuilder::new_base(&test).with_canonical_block_number(100).build(),
        )
        .await;

        assert!(
            test.flashblocks.get_pending_blocks().is_none(),
            "flashblock too far ahead should not be cached or produce pending state"
        );
    }

    /// Regression for `117eae2`: `BLOCKHASH` of a parent that is itself pending resolves to the
    /// parent hash the wire advertised. Needs a two-block window — the first pending block's
    /// parent is canonical, so the provider can still answer it.
    #[tokio::test]
    async fn pending_execution_resolves_blockhash_of_pending_parent() {
        reth_tracing::init_test_tracing();
        let test = TestHarness::new();

        let first_base = FlashblockBuilder::new_base(&test).build();
        let first_block = first_base.metadata.block_number;
        test.send_flashblock(first_base).await;

        let second_base =
            FlashblockBuilder::new_base(&test).with_canonical_block_number(first_block).build();
        let parent_hash = second_base.base.as_ref().expect("index 0 carries a base").parent_hash;
        test.send_flashblock(second_base).await;

        let deployer = User::Bob;
        let nonce = test.account_state(deployer).nonce;
        let probe = test.address(deployer).create(nonce);
        let deployment =
            test.build_deployment_transaction(deployer, PendingProbe::BYTECODE.clone(), nonce);
        let record = test.build_contract_call(
            deployer,
            probe,
            PendingProbe::recordParentHashCall {}.abi_encode().into(),
            nonce + 1,
        );

        test.send_flashblock(
            FlashblockBuilder::new(&test, 1)
                .with_canonical_block_number(first_block)
                .with_transactions(vec![deployment, record])
                .build(),
        )
        .await;

        let stored = B256::from(
            test.pending_storage(probe, U256::ZERO).expect("parentHash occupies slot 0"),
        );

        assert_eq!(stored, parent_hash);
        assert_ne!(stored, B256::ZERO, "a zero hash would mean BLOCKHASH missed");
    }

    /// Advances the node without telling the state processor, reproducing the queue-lag shape:
    /// the chain moves on while the processor is still draining old entries.
    async fn advance_tip_without_processing(test: &mut TestHarness, blocks: u64) {
        for _ in 0..blocks {
            test.new_canonical_block_without_processing(vec![]).await;
        }
    }

    #[tokio::test]
    async fn test_stale_pending_dropped_once_node_advances_past_it() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        let anchor_parent = test.current_canonical_block().number;
        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
        let anchored = test.flashblocks.get_pending_blocks().get_block(true);
        assert!(anchored.is_some(), "the base flashblock should publish a snapshot");

        advance_tip_without_processing(&mut test, 5).await;

        // Pinned to the old parent, so this is a genuinely backlogged flashblock continuing the
        // block pending already tracks, not a new one. It must not keep the stranded overlay live.
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1).with_canonical_block_number(anchor_parent).build(),
        )
        .await;

        assert!(
            test.flashblocks.get_pending_blocks().is_none(),
            "a snapshot anchored more than max_depth behind the tip must not stay readable"
        );
    }

    #[tokio::test]
    async fn test_stale_notification_rebases_onto_the_real_tip() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        let genesis = test.current_canonical_block().number;
        for parent in genesis..genesis + 3 {
            test.send_flashblock(
                FlashblockBuilder::new_base(&test).with_canonical_block_number(parent).build(),
            )
            .await;
        }
        // Block genesis+1 lands canonically carrying a transaction pending never tracked, which
        // is what forces a rebuild. The node then moves on without the processor hearing about it.
        let txn = test.build_transaction_to_send_eth(User::Alice, User::Bob, 100);
        let stale_notification = test.new_canonical_block_without_processing(vec![txn]).await;
        advance_tip_without_processing(&mut test, 1).await;
        let tip = test.current_canonical_block().number;
        assert_eq!(stale_notification.number, tip - 1);

        // The rebuild must drop every flashblock the node has already canonicalized, not just
        // those up to the notified block, or pending re-executes an already canonical range.
        test.flashblocks.on_canonical_block_received(&stale_notification);
        sleep(Duration::from_millis(SLEEP_TIME)).await;

        assert_eq!(
            test.flashblocks.get_pending_blocks().get_canonical_block_number(),
            BlockNumberOrTag::Number(tip),
            "pending must rebase onto the real tip, not onto the notified block"
        );
    }

    #[tokio::test]
    async fn test_canonical_notification_drops_snapshot_left_behind() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
        assert!(test.flashblocks.get_pending_blocks().is_some());

        advance_tip_without_processing(&mut test, 5).await;

        // The receiving task drops the stranded snapshot itself, so staleness is bounded by
        // chain progress rather than by how long the processor takes to reach the queue entry.
        let block = test.current_canonical_block();
        test.flashblocks.on_canonical_block_received(&block);

        assert!(test.flashblocks.get_pending_blocks().is_none());
    }

    #[tokio::test]
    async fn test_pending_recovers_at_current_canonical_tip() {
        reth_tracing::init_test_tracing();
        let mut test = TestHarness::new();

        let anchor_parent = test.current_canonical_block().number;
        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;
        advance_tip_without_processing(&mut test, 5).await;
        test.send_flashblock(
            FlashblockBuilder::new(&test, 1).with_canonical_block_number(anchor_parent).build(),
        )
        .await;
        assert!(test.flashblocks.get_pending_blocks().is_none());

        // Recovery needs no mechanism of its own: the next flashblock rooted at the tip rebuilds
        // from an absent snapshot through the existing path.
        let tip = test.current_canonical_block().number;
        test.send_flashblock(FlashblockBuilder::new_base(&test).build()).await;

        let recovered = test
            .flashblocks
            .get_pending_blocks()
            .get_block(true)
            .expect("a tip-rooted flashblock must rebuild pending");
        assert_eq!(recovered.header.number, tip + 1);
    }
}
