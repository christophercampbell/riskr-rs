use rust_decimal::Decimal;

use crate::actor::state::UserState;
use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::StreamingRule;

/// Structuring detection rule.
///
/// Detects potential structuring behavior by counting small transactions
/// within a 24-hour window. Triggers when the count exceeds a threshold.
#[derive(Debug)]
pub struct StructuringRule {
    id: String,
    action: Decision,
    /// Threshold below which a transaction is considered "small"
    amount_threshold: Decimal,
    /// Number of small transactions to trigger the rule
    count_threshold: u32,
}

impl StructuringRule {
    /// Create a new structuring detection rule.
    pub fn new(id: String, action: Decision, amount_threshold: Decimal, count_threshold: u32) -> Self {
        StructuringRule {
            id,
            action,
            amount_threshold,
            count_threshold,
        }
    }
}

impl StreamingRule for StructuringRule {
    fn id(&self) -> &str {
        &self.id
    }

    fn evaluate(&self, event: &TxEvent, state: &UserState) -> RuleResult {
        // Count existing small transactions
        let small_count = state.count_small_tx(self.amount_threshold);

        // Check if current transaction is also small
        let current_is_small = event.usd_value < self.amount_threshold;

        // Calculate total including current transaction
        let total_count = if current_is_small {
            small_count + 1
        } else {
            small_count
        };

        // Trigger if count exceeds threshold (not just equals)
        if total_count > self.count_threshold as u64 {
            return RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "small_cnt_24h",
                    total_count.to_string(),
                    self.count_threshold.to_string(),
                ),
            );
        }

        RuleResult::allow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actor::state::TxEntry;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use chrono::Utc;
    use smallvec::smallvec;

    fn test_event(usd_value: i64) -> TxEvent {
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
                kyc_tier: KycTier::L1,
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

    #[test]
    fn test_under_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0), // $10k threshold
            5,                       // 5 count threshold
        );

        let mut state = UserState::new("U1".to_string());
        // Add 3 small transactions
        for _ in 0..3 {
            state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));
        }

        let event = test_event(5000); // 4th small tx
        let result = rule.evaluate(&event, &state);

        assert!(!result.hit); // 4 <= 5, should not trigger
    }

    #[test]
    fn test_at_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let mut state = UserState::new("U1".to_string());
        // Add 4 small transactions
        for _ in 0..4 {
            state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));
        }

        let event = test_event(5000); // 5th small tx
        let result = rule.evaluate(&event, &state);

        assert!(!result.hit); // 5 == 5, at threshold but not over
    }

    #[test]
    fn test_over_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let mut state = UserState::new("U1".to_string());
        // Add 5 small transactions
        for _ in 0..5 {
            state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));
        }

        let event = test_event(5000); // 6th small tx
        let result = rule.evaluate(&event, &state);

        assert!(result.hit);
        assert_eq!(result.decision, Decision::Review);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.value, "6");
        assert_eq!(ev.limit, Some("5".to_string()));
    }

    #[test]
    fn test_large_tx_not_counted() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let mut state = UserState::new("U1".to_string());
        // Add 5 small transactions
        for _ in 0..5 {
            state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));
        }

        // Large transaction ($20k >= $10k threshold)
        let event = test_event(20000);
        let result = rule.evaluate(&event, &state);

        assert!(!result.hit); // Large tx not counted, still at 5
    }

    #[test]
    fn test_mixed_transactions() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let mut state = UserState::new("U1".to_string());
        // Mix of small and large
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));  // small
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(15000, 0))); // large
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(8000, 0)));  // small
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(20000, 0))); // large
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(9000, 0)));  // small

        // Another small tx (4th small)
        let event = test_event(5000);
        let result = rule.evaluate(&event, &state);

        assert!(!result.hit); // Only 4 small txs
    }
}
