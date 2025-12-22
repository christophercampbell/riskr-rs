use std::collections::HashSet;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::InlineRule;

/// Jurisdiction blocking rule.
///
/// Blocks transactions from specific countries based on ISO 3166-1 alpha-2 codes.
#[derive(Debug)]
pub struct JurisdictionRule {
    id: String,
    action: Decision,
    /// Set of blocked country codes (uppercase)
    blocked: HashSet<String>,
}

impl JurisdictionRule {
    /// Create a new jurisdiction rule with blocked countries.
    pub fn new(id: String, action: Decision, blocked_countries: HashSet<String>) -> Self {
        // Normalize to uppercase
        let blocked = blocked_countries
            .into_iter()
            .map(|c| c.to_uppercase())
            .collect();

        JurisdictionRule {
            id,
            action,
            blocked,
        }
    }

    /// Check if a country code is blocked.
    #[inline]
    fn is_blocked(&self, country_code: &str) -> bool {
        self.blocked.contains(&country_code.to_uppercase())
    }
}

impl InlineRule for JurisdictionRule {
    fn id(&self) -> &str {
        &self.id
    }

    fn evaluate(&self, event: &TxEvent) -> RuleResult {
        let country = event.subject.geo_iso.as_str();

        if self.is_blocked(country) {
            return RuleResult::trigger(
                self.action,
                Evidence::new(&self.id, "geo_iso", country),
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
    use rust_decimal::Decimal;
    use smallvec::smallvec;

    fn test_event(country: &str) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new("U1"),
                account_id: AccountId::new("A1"),
                addresses: smallvec![Address::new("0xabc")],
                geo_iso: CountryCode::new(country),
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
    fn test_allowed_country() {
        let blocked = HashSet::from(["IR".to_string(), "KP".to_string()]);
        let rule = JurisdictionRule::new("R2_JURISDICTION".to_string(), Decision::RejectFatal, blocked);

        let event = test_event("US");
        let result = rule.evaluate(&event);

        assert!(!result.hit);
        assert_eq!(result.decision, Decision::Allow);
    }

    #[test]
    fn test_blocked_country() {
        let blocked = HashSet::from(["IR".to_string(), "KP".to_string()]);
        let rule = JurisdictionRule::new("R2_JURISDICTION".to_string(), Decision::RejectFatal, blocked);

        let event = test_event("IR");
        let result = rule.evaluate(&event);

        assert!(result.hit);
        assert_eq!(result.decision, Decision::RejectFatal);
        assert_eq!(result.evidence.as_ref().unwrap().value, "IR");
    }

    #[test]
    fn test_blocked_country_lowercase() {
        let blocked = HashSet::from(["IR".to_string()]);
        let rule = JurisdictionRule::new("R2_JURISDICTION".to_string(), Decision::RejectFatal, blocked);

        let event = test_event("ir"); // lowercase input
        let result = rule.evaluate(&event);

        assert!(result.hit);
    }

    #[test]
    fn test_empty_country() {
        let blocked = HashSet::from(["IR".to_string()]);
        let rule = JurisdictionRule::new("R2_JURISDICTION".to_string(), Decision::RejectFatal, blocked);

        let event = test_event("");
        let result = rule.evaluate(&event);

        assert!(!result.hit);
    }
}
