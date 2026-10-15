//! Pending state as the snapshot's bundle over the anchor's state, with no copy per read.

use std::{fmt, sync::Arc};

use alloy_primitives::{
    Address, B256, BlockNumber, Bytes, StorageKey, StorageValue, U256, keccak256,
};
use reth_primitives_traits::{Account, Bytecode};
use reth_provider::{
    AccountReader, BlockHashReader, BytecodeReader, HashedPostStateProvider, ProviderResult,
    StateProofProvider, StateProvider, StateProviderBox, StateRootProvider, StorageRootProvider,
};
use reth_revm::db::BundleState;
use reth_trie_common::{
    AccountProof, DecodedMultiProofV2, ExecutionWitnessMode, HashedPostState, HashedStorage,
    MultiProof, MultiProofTargets, MultiProofTargetsV2, StorageMultiProof, StorageProof, TrieInput,
    updates::TrieUpdates,
};

/// Answers each read from the pending bundle first and from the anchor's state for the rest.
///
/// It reads the bundle in place. reth's provider for an appended block copies the whole bundle
/// into a new overlay on the first read of each provider, and `pending` opens one provider per
/// call. The answers follow that overlay's rules: an account in the bundle is answered from the
/// bundle, and its unknown slots read as zero only when the bundle destroyed it.
pub(crate) struct PendingStateProvider {
    anchor: StateProviderBox,
    bundle: Arc<BundleState>,
}

impl PendingStateProvider {
    /// Layers `bundle` over `anchor`, the state of the block the bundle was executed on.
    pub(crate) const fn new(anchor: StateProviderBox, bundle: Arc<BundleState>) -> Self {
        Self { anchor, bundle }
    }

    /// The bundle in the trie's hashed form, built per call: only the root and proof methods
    /// need it.
    fn hashed_bundle(&self) -> ProviderResult<HashedPostState> {
        self.anchor.hashed_post_state(&self.bundle)
    }

    fn hashed_bundle_storage(&self, address: Address) -> ProviderResult<HashedStorage> {
        Ok(self.hashed_bundle()?.storages.remove(&keccak256(address)).unwrap_or_default())
    }
}

impl fmt::Debug for PendingStateProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("PendingStateProvider")
            .field("accounts", &self.bundle.state.len())
            .finish_non_exhaustive()
    }
}

impl AccountReader for PendingStateProvider {
    fn basic_account(&self, address: &Address) -> ProviderResult<Option<Account>> {
        match self.bundle.account(address) {
            Some(account) => Ok(account.info.as_ref().map(Account::from)),
            None => self.anchor.basic_account(address),
        }
    }
}

/// Pending blocks have no hash: their numbers resolve through the anchor's state, which holds
/// only canonical blocks.
impl BlockHashReader for PendingStateProvider {
    fn block_hash(&self, number: BlockNumber) -> ProviderResult<Option<B256>> {
        self.anchor.block_hash(number)
    }

    fn canonical_hashes_range(
        &self,
        start: BlockNumber,
        end: BlockNumber,
    ) -> ProviderResult<Vec<B256>> {
        self.anchor.canonical_hashes_range(start, end)
    }
}

impl BytecodeReader for PendingStateProvider {
    fn bytecode_by_hash(&self, code_hash: &B256) -> ProviderResult<Option<Bytecode>> {
        match self.bundle.bytecode(code_hash) {
            Some(bytecode) => Ok(Some(Bytecode(bytecode))),
            None => self.anchor.bytecode_by_hash(code_hash),
        }
    }
}

impl StateProvider for PendingStateProvider {
    fn storage(
        &self,
        address: Address,
        storage_key: StorageKey,
    ) -> ProviderResult<Option<StorageValue>> {
        if let Some(account) = self.bundle.account(&address) {
            let slot = U256::from_be_bytes(storage_key.0);
            if let Some(value) = account.storage.get(&slot) {
                return Ok(Some(value.present_value));
            }
            // Since Cancun (EIP-6780) only an account created in the same transaction is
            // destroyed, so the anchor holds no slots for it; reth's overlay answers the same.
            if account.was_destroyed() {
                return Ok(Some(U256::ZERO));
            }
        }
        self.anchor.storage(address, storage_key)
    }
}

/// The anchor's state is enough to hash a later bundle: since Cancun (EIP-6780) an account is
/// destroyed only in the transaction that created it, so the pending bundle holds no slot of it.
impl HashedPostStateProvider for PendingStateProvider {
    fn hashed_post_state(&self, bundle_state: &BundleState) -> ProviderResult<HashedPostState> {
        self.anchor.hashed_post_state(bundle_state)
    }
}

impl StateRootProvider for PendingStateProvider {
    fn state_root(&self, hashed_state: HashedPostState) -> ProviderResult<B256> {
        let mut state = self.hashed_bundle()?;
        state.extend(hashed_state);
        self.anchor.state_root(state)
    }

    fn state_root_from_nodes(&self, mut input: TrieInput) -> ProviderResult<B256> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.state_root_from_nodes(input)
    }

    fn state_root_with_updates(
        &self,
        hashed_state: HashedPostState,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        let mut state = self.hashed_bundle()?;
        state.extend(hashed_state);
        self.anchor.state_root_with_updates(state)
    }

    fn state_root_from_nodes_with_updates(
        &self,
        mut input: TrieInput,
    ) -> ProviderResult<(B256, TrieUpdates)> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.state_root_from_nodes_with_updates(input)
    }
}

impl StorageRootProvider for PendingStateProvider {
    fn storage_root(
        &self,
        address: Address,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<B256> {
        let mut storage = self.hashed_bundle_storage(address)?;
        storage.extend(&hashed_storage);
        self.anchor.storage_root(address, storage)
    }

    fn storage_proof(
        &self,
        address: Address,
        slot: B256,
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageProof> {
        let mut storage = self.hashed_bundle_storage(address)?;
        storage.extend(&hashed_storage);
        self.anchor.storage_proof(address, slot, storage)
    }

    fn storage_multiproof(
        &self,
        address: Address,
        slots: &[B256],
        hashed_storage: HashedStorage,
    ) -> ProviderResult<StorageMultiProof> {
        let mut storage = self.hashed_bundle_storage(address)?;
        storage.extend(&hashed_storage);
        self.anchor.storage_multiproof(address, slots, storage)
    }
}

impl StateProofProvider for PendingStateProvider {
    fn proof(
        &self,
        mut input: TrieInput,
        address: Address,
        slots: &[B256],
    ) -> ProviderResult<AccountProof> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.proof(input, address, slots)
    }

    fn multiproof(
        &self,
        mut input: TrieInput,
        targets: MultiProofTargets,
    ) -> ProviderResult<MultiProof> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.multiproof(input, targets)
    }

    fn multiproof_v2(
        &self,
        mut input: TrieInput,
        targets: MultiProofTargetsV2,
    ) -> ProviderResult<DecodedMultiProofV2> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.multiproof_v2(input, targets)
    }

    fn witness(
        &self,
        mut input: TrieInput,
        target: HashedPostState,
        mode: ExecutionWitnessMode,
    ) -> ProviderResult<Vec<Bytes>> {
        input.prepend(self.hashed_bundle()?);
        self.anchor.witness(input, target, mode)
    }
}

#[cfg(test)]
mod tests {
    use alloy_consensus::BlockHeader;
    use alloy_eips::{eip2935::HISTORY_STORAGE_ADDRESS, eip4788::BEACON_ROOTS_ADDRESS};
    use alloy_network::TransactionBuilder;
    use alloy_primitives::{address, map::B256Map};
    use alloy_rpc_types_eth::TransactionRequest;
    use alloy_sol_types::{SolCall, SolConstructor};
    use reth_provider::{
        HeaderProvider, StateProviderFactory,
        test_utils::{ExtendedAccount, MockEthProvider},
    };
    use reth_revm::{
        db::{AccountStatus, BundleAccount, states::StorageSlot},
        state::AccountInfo,
    };
    use reth_trie_common::ProofV2Target;

    use super::*;
    use crate::test_utils::{
        Account as TestAccount, DoubleCounter, PendingProbe, SelfDestructor, TestHarness,
    };

    const ALICE: Address = Address::with_last_byte(0xA1);
    const BOB: Address = Address::with_last_byte(0xB2);
    const COUNTER: Address = Address::with_last_byte(0xC3);
    const NEWCOMER: Address = Address::with_last_byte(0xD4);
    const COUNTER_CODE: [u8; 2] = [0x60, 0x00];

    fn slot(index: u64) -> StorageKey {
        B256::from(U256::from(index))
    }

    /// ALICE and BOB with balances, and COUNTER with code and slots 0 and 1.
    fn anchor() -> StateProviderBox {
        let provider: MockEthProvider = MockEthProvider::new();
        provider.extend_accounts([
            (ALICE, ExtendedAccount::new(1, U256::from(10))),
            (BOB, ExtendedAccount::new(0, U256::from(20))),
            (
                COUNTER,
                ExtendedAccount::new(1, U256::ZERO)
                    .with_bytecode(Bytes::from_static(&COUNTER_CODE))
                    .extend_storage([(slot(0), U256::from(11)), (slot(1), U256::from(22))]),
            ),
        ]);
        Box::new(provider)
    }

    fn account(
        info: Option<AccountInfo>,
        slots: impl IntoIterator<Item = (u64, u64)>,
        status: AccountStatus,
    ) -> BundleAccount {
        let storage = slots
            .into_iter()
            .map(|(slot, value)| {
                (U256::from(slot), StorageSlot::new_changed(U256::ZERO, U256::from(value)))
            })
            .collect();
        BundleAccount::new(None, info, storage, status)
    }

    #[test]
    fn the_bundle_answers_first_and_the_anchor_answers_the_rest() -> ProviderResult<()> {
        let code = Bytecode::new_raw(Bytes::from_static(&[0x60, 0x01]));
        let counter = AccountInfo::default().with_nonce(1).with_code_hash(keccak256(COUNTER_CODE));
        let newcomer = AccountInfo::default().with_code(code.0.clone());
        let mut bundle = BundleState::default();
        bundle.state.extend([
            (
                ALICE,
                account(
                    Some(AccountInfo::default().with_nonce(2).with_balance(U256::from(9))),
                    [],
                    AccountStatus::Changed,
                ),
            ),
            (COUNTER, account(Some(counter), [(0, 12)], AccountStatus::Changed)),
            (NEWCOMER, account(Some(newcomer), [(5, 55)], AccountStatus::InMemoryChange)),
        ]);
        bundle.contracts.insert(code.hash_slow(), code.0.clone());
        let pending = PendingStateProvider::new(anchor(), Arc::new(bundle));

        let alice = pending.basic_account(&ALICE)?.expect("alice exists");
        assert_eq!((alice.nonce, alice.balance), (2, U256::from(9)));
        assert_eq!(pending.basic_account(&BOB)?.map(|bob| bob.balance), Some(U256::from(20)));
        assert_eq!(pending.storage(COUNTER, slot(0))?, Some(U256::from(12)));
        assert_eq!(pending.storage(COUNTER, slot(1))?, Some(U256::from(22)), "not in the bundle");
        assert_eq!(pending.storage(NEWCOMER, slot(5))?, Some(U256::from(55)));
        assert_eq!(
            pending.bytecode_by_hash(&code.hash_slow())?.map(|code| code.original_bytes()),
            Some(code.original_bytes())
        );
        assert_eq!(
            pending.bytecode_by_hash(&keccak256(COUNTER_CODE))?.map(|code| code.original_bytes()),
            Some(Bytes::from_static(&COUNTER_CODE)),
            "code only the anchor holds"
        );

        Ok(())
    }

    #[test]
    fn a_destroyed_account_keeps_nothing_of_the_anchor() -> ProviderResult<()> {
        let mut bundle = BundleState::default();
        bundle.state.insert(COUNTER, account(None, [], AccountStatus::Destroyed));
        let pending = PendingStateProvider::new(anchor(), Arc::new(bundle));

        assert_eq!(pending.basic_account(&COUNTER)?, None);
        assert_eq!(pending.storage(COUNTER, slot(1))?, Some(U256::ZERO));

        Ok(())
    }

    /// A slot that the bundle set to zero reads zero, not the anchor's value.
    #[test]
    fn a_slot_the_bundle_set_to_zero_reads_zero() -> ProviderResult<()> {
        let counter = AccountInfo::default().with_nonce(1).with_code_hash(keccak256(COUNTER_CODE));
        let mut bundle = BundleState::default();
        bundle.state.insert(COUNTER, account(Some(counter), [(0, 0)], AccountStatus::Changed));
        let pending = PendingStateProvider::new(anchor(), Arc::new(bundle));

        assert_eq!(pending.storage(COUNTER, slot(0))?, Some(U256::ZERO));
        assert_eq!(pending.storage(COUNTER, slot(1))?, Some(U256::from(22)), "not in the bundle");

        Ok(())
    }

    const BENEFICIARY: Address = address!("0x00000000000000000000000000000000000000b1");

    /// Block 2's bundle over block 1's state reads as block 2's own state: every account, slot
    /// and code, and every root and proof, with block 2's state root as the reference.
    #[tokio::test]
    async fn the_bundle_over_its_parent_reads_as_the_block_that_wrote_it() -> eyre::Result<()> {
        // reth's default persistence threshold keeps both blocks in memory, where a live
        // snapshot's anchor is.
        let harness = TestHarness::new().await?;
        let deployer = TestAccount::Deployer;

        // Block 1: a counter with `count1` and `count2` at 1, and a balance at the address that
        // block 2 creates and destroys a contract at.
        let (deploy_counter, counter, _) =
            deployer.create_deployment_tx(DoubleCounter::BYTECODE.clone(), 0)?;
        let doomed = deployer.address().create(2);
        let (prefund, _) = TestAccount::Alice.sign_txn_request(
            TransactionRequest::default().to(doomed).value(U256::from(5)).nonce(0),
        )?;
        harness.build_block_from_transactions(vec![deploy_counter, prefund]).await?;

        // Block 2: `count1` changes and `count2` does not, new code, and a destroyed account.
        let (increment, _) = TestAccount::Bob.sign_txn_request(
            TransactionRequest::default()
                .to(counter)
                .with_input(DoubleCounter::incrementCall {}.abi_encode())
                .nonce(0),
        )?;
        let (deploy_probe, probe, _) =
            deployer.create_deployment_tx(PendingProbe::BYTECODE.clone(), 1)?;
        let destroy_on_create = [
            SelfDestructor::BYTECODE.to_vec(),
            SelfDestructor::constructorCall { beneficiary: BENEFICIARY, onCreate: true }
                .abi_encode(),
        ]
        .concat();
        let (deploy_doomed, _, _) = deployer.create_deployment_tx(destroy_on_create.into(), 2)?;
        harness.build_block_from_transactions(vec![increment, deploy_probe, deploy_doomed]).await?;

        let block = harness.latest_block();
        assert_eq!((block.number(), block.body().transactions.len()), (2, 3));
        let provider = harness.blockchain_provider();
        let output = provider
            .canonical_in_memory_state()
            .state_by_hash(block.hash())
            .expect("block 2 is in memory")
            .block_ref()
            .execution_output
            .clone();
        let pending = PendingStateProvider::new(
            provider.state_by_block_hash(block.parent_hash())?,
            Arc::new(output.state.clone()),
        );
        let sealed = provider.state_by_block_hash(block.hash())?;

        let mut addresses = TestAccount::all().map(|account| account.address()).to_vec();
        addresses.extend([
            counter,
            probe,
            doomed,
            BENEFICIARY,
            BEACON_ROOTS_ADDRESS,
            HISTORY_STORAGE_ADDRESS,
            block.beneficiary(),
            Address::repeat_byte(0x42),
        ]);
        for address in addresses {
            let account = sealed.basic_account(&address)?;
            assert_eq!(pending.basic_account(&address)?, account, "account {address}");
            if let Some(code_hash) = account.and_then(|account| account.bytecode_hash) {
                assert_eq!(
                    pending.bytecode_by_hash(&code_hash)?.map(|code| code.original_bytes()),
                    sealed.bytecode_by_hash(&code_hash)?.map(|code| code.original_bytes()),
                    "code of {address}"
                );
            }
        }

        // The beacon-root ring buffer (EIP-4788) writes two slots per block, the block-hash ring
        // buffer (EIP-2935) one.
        let ring = |timestamp: u64| [timestamp % 8191, timestamp % 8191 + 8191];
        let parent = provider.sealed_header(1)?.expect("block 1 exists");
        let mut slots = vec![(counter, 0), (counter, 1), (counter, 2), (probe, 0), (doomed, 0)];
        slots.extend([0, 1].map(|index| (HISTORY_STORAGE_ADDRESS, index)));
        slots.extend(ring(parent.timestamp()).map(|index| (BEACON_ROOTS_ADDRESS, index)));
        slots.extend(ring(block.timestamp()).map(|index| (BEACON_ROOTS_ADDRESS, index)));
        for (address, index) in slots {
            assert_eq!(
                pending.storage(address, slot(index))?,
                sealed.storage(address, slot(index))?,
                "slot {index} of {address}"
            );
        }

        for number in 0..block.number() {
            assert_eq!(pending.block_hash(number)?, sealed.block_hash(number)?);
        }
        assert_eq!(pending.block_hash(block.number())?, None, "a pending block has no hash");
        assert_eq!(
            pending.canonical_hashes_range(0, block.number())?,
            sealed.canonical_hashes_range(0, block.number())?
        );
        assert_eq!(
            pending.hashed_post_state(&output.state)?,
            sealed.hashed_post_state(&output.state)?
        );

        let root = block.state_root();
        assert_eq!(pending.state_root(HashedPostState::default())?, root);
        assert_eq!(pending.state_root_with_updates(HashedPostState::default())?.0, root);
        assert_eq!(pending.state_root_from_nodes(TrieInput::default())?, root);
        assert_eq!(pending.state_root_from_nodes_with_updates(TrieInput::default())?.0, root);

        // A later change lands on top of the bundle, as a simulated block's does.
        let later = HashedPostState::default()
            .with_accounts([(
                keccak256(TestAccount::Charlie.address()),
                Some(Account { nonce: 9, ..Default::default() }),
            )])
            .with_storages([(
                keccak256(counter),
                HashedStorage::from_iter([(keccak256(slot(0)), U256::from(7))]),
            )]);
        assert_eq!(pending.state_root(later.clone())?, sealed.state_root(later.clone())?);
        assert_eq!(
            pending.state_root_with_updates(later.clone())?.0,
            sealed.state_root_with_updates(later)?.0
        );

        let keys = [slot(0), slot(1)];
        for address in [counter, probe, doomed, BEACON_ROOTS_ADDRESS, HISTORY_STORAGE_ADDRESS] {
            assert_eq!(
                pending.storage_root(address, HashedStorage::default())?,
                sealed.storage_root(address, HashedStorage::default())?,
                "storage root of {address}"
            );
        }
        assert_eq!(
            pending.storage_proof(counter, keys[0], HashedStorage::default())?,
            sealed.storage_proof(counter, keys[0], HashedStorage::default())?
        );
        assert_eq!(
            pending.storage_multiproof(counter, &keys, HashedStorage::default())?,
            sealed.storage_multiproof(counter, &keys, HashedStorage::default())?
        );
        assert_eq!(
            pending.proof(TrieInput::default(), counter, &keys)?,
            sealed.proof(TrieInput::default(), counter, &keys)?
        );
        let targets =
            || MultiProofTargets::account_with_slots(keccak256(counter), keys.map(keccak256));
        assert_eq!(
            pending.multiproof(TrieInput::default(), targets())?,
            sealed.multiproof(TrieInput::default(), targets())?
        );
        let targets_v2 = || MultiProofTargetsV2 {
            account_targets: vec![ProofV2Target::new(keccak256(counter))],
            storage_targets: B256Map::from_iter([(
                keccak256(counter),
                keys.map(|key| ProofV2Target::new(keccak256(key))).to_vec(),
            )]),
        };
        assert_eq!(
            pending.multiproof_v2(TrieInput::default(), targets_v2())?,
            sealed.multiproof_v2(TrieInput::default(), targets_v2())?
        );
        let counter_account = sealed.basic_account(&counter)?;
        let witness = |provider: &dyn StateProvider| -> ProviderResult<Vec<Bytes>> {
            let target = HashedPostState::default()
                .with_accounts([(keccak256(counter), counter_account)])
                .with_storages([(
                    keccak256(counter),
                    HashedStorage::from_iter([(keccak256(slot(1)), U256::from(8))]),
                )]);
            let mut nodes =
                provider.witness(TrieInput::default(), target, ExecutionWitnessMode::Legacy)?;
            nodes.sort();
            Ok(nodes)
        };
        assert_eq!(witness(&pending)?, witness(&*sealed)?);

        Ok(())
    }
}
