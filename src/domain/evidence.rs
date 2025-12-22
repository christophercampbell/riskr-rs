use serde::{Deserialize, Serialize};

/// Evidence captured when a rule triggers.
///
/// Provides audit trail information about why a decision was made.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    /// The rule that triggered
    pub rule_id: String,

    /// Key identifying what was checked (e.g., "address", "geo_iso", "daily_usd")
    pub key: String,

    /// The actual value that triggered the rule
    pub value: String,

    /// The threshold/limit that was exceeded (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<String>,
}

impl Evidence {
    /// Create evidence for a rule hit.
    pub fn new(rule_id: impl Into<String>, key: impl Into<String>, value: impl Into<String>) -> Self {
        Evidence {
            rule_id: rule_id.into(),
            key: key.into(),
            value: value.into(),
            limit: None,
        }
    }

    /// Create evidence with a limit/threshold.
    pub fn with_limit(
        rule_id: impl Into<String>,
        key: impl Into<String>,
        value: impl Into<String>,
        limit: impl Into<String>,
    ) -> Self {
        Evidence {
            rule_id: rule_id.into(),
            key: key.into(),
            value: value.into(),
            limit: Some(limit.into()),
        }
    }
}

/// Result of evaluating a rule.
#[derive(Debug, Clone)]
pub struct RuleResult {
    /// Whether the rule triggered
    pub hit: bool,

    /// The decision if the rule triggered
    pub decision: crate::domain::Decision,

    /// Evidence if the rule triggered
    pub evidence: Option<Evidence>,
}

impl RuleResult {
    /// Create an allowing result (rule did not trigger).
    #[inline]
    pub fn allow() -> Self {
        RuleResult {
            hit: false,
            decision: crate::domain::Decision::Allow,
            evidence: None,
        }
    }

    /// Create a triggering result with evidence.
    pub fn trigger(decision: crate::domain::Decision, evidence: Evidence) -> Self {
        RuleResult {
            hit: true,
            decision,
            evidence: Some(evidence),
        }
    }

    /// Combine two results, taking the more severe decision.
    pub fn combine(self, other: Self) -> Self {
        if other.decision > self.decision {
            other
        } else {
            self
        }
    }
}

impl Default for RuleResult {
    fn default() -> Self {
        RuleResult::allow()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Decision;

    #[test]
    fn test_evidence_creation() {
        let ev = Evidence::new("R1_OFAC", "address", "0xdead");
        assert_eq!(ev.rule_id, "R1_OFAC");
        assert_eq!(ev.key, "address");
        assert_eq!(ev.value, "0xdead");
        assert!(ev.limit.is_none());
    }

    #[test]
    fn test_evidence_with_limit() {
        let ev = Evidence::with_limit("R4_DAILY", "daily_usd", "60000", "50000");
        assert_eq!(ev.limit, Some("50000".to_string()));
    }

    #[test]
    fn test_rule_result_combine() {
        let allow = RuleResult::allow();
        let hold = RuleResult::trigger(
            Decision::HoldAuto,
            Evidence::new("R3", "test", "value"),
        );

        let combined = allow.combine(hold.clone());
        assert!(combined.hit);
        assert_eq!(combined.decision, Decision::HoldAuto);
    }
}
