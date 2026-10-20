//! The address disallow list the simulator fetches from its blacklist provider.

use alloy_primitives::Address;
use dashmap::DashSet;
use serde::Deserialize;
use sha2::{Digest, Sha256};

/// A blacklist payload, either a bare array of addresses or one wrapped in `items`.
#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum DisallowListPayload {
    /// A bare array of addresses.
    Bare(Vec<String>),
    /// An array of addresses wrapped in `items`.
    Wrapped {
        /// The addresses.
        items: Vec<String>,
    },
}

impl DisallowListPayload {
    /// Returns the addresses to disallow.
    pub fn into_addresses(self) -> Vec<Address> {
        match self {
            Self::Bare(list) => parse_disallow_list(list),
            Self::Wrapped { items } => parse_disallow_list(items),
        }
    }
}

/// Parses a blacklist payload into the addresses to disallow.
pub fn parse_disallow_list(list: Vec<String>) -> Vec<Address> {
    let mut addrs = Vec::new();
    for hex in list {
        if let Ok(addr) = hex.strip_prefix("0x").unwrap_or(&hex).parse::<Address>() {
            addrs.push(addr);
        }
    }
    addrs
}

/// Fingerprints the disallow list so operators can confirm every node enforces the same one.
///
/// Entries are sorted first because `DashSet` iteration order is not stable. Mirrors reth's
/// `hash_disallow_list` so the digests are comparable against reth-based builders.
pub fn hash_disallow_list(disallow: &DashSet<Address>) -> String {
    let mut sorted: Vec<Address> = disallow.iter().map(|addr| *addr).collect();
    sorted.sort_unstable();

    let mut hasher = Sha256::new();
    for addr in &sorted {
        hasher.update(addr.as_slice());
    }

    format!("{:x}", hasher.finalize())
}

/// Returns the disallow list's digest, or `None` when it still matches `previous`.
pub fn changed_disallow_hash(
    disallow: &DashSet<Address>,
    previous: Option<&str>,
) -> Option<String> {
    let hash = hash_disallow_list(disallow);
    (previous != Some(hash.as_str())).then_some(hash)
}
