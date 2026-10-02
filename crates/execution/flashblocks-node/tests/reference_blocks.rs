//! reth's own payload builder is the reference for the block assembler. Each test builds
//! canonical blocks with the engine, replays each block as the producer streams one, and requires
//! the header the node assembles to equal the canonical header. The reference block carries no
//! payout transaction, so every field can match. Pending carries no block access list hash, so
//! the test copies the canonical one in before it compares the hashes.

use std::sync::Arc;

use alloy_consensus::Header;
use alloy_eips::{Encodable2718, eip4895::Withdrawal};
use alloy_genesis::Genesis;
use alloy_primitives::{Address, B256, Bytes, hex::FromHex, keccak256, map::foldhash::HashMap};
use alloy_rpc_types_engine::PayloadId;
use ethgas_node_runner::test_utils::{
    Account, EnginePayload, TestHarness, build_prague_test_genesis,
    build_test_genesis_with_amsterdam_at, slot_number_at,
};
use ethgas_reth_flashblocks::{
    BlockAssembler, CanonicalBlockOracle, ReorgDetector,
    payload::{ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata},
};
use eyre::{Result, eyre};
use reth_chainspec::ChainSpec;
use reth_ethereum_primitives::{Receipt, TransactionSigned};
use reth_provider::{BlockReader, HeaderProvider, ReceiptProvider};
use reth_transaction_pool::test_utils::TransactionBuilder;

/// A transfer from `from` at `nonce`, signed with its test key.
fn transfer(from: Account, nonce: u64) -> TransactionSigned {
    TransactionBuilder::default()
        .signer(B256::from_hex(from.private_key()).expect("a hex key"))
        .chain_id(1)
        .to(Account::Charlie.address())
        .nonce(nonce)
        .value(1)
        .gas_limit(21_000)
        .max_fee_per_gas(1_000_000_000)
        .max_priority_fee_per_gas(1_000_000_000)
        .into_eip1559()
}

/// The block's flashblocks as the producer streams them: the base payload with the first
/// transaction, then the rest. Each diff carries the cumulative fields of the block so far, which
/// for the last one are the block's own. The producer sends a slot on every chain.
fn flashblocks_of(
    payload: &EnginePayload,
    parent_beacon_block_root: B256,
    receipts: &HashMap<B256, Receipt>,
) -> Vec<FlashBlock> {
    let v3 = payload.payload_v3();
    let v1 = &v3.payload_inner.payload_inner;
    let slot_number = match payload {
        EnginePayload::V6(envelope) => envelope.execution_payload.slot_number,
        _ => slot_number_at(v1.timestamp),
    };
    let base = ExecutionPayloadBaseV1 {
        parent_beacon_block_root,
        parent_hash: v1.parent_hash,
        fee_recipient: v1.fee_recipient,
        prev_randao: v1.prev_randao,
        block_number: v1.block_number,
        gas_limit: v1.gas_limit,
        timestamp: v1.timestamp,
        extra_data: v1.extra_data.clone(),
        base_fee_per_gas: v1.base_fee_per_gas,
        slot_number: Some(slot_number),
    };

    let split = v1.transactions.len().min(1);
    [&v1.transactions[..split], &v1.transactions[split..]]
        .into_iter()
        .enumerate()
        .map(|(index, transactions)| {
            let own_receipts = transactions
                .iter()
                .map(|transaction| {
                    let hash = keccak256(transaction);
                    (hash, receipts[&hash].clone())
                })
                .collect();
            FlashBlock {
                payload_id: PayloadId::default(),
                index: index as u64,
                base: (index == 0).then(|| base.clone()),
                diff: ExecutionPayloadFlashblockDeltaV1 {
                    state_root: v1.state_root,
                    receipts_root: v1.receipts_root,
                    logs_bloom: v1.logs_bloom,
                    gas_used: v1.gas_used,
                    block_hash: v1.block_hash,
                    transactions: transactions.to_vec(),
                    withdrawals: v3.payload_inner.withdrawals.clone(),
                    blob_gas_used: v3.blob_gas_used,
                    excess_blob_gas: v3.excess_blob_gas,
                    requests: Some(payload.execution_requests().clone()),
                },
                metadata: Metadata {
                    block_number: v1.block_number,
                    receipts: own_receipts,
                    ..Default::default()
                },
            }
        })
        .collect()
}

/// Builds three blocks of two transfers and one withdrawal each on `genesis`, and requires the
/// node to assemble each one as reth built it.
async fn assert_assembles_like_reth(genesis: Genesis) -> Result<()> {
    let chain_spec = Arc::new(ChainSpec::from(genesis));
    let harness = TestHarness::builder().with_chain_spec(Arc::clone(&chain_spec)).build().await?;
    let provider = harness.blockchain_provider();

    for number in 1..=3u64 {
        let transactions: Vec<Bytes> = [Account::Alice, Account::Bob]
            .map(|from| transfer(from, number - 1).encoded_2718().into())
            .to_vec();
        let withdrawal = Withdrawal {
            index: number,
            validator_index: number,
            address: Address::with_last_byte(number as u8),
            amount: 1_000,
        };
        let payload = harness.build_block_with_withdrawals(transactions, vec![withdrawal]).await?;

        let canonical: Header =
            provider.header_by_number(number)?.ok_or_else(|| eyre!("no block {number}"))?;
        let block = provider.block(number.into())?.ok_or_else(|| eyre!("no block {number}"))?;
        let receipts = provider.receipts_by_block(number.into())?.unwrap_or_default();
        assert_eq!(block.body.transactions.len(), 2, "both transfers are in block {number}");
        let receipts_by_hash: HashMap<B256, Receipt> = block
            .body
            .transactions
            .iter()
            .map(|transaction| *transaction.tx_hash())
            .zip(receipts)
            .collect();
        let parent_beacon_block_root =
            canonical.parent_beacon_block_root.ok_or_else(|| eyre!("no beacon root"))?;

        let flashblocks = flashblocks_of(&payload, parent_beacon_block_root, &receipts_by_hash);
        let assembled = BlockAssembler::assemble(chain_spec.as_ref(), &flashblocks)?;

        let pending_hashes: Vec<B256> =
            assembled.block.body.transactions.iter().map(|tx| *tx.tx_hash()).collect();
        let canonical_hashes: Vec<B256> =
            block.body.transactions.iter().map(|tx| *tx.tx_hash()).collect();
        let reorg = ReorgDetector::detect(&pending_hashes, &canonical_hashes);
        assert_eq!(
            CanonicalBlockOracle::compare(&assembled.header, &canonical, true, &reorg),
            vec![],
            "block {number}"
        );

        let mut header = assembled.header.into_inner();
        if let EnginePayload::V6(envelope) = &payload {
            header.block_access_list_hash =
                Some(keccak256(&envelope.execution_payload.block_access_list));
        }
        assert_eq!(header, canonical, "block {number}");
        assert_eq!(header.hash_slow(), payload.payload_v3().payload_inner.payload_inner.block_hash);
    }
    Ok(())
}

#[tokio::test]
async fn the_node_assembles_prague_blocks_as_reth_builds_them() -> Result<()> {
    assert_assembles_like_reth(build_prague_test_genesis()).await
}

/// Block 1 is an Osaka block, blocks 2 and 3 are Amsterdam blocks.
#[tokio::test]
async fn the_node_assembles_blocks_across_the_amsterdam_fork_as_reth_builds_them() -> Result<()> {
    assert_assembles_like_reth(build_test_genesis_with_amsterdam_at(Some(5))).await
}
