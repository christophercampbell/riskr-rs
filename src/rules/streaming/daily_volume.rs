use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::StreamingRule;
use crate::storage::Storage;

/// Daily USD volume limit rule.
///
/// Tracks rolling 24-hour transaction volume per user and triggers
/// when the cumulative volume exceeds the configured threshold.
#[derive(Debug)]
pub struct DailyVolumeRule {
    id: String,
    action: Decision,
    /// Daily volume limit in USD
    limit: Decimal,
}

impl DailyVolumeRule {
    /// Create a new daily volume rule.
    pub fn new(id: String, action: Decision, limit: Decimal) -> Self {
        DailyVolumeRule { id, action, limit }
    }
}

#[async_trait]
impl StreamingRule for DailyVolumeRule {
    fn id(&self) -> &str {
        &self.id
    }

    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> anyhow::Result<RuleResult> {
        // Get current rolling 24h volume
        let current_volume = storage
            .get_rolling_volume(subject_id, Duration::hours(24))
            .await?;

        // Calculate new total including this transaction
        let new_volume = current_volume + event.usd_value;

        // Check if new volume exceeds limit
        if new_volume > self.limit {
            return Ok(RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "daily_usd",
                    new_volume.to_string(),
                    self.limit.to_string(),
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
    async fn test_under_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(10000, 0));

        let event = test_event(10000); // $10k, total would be $20k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit);
    }

    #[tokio::test]
    async fn test_over_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(40000, 0));

        let event = test_event(20000); // $20k, total would be $60k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(result.hit);
        assert_eq!(result.decision, Decision::HoldAuto);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.value, "60000");
        assert_eq!(ev.limit, Some("50000".to_string()));
    }

    #[tokio::test]
    async fn test_exactly_at_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(40000, 0));

        let event = test_event(10000); // $10k, total would be exactly $50k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // At limit, not over
    }

    #[tokio::test]
    async fn test_old_transactions_not_counted() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        // Storage layer handles time window filtering
        // Old transactions would not be included in rolling_volume
        storage.set_rolling_volume(subject_id, Decimal::ZERO);

        let event = test_event(20000);
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // Old tx pruned, only new $20k counted
    }
}
