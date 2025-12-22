use chrono::Utc;
use std::sync::Arc;

use crate::domain::evidence::RuleResult;
use crate::domain::TxEvent;
use crate::rules::traits::StreamingRule;

use super::state::{TxEntry, UserState};

/// Actor representing a single user's state and rule evaluation.
///
/// Each user has their own actor that owns their rolling window state.
/// This ensures no shared mutable state between users and enables
/// lock-free access patterns within a shard.
pub struct UserActor {
    /// The user's rolling window state
    state: UserState,

    /// Streaming rules to evaluate (shared across all actors)
    streaming_rules: Arc<Vec<Arc<dyn StreamingRule>>>,
}

impl UserActor {
    /// Create a new user actor with empty state.
    pub fn new(user_id: String, streaming_rules: Arc<Vec<Arc<dyn StreamingRule>>>) -> Self {
        UserActor {
            state: UserState::new(user_id),
            streaming_rules,
        }
    }

    /// Create a user actor with existing state (for recovery).
    pub fn with_state(state: UserState, streaming_rules: Arc<Vec<Arc<dyn StreamingRule>>>) -> Self {
        UserActor {
            state,
            streaming_rules,
        }
    }

    /// Get the user ID.
    pub fn user_id(&self) -> &str {
        &self.state.user_id
    }

    /// Get read access to the user's state.
    pub fn state(&self) -> &UserState {
        &self.state
    }

    /// Evaluate streaming rules for a transaction.
    ///
    /// This:
    /// 1. Prunes expired entries from the rolling window
    /// 2. Evaluates all streaming rules
    /// 3. Updates state with the new transaction
    /// 4. Returns the combined rule result
    pub fn evaluate(&mut self, event: &TxEvent) -> RuleResult {
        // Prune expired entries first
        self.state.prune_expired();

        // Evaluate all streaming rules
        let mut result = RuleResult::allow();
        for rule in self.streaming_rules.iter() {
            let rule_result = rule.evaluate(event, &self.state);
            if rule_result.decision > result.decision {
                result = rule_result;
            }
        }

        // Update state with this transaction
        self.state.add_tx(TxEntry::new(
            event.occurred_at,
            event.usd_value,
        ));

        result
    }

    /// Update the streaming rules (for hot reload).
    pub fn update_rules(&mut self, rules: Arc<Vec<Arc<dyn StreamingRule>>>) {
        self.streaming_rules = rules;
    }

    /// Check if this actor is idle (no recent activity).
    pub fn is_idle(&self, idle_threshold_secs: i64) -> bool {
        let now = Utc::now();
        let idle_duration = now - self.state.last_access;
        idle_duration.num_seconds() > idle_threshold_secs
    }

    /// Get the number of transactions in state.
    pub fn entry_count(&self) -> usize {
        self.state.entry_count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use crate::domain::Decision;
    use crate::rules::streaming::DailyVolumeRule;
    use rust_decimal::Decimal;
    use smallvec::smallvec;

    fn test_event(user_id: &str, usd_value: i64) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new(user_id),
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

    fn test_rules() -> Arc<Vec<Arc<dyn StreamingRule>>> {
        Arc::new(vec![Arc::new(DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        )) as Arc<dyn StreamingRule>])
    }

    #[test]
    fn test_actor_evaluate() {
        let mut actor = UserActor::new("U1".to_string(), test_rules());

        // First transaction
        let event = test_event("U1", 10000);
        let result = actor.evaluate(&event);
        assert!(!result.hit);
        assert_eq!(actor.entry_count(), 1);

        // Second transaction
        let event = test_event("U1", 20000);
        let result = actor.evaluate(&event);
        assert!(!result.hit);
        assert_eq!(actor.entry_count(), 2);
    }

    #[test]
    fn test_actor_triggers_rule() {
        let mut actor = UserActor::new("U1".to_string(), test_rules());

        // Add $40k
        let event = test_event("U1", 40000);
        actor.evaluate(&event);

        // Add $20k more (total $60k, over $50k limit)
        let event = test_event("U1", 20000);
        let result = actor.evaluate(&event);

        assert!(result.hit);
        assert_eq!(result.decision, Decision::HoldAuto);
    }

    #[test]
    fn test_actor_state_accumulates() {
        let mut actor = UserActor::new("U1".to_string(), test_rules());

        for i in 0..5 {
            let event = test_event("U1", 1000 * (i + 1));
            actor.evaluate(&event);
        }

        assert_eq!(actor.entry_count(), 5);
        // 1000 + 2000 + 3000 + 4000 + 5000 = 15000
        assert_eq!(actor.state().rolling_usd_24h(), Decimal::new(15000, 0));
    }
}
