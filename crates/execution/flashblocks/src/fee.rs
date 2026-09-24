//! The builder's inclusion fee, held as received and served over RPC.

use std::time::{Duration, Instant};

use alloy_primitives::U256;
use serde::{Deserialize, Serialize};

use crate::payload::InclusionFee;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReceivedInclusionFee {
    pub inclusion_fee: InclusionFee,
    pub block_number: u64,
    pub flashblock_index: u64,
    pub received_at: Instant,
}

impl ReceivedInclusionFee {
    pub fn age(&self, now: Instant) -> Duration {
        now.saturating_duration_since(self.received_at)
    }
}

/// Where a served fee value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FeeSource {
    /// The builder's inclusion fee, within its maximum age.
    Builder,
    /// The node's own suggestion.
    Fallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InclusionPriorityFee {
    pub max_priority_fee_per_gas: U256,
    pub source: FeeSource,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub age_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_number: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flashblock_index: Option<u64>,
}

/// The builder's value, when one is held and it is at most `max_age` old.
pub fn builder_inclusion_fee(
    latest: Option<&ReceivedInclusionFee>,
    now: Instant,
    max_age: Duration,
) -> Option<InclusionPriorityFee> {
    let latest = latest?;
    let age = latest.age(now);
    if age > max_age {
        return None;
    }

    Some(InclusionPriorityFee {
        max_priority_fee_per_gas: latest.inclusion_fee.priority_fee,
        source: FeeSource::Builder,
        age_ms: Some(duration_ms(age)),
        block_number: Some(latest.block_number),
        flashblock_index: Some(latest.flashblock_index),
    })
}

/// The node's own suggestion, marked as such.
pub const fn fallback_inclusion_fee(max_priority_fee_per_gas: U256) -> InclusionPriorityFee {
    InclusionPriorityFee {
        max_priority_fee_per_gas,
        source: FeeSource::Fallback,
        age_ms: None,
        block_number: None,
        flashblock_index: None,
    }
}

fn duration_ms(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    const GWEI: u64 = 1_000_000_000;

    fn inclusion_fee() -> InclusionFee {
        InclusionFee { priority_fee: U256::from(11 * GWEI) }
    }

    fn received(age: Duration, now: Instant) -> ReceivedInclusionFee {
        ReceivedInclusionFee {
            inclusion_fee: inclusion_fee(),
            block_number: 1_001,
            flashblock_index: 4,
            received_at: now.checked_sub(age).unwrap(),
        }
    }

    #[test]
    fn a_fresh_inclusion_fee_is_served_with_its_provenance() {
        let now = Instant::now();
        let latest = received(Duration::from_secs(2), now);

        let served = builder_inclusion_fee(Some(&latest), now, Duration::from_secs(15)).unwrap();

        assert_eq!(served.max_priority_fee_per_gas, U256::from(11 * GWEI));
        assert_eq!(served.source, FeeSource::Builder);
        assert_eq!(served.age_ms, Some(2_000));
        assert_eq!(served.block_number, Some(1_001));
        assert_eq!(served.flashblock_index, Some(4));
    }

    #[test]
    fn an_inclusion_fee_older_than_max_age_is_not_served() {
        let now = Instant::now();
        let latest = received(Duration::from_secs(16), now);

        assert_eq!(builder_inclusion_fee(Some(&latest), now, Duration::from_secs(15)), None);
    }

    #[test]
    fn an_inclusion_fee_exactly_at_max_age_is_still_served() {
        let now = Instant::now();
        let latest = received(Duration::from_secs(15), now);

        assert!(builder_inclusion_fee(Some(&latest), now, Duration::from_secs(15)).is_some());
    }

    #[test]
    fn nothing_held_means_nothing_served() {
        assert_eq!(builder_inclusion_fee(None, Instant::now(), Duration::from_secs(15)), None);
    }

    #[test]
    fn the_fallback_carries_no_builder_fields() {
        let served = fallback_inclusion_fee(U256::from(GWEI));

        let json = serde_json::to_value(&served).unwrap();
        assert_eq!(json["maxPriorityFeePerGas"], "0x3b9aca00");
        assert_eq!(json["source"], "fallback");
        assert_eq!(json.as_object().unwrap().len(), 2);
    }

    #[test]
    fn the_response_round_trips_through_json() {
        let now = Instant::now();
        let latest = received(Duration::from_secs(1), now);
        let served = builder_inclusion_fee(Some(&latest), now, Duration::from_secs(15)).unwrap();

        let json = serde_json::to_string(&served).unwrap();
        let decoded: InclusionPriorityFee = serde_json::from_str(&json).unwrap();

        assert!(json.contains("\"source\":\"builder\""));
        assert_eq!(decoded, served);
    }
}
