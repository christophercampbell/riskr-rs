use crate::actor::state::UserState;
use crate::domain::evidence::RuleResult;
use crate::domain::TxEvent;
use std::fmt::Debug;

/// Trait for stateless inline rules.
///
/// Inline rules are evaluated synchronously in the request path
/// and must complete within the latency budget (<10ms total).
///
/// These rules have no access to historical state and make
/// decisions based solely on the current transaction.
pub trait InlineRule: Send + Sync + Debug {
    /// Unique identifier for this rule.
    fn id(&self) -> &str;

    /// Evaluate the rule against a transaction.
    ///
    /// Returns a RuleResult indicating whether the rule triggered
    /// and what decision/evidence resulted.
    fn evaluate(&self, event: &TxEvent) -> RuleResult;
}

/// Trait for stateful streaming rules.
///
/// Streaming rules have access to the user's historical state
/// (rolling 24h window) and can make decisions based on patterns
/// over time.
pub trait StreamingRule: Send + Sync + Debug {
    /// Unique identifier for this rule.
    fn id(&self) -> &str;

    /// Evaluate the rule against a transaction with state context.
    ///
    /// The state provides access to the user's transaction history
    /// for the rolling window (typically 24h).
    fn evaluate(&self, event: &TxEvent, state: &UserState) -> RuleResult;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Decision, Evidence};

    #[derive(Debug)]
    struct TestInlineRule {
        id: String,
        should_trigger: bool,
    }

    impl InlineRule for TestInlineRule {
        fn id(&self) -> &str {
            &self.id
        }

        fn evaluate(&self, _event: &TxEvent) -> RuleResult {
            if self.should_trigger {
                RuleResult::trigger(
                    Decision::HoldAuto,
                    Evidence::new(&self.id, "test", "triggered"),
                )
            } else {
                RuleResult::allow()
            }
        }
    }

    #[test]
    fn test_inline_rule_trait() {
        let rule = TestInlineRule {
            id: "TEST_RULE".to_string(),
            should_trigger: true,
        };

        assert_eq!(rule.id(), "TEST_RULE");
    }
}
