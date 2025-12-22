use bloomfilter::Bloom;
use std::collections::HashSet;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::InlineRule;

/// OFAC sanctions address screening rule.
///
/// Uses a bloom filter for fast negative checks, with a hash set
/// for definitive verification. This provides O(1) average case
/// for clean addresses (the common case).
#[derive(Debug)]
pub struct OfacRule {
    id: String,
    action: Decision,
    /// Bloom filter for fast negative check
    bloom: Bloom<String>,
    /// Definitive set for positive verification
    addresses: HashSet<String>,
}

impl OfacRule {
    /// Create a new OFAC rule with the given sanctions list.
    pub fn new(id: String, action: Decision, sanctions: HashSet<String>) -> Self {
        // Create bloom filter with expected size and false positive rate
        let item_count = sanctions.len().max(100);
        let fp_rate = 0.01; // 1% false positive rate
        let mut bloom = Bloom::new_for_fp_rate(item_count, fp_rate);

        // Normalize and add all addresses
        let normalized: HashSet<String> = sanctions
            .into_iter()
            .map(|addr| addr.to_lowercase())
            .collect();

        for addr in &normalized {
            bloom.set(addr);
        }

        OfacRule {
            id,
            action,
            bloom,
            addresses: normalized,
        }
    }

    /// Check if an address is sanctioned.
    #[inline]
    fn is_sanctioned(&self, addr: &str) -> bool {
        let normalized = addr.to_lowercase();

        // Fast path: bloom filter says definitely not present
        if !self.bloom.check(&normalized) {
            return false;
        }

        // Slow path: verify in hash set (bloom filter may have false positive)
        self.addresses.contains(&normalized)
    }
}

impl InlineRule for OfacRule {
    fn id(&self) -> &str {
        &self.id
    }

    fn evaluate(&self, event: &TxEvent) -> RuleResult {
        // Check all subject addresses
        for addr in &event.subject.addresses {
            if self.is_sanctioned(addr.as_str()) {
                return RuleResult::trigger(
                    self.action,
                    Evidence::new(&self.id, "address", addr.as_str()),
                );
            }
        }

        RuleResult::allow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use chrono::Utc;
    use rust_decimal::Decimal;
    use smallvec::smallvec;

    fn test_event(addresses: Vec<&str>) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new("U1"),
                account_id: AccountId::new("A1"),
                addresses: addresses.into_iter().map(Address::new).collect(),
                geo_iso: CountryCode::new("US"),
                kyc_tier: KycTier::L1,
            },
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction: Direction::Outbound,
            asset: Asset::new("USDC"),
            amount: "1000".to_string(),
            usd_value: Decimal::new(1000, 0),
            confirmations: 0,
            max_finality_depth: 0,
        }
    }

    #[test]
    fn test_clean_address() {
        let sanctions = HashSet::from(["0xdead".to_string(), "0xbeef".to_string()]);
        let rule = OfacRule::new("R1_OFAC".to_string(), Decision::RejectFatal, sanctions);

        let event = test_event(vec!["0xclean"]);
        let result = rule.evaluate(&event);

        assert!(!result.hit);
        assert_eq!(result.decision, Decision::Allow);
    }

    #[test]
    fn test_sanctioned_address() {
        let sanctions = HashSet::from(["0xdead".to_string(), "0xbeef".to_string()]);
        let rule = OfacRule::new("R1_OFAC".to_string(), Decision::RejectFatal, sanctions);

        let event = test_event(vec!["0xDEAD"]); // Test case insensitivity
        let result = rule.evaluate(&event);

        assert!(result.hit);
        assert_eq!(result.decision, Decision::RejectFatal);
        assert_eq!(result.evidence.as_ref().unwrap().rule_id, "R1_OFAC");
    }

    #[test]
    fn test_multiple_addresses_one_bad() {
        let sanctions = HashSet::from(["0xdead".to_string()]);
        let rule = OfacRule::new("R1_OFAC".to_string(), Decision::RejectFatal, sanctions);

        let event = test_event(vec!["0xclean", "0xdead", "0xsafe"]);
        let result = rule.evaluate(&event);

        assert!(result.hit);
        assert_eq!(result.evidence.as_ref().unwrap().value, "0xdead");
    }

    #[test]
    fn test_empty_addresses() {
        let sanctions = HashSet::from(["0xdead".to_string()]);
        let rule = OfacRule::new("R1_OFAC".to_string(), Decision::RejectFatal, sanctions);

        let event = test_event(vec![]);
        let result = rule.evaluate(&event);

        assert!(!result.hit);
    }
}
