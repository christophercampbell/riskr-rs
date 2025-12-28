use rust_decimal::Decimal;
use std::collections::HashMap;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::InlineRule;

/// KYC tier transaction cap rule.
///
/// Enforces per-transaction USD limits based on the user's KYC verification level.
#[derive(Debug)]
pub struct KycCapRule {
    id: String,
    action: Decision,
    /// Per-tier caps in USD
    caps: HashMap<String, Decimal>,
}

impl KycCapRule {
    /// Create a new KYC cap rule with tier limits.
    pub fn new(id: String, action: Decision, caps: HashMap<String, Decimal>) -> Self {
        KycCapRule { id, action, caps }
    }

    /// Get the cap for a KYC tier, if any.
    fn get_cap(&self, tier: &str) -> Option<Decimal> {
        self.caps.get(tier).copied()
    }
}

impl InlineRule for KycCapRule {
    fn id(&self) -> &str {
        &self.id
    }

    fn evaluate(&self, event: &TxEvent) -> RuleResult {
        let tier = event.subject.kyc_tier.as_str();
        let usd_value = event.usd_value;

        // Get cap for this tier; if no cap defined, allow
        let cap = match self.get_cap(tier) {
            Some(c) if c > Decimal::ZERO => c,
            _ => return RuleResult::allow(),
        };

        // Check if transaction exceeds cap
        if usd_value > cap {
            return RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "usd_value",
                    usd_value.to_string(),
                    cap.to_string(),
                ),
            );
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
    use smallvec::smallvec;

    fn test_event(kyc_tier: KycTier, usd_value: i64) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new("U1"),
                account_id: AccountId::new("A1"),
                addresses: smallvec![Address::new("0xabc")],
                geo_iso: CountryCode::new("US"),
                kyc_tier,
            },
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction: Direction::Outbound,
            asset: Asset::new("USDC"),
            amount: usd_value.to_string(),
            usd_value: Decimal::new(usd_value, 0),
            confirmations: 0,
            max_finality_depth: 0,
        }
    }

    fn test_caps() -> HashMap<String, Decimal> {
        HashMap::from([
            ("L0".to_string(), Decimal::new(1000, 0)),   // $1,000
            ("L1".to_string(), Decimal::new(5000, 0)),   // $5,000
            ("L2".to_string(), Decimal::new(100000, 0)), // $100,000
        ])
    }

    #[test]
    fn test_under_limit() {
        let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, test_caps());

        let event = test_event(KycTier::L0, 500);
        let result = rule.evaluate(&event);

        assert!(!result.hit);
        assert_eq!(result.decision, Decision::Allow);
    }

    #[test]
    fn test_at_limit() {
        let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, test_caps());

        let event = test_event(KycTier::L0, 1000);
        let result = rule.evaluate(&event);

        assert!(!result.hit); // At limit, not over
    }

    #[test]
    fn test_over_limit() {
        let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, test_caps());

        let event = test_event(KycTier::L0, 1001);
        let result = rule.evaluate(&event);

        assert!(result.hit);
        assert_eq!(result.decision, Decision::HoldAuto);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.rule_id, "R3_KYC");
        assert_eq!(ev.value, "1001");
        assert_eq!(ev.limit, Some("1000".to_string()));
    }

    #[test]
    fn test_higher_tier_higher_limit() {
        let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, test_caps());

        // L1 can do $5000
        let event = test_event(KycTier::L1, 4000);
        let result = rule.evaluate(&event);
        assert!(!result.hit);

        // L2 can do $100,000
        let event = test_event(KycTier::L2, 50000);
        let result = rule.evaluate(&event);
        assert!(!result.hit);
    }

    #[test]
    fn test_unknown_tier_no_limit() {
        // If tier not in caps map, no limit applies
        let caps = HashMap::from([("L0".to_string(), Decimal::new(1000, 0))]);
        let rule = KycCapRule::new("R3_KYC".to_string(), Decision::HoldAuto, caps);

        // L1 not in caps, so no limit
        let event = test_event(KycTier::L1, 999999);
        let result = rule.evaluate(&event);
        assert!(!result.hit);
    }
}
