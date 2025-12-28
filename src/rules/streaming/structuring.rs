use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::StreamingRule;
use crate::storage::Storage;

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
    pub fn new(
        id: String,
        action: Decision,
        amount_threshold: Decimal,
        count_threshold: u32,
    ) -> Self {
        StructuringRule {
            id,
            action,
            amount_threshold,
            count_threshold,
        }
    }
}

#[async_trait]
impl StreamingRule for StructuringRule {
    fn id(&self) -> &str {
        &self.id
    }

    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> anyhow::Result<RuleResult> {
        // Count existing small transactions
        let small_count = storage
            .get_small_tx_count(subject_id, Duration::hours(24), self.amount_threshold)
            .await?;

        // Check if current transaction is also small
        let current_is_small = event.usd_value < self.amount_threshold;

        // Calculate total including current transaction
        let total_count = if current_is_small {
            small_count + 1
        } else {
            small_count
        };

        // Trigger if count exceeds threshold (not just equals)
        if total_count > self.count_threshold {
            return Ok(RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "small_cnt_24h",
                    total_count.to_string(),
                    self.count_threshold.to_string(),
                ),
            ));
        }

        Ok(RuleResult::allow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use crate::storage::MockStorage;
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

    #[tokio::test]
    async fn test_under_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0), // $10k threshold
            5,                      // 5 count threshold
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Add 3 small transactions
        storage.set_small_tx_count(subject_id, 3);

        let event = test_event(5000); // 4th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // 4 <= 5, should not trigger
    }

    #[tokio::test]
    async fn test_at_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Add 4 small transactions
        storage.set_small_tx_count(subject_id, 4);

        let event = test_event(5000); // 5th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // 5 == 5, at threshold but not over
    }

    #[tokio::test]
    async fn test_over_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Add 5 small transactions
        storage.set_small_tx_count(subject_id, 5);

        let event = test_event(5000); // 6th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(result.hit);
        assert_eq!(result.decision, Decision::Review);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.value, "6");
        assert_eq!(ev.limit, Some("5".to_string()));
    }

    #[tokio::test]
    async fn test_large_tx_not_counted() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Add 5 small transactions
        storage.set_small_tx_count(subject_id, 5);

        // Large transaction ($20k >= $10k threshold)
        let event = test_event(20000);
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // Large tx not counted, still at 5
    }

    #[tokio::test]
    async fn test_mixed_transactions() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Mix of small and large - storage counts only small transactions
        // 3 small transactions: $5k, $8k, $9k (< $10k threshold)
        // 2 large transactions: $15k, $20k (>= $10k threshold) not counted
        storage.set_small_tx_count(subject_id, 3);

        // Another small tx (4th small)
        let event = test_event(5000);
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // Only 4 small txs
    }
}
