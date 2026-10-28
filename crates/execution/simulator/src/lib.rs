//! Block simulation extension for the Ethgas node.
//!
//! Serves a relay's block simulation requests from this node's state. The
//! `relay_validateBuilderSubmissionV*` methods execute a submitted block on top of its parent and
//! check it against its bid. The `relay_inclusionList` methods publish an inclusion list, built
//! from this node's transaction pool, after every canonical block.
//!
//! The crate is a port of the reth simulator in helix (`crates/simulator`, at `af5ccfed`). Its SSZ
//! validation server, `ClickHouse` transaction sink and block state recorder are not part of it.

pub mod blacklist;

pub mod common;

mod extension;
pub use extension::{SimulatorConfig, SimulatorExtension};

pub mod inclusion;
pub use inclusion::{
    api::{InclusionExt, InclusionExtApiServer},
    inclusion_producer::inclusion_producer,
};

pub mod payment;

pub mod validation;
pub use validation::{
    BlockSubmissionValidationApiServer, ExtendedMergedValidationRequestV5,
    ExtendedMergedValidationRequestV6, ExtendedValidationRequestV4, ExtendedValidationRequestV5,
    ExtendedValidationRequestV6, InclusionListTxWithMetadata, InclusionListWithMetadata,
    ValidationApi, ValidationApiConfig,
};
