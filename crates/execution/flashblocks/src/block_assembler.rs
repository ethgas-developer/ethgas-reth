//! Block assembly from flashblocks.
//!
//! This module provides the [`BlockAssembler`] which reconstructs Ethereum blocks
//! from flashblocks.

use std::collections::HashMap;

use alloy_consensus::{Header, Sealable};
use alloy_eips::{eip6110::DEPOSIT_REQUEST_TYPE, eip7685::Requests};
use alloy_primitives::{B256, Bytes, Sealed};
use alloy_rpc_types::Withdrawal;
use alloy_rpc_types_engine::{ExecutionPayloadV1, ExecutionPayloadV2, ExecutionPayloadV3};
use reth_ethereum_primitives::{Block, Receipt};
use reth_evm::eth::{eip6110::parse_deposits_from_receipts, spec::EthExecutorSpec};
use tracing::warn;

use crate::{
    error::{ExecutionError, ProtocolError, Result},
    payload::FlashBlock,
};

/// Result of assembling a block from flashblocks.
#[derive(Debug, Clone)]
pub struct AssembledBlock {
    /// The reconstructed Ethereum block.
    pub block: Block,
    /// The sealed header for this block.
    pub header: Sealed<Header>,
}

/// Assembles Ethereum blocks from flashblocks.
///
/// This component handles the reconstruction of complete blocks from
/// a sequence of flashblocks, extracting transactions, withdrawals,
/// and building the execution payload.
#[derive(Debug, Default)]
pub struct BlockAssembler;

impl BlockAssembler {
    /// Creates a new block assembler.
    pub const fn new() -> Self {
        Self
    }

    /// Assembles a complete block from a slice of flashblocks.
    ///
    /// # Arguments
    /// * `flashblocks` - A slice of flashblocks for a single block number.
    ///
    /// # Returns
    /// An [`AssembledBlock`] containing the reconstructed block and sealed header.
    ///
    /// # Errors
    /// Returns an error if:
    /// - The flashblocks slice is empty
    /// - The first flashblock is missing its base payload
    /// - Block conversion fails
    pub fn assemble(
        spec: &impl EthExecutorSpec,
        flashblocks: &[FlashBlock],
    ) -> Result<AssembledBlock> {
        let first = flashblocks.first().ok_or(ProtocolError::EmptyFlashblocks)?;
        let base = first.base.clone().ok_or(ProtocolError::MissingBase)?;
        let latest_flashblock = flashblocks.last().ok_or(ProtocolError::EmptyFlashblocks)?;

        let transactions: Vec<Bytes> = flashblocks
            .iter()
            .flat_map(|flashblock| flashblock.diff.transactions.clone())
            .collect();

        // Cumulative, not an increment: the producer resends the whole list on every flashblock.
        let withdrawals: Vec<Withdrawal> = latest_flashblock.diff.withdrawals.clone();
        if flashblocks.iter().any(|flashblock| {
            !flashblock.diff.withdrawals.is_empty() && flashblock.diff.withdrawals != withdrawals
        }) {
            warn!(
                message =
                    "flashblock withdrawals are not cumulative; assembler assumption violated",
                block_number = base.block_number,
            );
        }

        let execution_payload = ExecutionPayloadV3 {
            blob_gas_used: latest_flashblock.diff.blob_gas_used,
            excess_blob_gas: latest_flashblock.diff.excess_blob_gas,
            payload_inner: ExecutionPayloadV2 {
                withdrawals,
                payload_inner: ExecutionPayloadV1 {
                    parent_hash: base.parent_hash,
                    fee_recipient: base.fee_recipient,
                    state_root: latest_flashblock.diff.state_root,
                    receipts_root: latest_flashblock.diff.receipts_root,
                    logs_bloom: latest_flashblock.diff.logs_bloom,
                    prev_randao: base.prev_randao,
                    block_number: base.block_number,
                    gas_limit: base.gas_limit,
                    gas_used: latest_flashblock.diff.gas_used,
                    timestamp: base.timestamp,
                    extra_data: base.extra_data.clone(),
                    base_fee_per_gas: base.base_fee_per_gas,
                    block_hash: latest_flashblock.diff.block_hash,
                    transactions,
                },
            },
        };

        let mut block: Block = execution_payload
            .try_into_block()
            .map_err(|e| ExecutionError::BlockConversion(e.to_string()))?;

        // ExecutionPayloadV3 predates both fields, so neither arrives on the wire.
        block.header.parent_beacon_block_root = Some(base.parent_beacon_block_root);
        block.header.requests_hash = Self::requests_hash(spec, &block, flashblocks)?;

        let sealed_header = block.header.clone().seal_slow();

        Ok(AssembledBlock { block, header: sealed_header })
    }

    /// Derives the requests hash for an assembled block.
    ///
    /// Only EIP-6110 deposits are recoverable: they are logs, so the wire receipts carry them. The
    /// rest — EIP-7002, EIP-7251, and Amsterdam's EIP-8282 pair — come from system calls made after
    /// the last transaction, so a block carrying one gets a confidently wrong hash.
    fn requests_hash(
        spec: &impl EthExecutorSpec,
        block: &Block,
        flashblocks: &[FlashBlock],
    ) -> Result<Option<B256>> {
        // The same timestamp reth gates on when it builds a canonical header.
        if !spec.is_prague_active_at_timestamp(block.header.timestamp) {
            return Ok(None);
        }

        let receipts: HashMap<&B256, &Receipt> =
            flashblocks.iter().flat_map(|flashblock| flashblock.metadata.receipts.iter()).collect();

        // Block order, not map order: the scan concatenates log data, and `Metadata.receipts` is
        // a `HashMap`, so iterating it directly would hash differently each run.
        let ordered = block
            .body
            .transactions
            .iter()
            .map(|transaction| {
                receipts.get(transaction.tx_hash()).copied().ok_or_else(|| {
                    ExecutionError::MissingReceipt { tx_hash: *transaction.tx_hash() }
                })
            })
            .collect::<std::result::Result<Vec<&Receipt>, _>>()?;

        let deposits = parse_deposits_from_receipts(spec, ordered)
            .map_err(|e| ExecutionError::BlockConversion(e.to_string()))?;

        let mut requests = Requests::default();
        // Skips the type entirely when there are no deposits, leaving the empty-requests hash.
        requests.push_request_with_type(DEPOSIT_REQUEST_TYPE, deposits.iter().copied());
        Ok(Some(requests.requests_hash()))
    }
}

#[cfg(test)]
mod tests {
    use alloy_primitives::{Address, B256, Bloom, U256};
    use alloy_rpc_types_engine::PayloadId;

    use alloy_eips::eip7685::EMPTY_REQUESTS_HASH;
    use alloy_genesis::{ChainConfig, Genesis};
    use alloy_primitives::b256;
    use reth_chainspec::ChainSpec;

    use super::*;
    use crate::{
        ProtocolError,
        payload::{
            ExecutionPayloadBaseV1, ExecutionPayloadFlashblockDeltaV1, FlashBlock, Metadata,
        },
    };

    /// Prague-active, matching the integration harness genesis (`prague_time: Some(0)`).
    fn test_spec() -> ChainSpec {
        ChainSpec::from(Genesis {
            config: ChainConfig { prague_time: Some(0), ..Default::default() },
            ..Default::default()
        })
    }

    fn create_test_flashblock(index: u64, with_base: bool) -> FlashBlock {
        FlashBlock {
            payload_id: PayloadId::default(),
            index,
            base: with_base.then(|| ExecutionPayloadBaseV1 {
                parent_beacon_block_root: B256::ZERO,
                parent_hash: B256::ZERO,
                fee_recipient: Address::ZERO,
                prev_randao: B256::ZERO,
                block_number: 100,
                gas_limit: 30_000_000,
                timestamp: 1700000000,
                extra_data: Bytes::default(),
                base_fee_per_gas: U256::from(1000000000u64),
            }),
            diff: ExecutionPayloadFlashblockDeltaV1 {
                state_root: B256::ZERO,
                receipts_root: B256::ZERO,
                logs_bloom: Bloom::default(),
                gas_used: 21000,
                block_hash: B256::ZERO,
                transactions: vec![],
                withdrawals: vec![],
                blob_gas_used: 0,
                excess_blob_gas: 0,
            },
            metadata: Metadata::default(),
        }
    }

    #[test]
    fn test_assemble_single_flashblock() {
        let flashblocks = vec![create_test_flashblock(0, true)];

        let result = BlockAssembler::assemble(&test_spec(), &flashblocks);
        assert!(result.is_ok());

        let assembled = result.unwrap();
        assert_eq!(assembled.block.header.number, 100);
    }

    #[test]
    fn test_assemble_multiple_flashblocks() {
        let flashblocks = vec![
            create_test_flashblock(0, true),
            create_test_flashblock(1, false),
            create_test_flashblock(2, false),
        ];

        let result = BlockAssembler::assemble(&test_spec(), &flashblocks);
        assert!(result.is_ok());
    }

    /// Two real mainnet deposit logs, from the fixtures in `alloy-evm`'s own eip6110 tests.
    const DEPOSIT_LOG_A: &str = r#"[{"address":"0x00000000219ab540356cbb839cbe05303d7705fa","topics":["0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5"],"data":"0x00000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000140000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000030998c8086669bf65e24581cda47d8537966e9f5066fc6ffdcba910a1bfb91eae7a4873fcce166a1c4ea217e6b1afd396200000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000002001000000000000000000000001c340fb72ed14d4eaa71f7633ee9e33b88d4f3900000000000000000000000000000000000000000000000000000000000000080040597307000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000006098ddbffd700c1aac324cfdf0492ff289223661eb26718ce3651ba2469b22f480d56efab432ed91af05a006bde0c1ea68134e0acd8cacca0c13ad1f716db874b44abfcc966368019753174753bca3af2ea84bc569c46f76592a91e97f311eddec0000000000000000000000000000000000000000000000000000000000000008e474160000000000000000000000000000000000000000000000000000000000","blockHash":"0x8d1289c5a7e0965b1d1bb75cdc4c3f73dda82d4ebb94ff5b98d1389cebd53b56","blockNumber":"0x12f0d8d","transactionHash":"0xa5239d4c542063d29022545835815b78b09f571f2bf1c8427f4765d6f5abbce9","transactionIndex":"0xc4","logIndex":"0x18f","removed":false}]"#;
    const DEPOSIT_LOG_B: &str = r#"[{"address":"0x00000000219ab540356cbb839cbe05303d7705fa","topics":["0x649bbc62d0e31342afea4e5cd82d4049e7e1ee912fc0889aa790803be39038c5"],"data":"0x00000000000000000000000000000000000000000000000000000000000000a000000000000000000000000000000000000000000000000000000000000001000000000000000000000000000000000000000000000000000000000000000140000000000000000000000000000000000000000000000000000000000000018000000000000000000000000000000000000000000000000000000000000002000000000000000000000000000000000000000000000000000000000000000030a1a2ba870a90e889aa594a0cc1c6feffb94c2d8f65646c937f1f456a315ef649533e25a4614d8f4f66ebdb06481b90af0000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000200100000000000000000000000a0f04a231efbc29e1db7d086300ff550211c2f6000000000000000000000000000000000000000000000000000000000000000800405973070000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000060ad416d590e1a7f52baff770a12835b68904efad22cc9f8ba531e50cbbd26f32b9c7373cf6538a0577f501e4d3e3e63e208767bcccaae94e1e3720bfb734a286f9c017d17af46536545ccb7ca94d71f295e71f6d25bf978c09ada6f8d3f7ba0390000000000000000000000000000000000000000000000000000000000000008e374160000000000000000000000000000000000000000000000000000000000","blockHash":"0x8d1289c5a7e0965b1d1bb75cdc4c3f73dda82d4ebb94ff5b98d1389cebd53b56","blockNumber":"0x12f0d8d","transactionHash":"0xd9734d4e3953bcaa939fd1c1d80950ee54aeecc02eef6ae8179f47f5b7103338","transactionIndex":"0x7c","logIndex":"0xe2","removed":false}]"#;

    /// EIP-7685 commitment over a block whose only request is the `DEPOSIT_LOG_A` deposit.
    const ONE_DEPOSIT_REQUESTS_HASH: B256 =
        b256!("0xfade8d87e2e9826e6a4d1c988bf4ba356f8bd4cf2cdf4b5eec9376686ec28acd");

    /// A transaction and its wire receipt, distinguished by `nonce` so each gets its own hash.
    fn transaction_with_receipt(
        nonce: u64,
        logs: Vec<alloy_primitives::Log>,
    ) -> (Bytes, B256, reth_ethereum_primitives::Receipt) {
        use alloy_consensus::TxEip1559;
        use alloy_eips::Encodable2718;
        use alloy_primitives::{Signature, TxKind};
        use reth_ethereum_primitives::{Receipt, TransactionSigned, TxType};

        let tx = TxEip1559 {
            chain_id: 1,
            nonce,
            gas_limit: 21_000,
            max_fee_per_gas: 1,
            max_priority_fee_per_gas: 1,
            to: TxKind::Call(Address::ZERO),
            value: U256::ZERO,
            ..Default::default()
        };
        let signed = TransactionSigned::new_unhashed(
            tx.into(),
            Signature::new(U256::from(1), U256::from(1), false),
        );
        let receipt =
            Receipt { tx_type: TxType::Eip1559, success: true, cumulative_gas_used: 21_000, logs };
        (signed.encoded_2718().into(), *signed.tx_hash(), receipt)
    }

    /// One flashblock carrying a transaction per entry in `logs_per_transaction`.
    fn flashblock_with_receipts(
        logs_per_transaction: Vec<Vec<alloy_primitives::Log>>,
    ) -> FlashBlock {
        let mut flashblock = create_test_flashblock(0, true);
        for (nonce, logs) in logs_per_transaction.into_iter().enumerate() {
            let (encoded, hash, receipt) = transaction_with_receipt(nonce as u64, logs);
            flashblock.diff.transactions.push(encoded);
            flashblock.metadata.receipts.insert(hash, receipt);
        }
        flashblock
    }

    fn deposit_logs(fixture: &str) -> Vec<alloy_primitives::Log> {
        serde_json::from_str(fixture).expect("deposit fixture parses")
    }

    #[test]
    fn requests_hash_is_empty_without_deposits() {
        let assembled =
            BlockAssembler::assemble(&test_spec(), &[flashblock_with_receipts(vec![vec![]])])
                .unwrap();

        assert_eq!(assembled.block.header.requests_hash, Some(EMPTY_REQUESTS_HASH));
    }

    /// Pins the exact commitment: a wrong type byte or a missing sha256 is still non-empty.
    #[test]
    fn requests_hash_commits_to_deposits_in_wire_receipts() {
        let assembled = BlockAssembler::assemble(
            &test_spec(),
            &[flashblock_with_receipts(vec![deposit_logs(DEPOSIT_LOG_A)])],
        )
        .unwrap();

        assert_eq!(assembled.block.header.requests_hash, Some(ONE_DEPOSIT_REQUESTS_HASH));
    }

    /// The deposit bytestring is a concatenation, so transaction order changes the hash.
    #[test]
    fn requests_hash_follows_block_order() {
        let forward = BlockAssembler::assemble(
            &test_spec(),
            &[flashblock_with_receipts(vec![
                deposit_logs(DEPOSIT_LOG_A),
                deposit_logs(DEPOSIT_LOG_B),
            ])],
        )
        .unwrap();
        let reversed = BlockAssembler::assemble(
            &test_spec(),
            &[flashblock_with_receipts(vec![
                deposit_logs(DEPOSIT_LOG_B),
                deposit_logs(DEPOSIT_LOG_A),
            ])],
        )
        .unwrap();

        assert_ne!(forward.block.header.requests_hash, reversed.block.header.requests_hash);
        assert_ne!(forward.block.header.requests_hash, Some(ONE_DEPOSIT_REQUESTS_HASH));
    }

    #[test]
    fn requests_hash_spans_every_flashblock_of_the_block() {
        let mut first = flashblock_with_receipts(vec![deposit_logs(DEPOSIT_LOG_A)]);
        let (encoded, hash, receipt) = transaction_with_receipt(1, deposit_logs(DEPOSIT_LOG_B));

        // The second flashblock carries only its own transaction.
        let mut second = create_test_flashblock(1, false);
        second.diff.transactions = vec![encoded.clone()];
        second.metadata.receipts.insert(hash, receipt.clone());

        let split = BlockAssembler::assemble(&test_spec(), &[first.clone(), second]).unwrap();

        // Identical to both transactions arriving in a single flashblock.
        first.diff.transactions.push(encoded);
        first.metadata.receipts.insert(hash, receipt);
        let together = BlockAssembler::assemble(&test_spec(), &[first]).unwrap();

        assert_eq!(split.block.header.requests_hash, together.block.header.requests_hash);
        assert_ne!(split.block.header.requests_hash, Some(ONE_DEPOSIT_REQUESTS_HASH));
    }

    /// `Metadata.receipts` is a map, so scanning it directly would pick up strays like this one.
    #[test]
    fn requests_hash_ignores_receipts_for_transactions_not_in_the_block() {
        let mut flashblock = flashblock_with_receipts(vec![deposit_logs(DEPOSIT_LOG_A)]);
        let (_, hash, receipt) = transaction_with_receipt(9, deposit_logs(DEPOSIT_LOG_B));
        flashblock.metadata.receipts.insert(hash, receipt);

        let assembled = BlockAssembler::assemble(&test_spec(), &[flashblock]).unwrap();

        assert_eq!(assembled.block.header.requests_hash, Some(ONE_DEPOSIT_REQUESTS_HASH));
    }

    #[test]
    fn assemble_fails_when_a_block_transaction_has_no_receipt() {
        let mut flashblock = flashblock_with_receipts(vec![deposit_logs(DEPOSIT_LOG_A)]);
        flashblock.metadata.receipts.clear();

        assert!(matches!(
            BlockAssembler::assemble(&test_spec(), &[flashblock]),
            Err(crate::error::StateProcessorError::Execution(
                ExecutionError::MissingReceipt { .. }
            ))
        ));
    }

    #[test]
    fn requests_hash_is_absent_before_prague() {
        let spec = ChainSpec::from(Genesis {
            config: ChainConfig { prague_time: None, ..Default::default() },
            ..Default::default()
        });
        let assembled =
            BlockAssembler::assemble(&spec, &[create_test_flashblock(0, true)]).unwrap();

        assert_eq!(assembled.block.header.requests_hash, None);
    }

    #[test]
    fn test_assemble_empty_flashblocks_fails() {
        let flashblocks: Vec<FlashBlock> = vec![];
        let result = BlockAssembler::assemble(&test_spec(), &flashblocks);
        assert!(matches!(
            result,
            Err(crate::error::StateProcessorError::Protocol(ProtocolError::EmptyFlashblocks))
        ));
    }

    #[test]
    fn test_withdrawals_are_cumulative_not_concatenated() {
        let withdrawals = vec![
            Withdrawal { index: 0, validator_index: 1, address: Address::ZERO, amount: 100 },
            Withdrawal { index: 1, validator_index: 2, address: Address::ZERO, amount: 200 },
        ];

        let mut flashblocks = vec![
            create_test_flashblock(0, true),
            create_test_flashblock(1, false),
            create_test_flashblock(2, false),
        ];
        for flashblock in &mut flashblocks {
            flashblock.diff.withdrawals = withdrawals.clone();
        }

        let assembled = BlockAssembler::assemble(&test_spec(), &flashblocks).unwrap();
        let assembled_withdrawals =
            assembled.block.body.withdrawals.as_ref().expect("withdrawals present");

        assert_eq!(assembled_withdrawals.len(), withdrawals.len());
        assert_eq!(assembled_withdrawals.as_ref(), withdrawals.as_slice());
    }

    #[test]
    fn test_assemble_missing_base_fails() {
        let flashblocks = vec![create_test_flashblock(0, false)];

        let result = BlockAssembler::assemble(&test_spec(), &flashblocks);
        assert!(matches!(
            result,
            Err(crate::error::StateProcessorError::Protocol(ProtocolError::MissingBase))
        ));
    }
}
