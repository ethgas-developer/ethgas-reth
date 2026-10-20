//! Concrete reth types the simulator runs against.

use reth_chainspec::ChainSpec;
use reth_consensus::FullConsensus;
use reth_ethereum_primitives::{Block, EthPrimitives};
use reth_evm_ethereum::{EthEvmConfig, factory::RethEvmFactory};
use reth_node_api::PayloadValidator;
use reth_node_ethereum::EthPayloadTypes;

/// Consensus rules the simulator checks a submitted block against.
pub type RethConsensus = dyn FullConsensus<EthPrimitives>;

/// Converts a submitted execution payload into a block.
pub type RethPayloadValidator = dyn PayloadValidator<EthPayloadTypes, Block = Block>;

/// The node's EVM configuration.
pub type RethEvmConfig = EthEvmConfig<ChainSpec, RethEvmFactory>;
