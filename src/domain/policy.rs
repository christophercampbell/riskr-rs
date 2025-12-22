use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::Decision;

/// Policy configuration defining rules and their parameters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Policy {
    /// Policy version identifier
    #[serde(rename = "policy_version")]
    pub version: String,

    /// Parameters used by rules
    #[serde(default)]
    pub params: RuleParams,

    /// Rule definitions
    #[serde(default)]
    pub rules: Vec<RuleDef>,

    /// Policy signature (for verification)
    #[serde(default)]
    pub signature: String,
}

impl Policy {
    /// Create an empty policy.
    pub fn empty() -> Self {
        Policy {
            version: "0.0.0".to_string(),
            params: RuleParams::default(),
            rules: Vec::new(),
            signature: String::new(),
        }
    }

    /// Compute a hash of the policy for integrity checking.
    pub fn compute_hash(&self) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        let mut hasher = DefaultHasher::new();
        self.version.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

/// Parameters used by rules.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RuleParams {
    /// KYC tier transaction caps in USD
    #[serde(default)]
    pub kyc_tier_caps_usd: HashMap<String, Decimal>,

    /// Daily volume limit in USD
    #[serde(default)]
    pub daily_volume_limit_usd: Option<Decimal>,

    /// Small transaction threshold for structuring detection
    #[serde(default)]
    pub structuring_small_usd: Option<Decimal>,

    /// Count threshold for structuring detection
    #[serde(default)]
    pub structuring_small_count: Option<u32>,
}

impl RuleParams {
    /// Get KYC cap for a tier, returning None if no limit.
    pub fn kyc_cap(&self, tier: &str) -> Option<Decimal> {
        self.kyc_tier_caps_usd.get(tier).copied()
    }
}

/// Rule type identifier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuleType {
    /// OFAC address screening
    OfacAddr,
    /// Jurisdiction blocking
    JurisdictionBlock,
    /// KYC tier transaction cap
    KycTierTxCap,
    /// Daily USD volume limit
    DailyUsdVolume,
    /// Structuring detection (small tx pattern)
    StructuringSmallTx,
}

/// Definition of a single rule.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuleDef {
    /// Unique rule identifier
    pub id: String,

    /// Rule type
    #[serde(rename = "type")]
    pub rule_type: RuleType,

    /// Action to take when rule triggers
    pub action: Decision,

    /// Blocked countries for jurisdiction rule
    #[serde(default)]
    pub blocked_countries: Vec<String>,
}

impl RuleDef {
    /// Check if this rule is an inline rule (stateless).
    pub fn is_inline(&self) -> bool {
        matches!(
            self.rule_type,
            RuleType::OfacAddr | RuleType::JurisdictionBlock | RuleType::KycTierTxCap
        )
    }

    /// Check if this rule is a streaming rule (stateful).
    pub fn is_streaming(&self) -> bool {
        matches!(
            self.rule_type,
            RuleType::DailyUsdVolume | RuleType::StructuringSmallTx
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_policy_deserialization() {
        let yaml = r#"
policy_version: "2025-01-01.1"
params:
  kyc_tier_caps_usd:
    L0: 1000
    L1: 5000
    L2: 100000
  daily_volume_limit_usd: 50000
  structuring_small_usd: 10000
  structuring_small_count: 5
rules:
  - id: R1_OFAC_ADDR
    type: ofac_addr
    action: REJECT_FATAL
  - id: R2_JURISDICTION_BLOCK
    type: jurisdiction_block
    action: REJECT_FATAL
    blocked_countries: ["IR", "KP", "SY", "RU"]
signature: "UNSIGNED-MVP"
"#;

        let policy: Policy = serde_yaml::from_str(yaml).unwrap();
        assert_eq!(policy.version, "2025-01-01.1");
        assert_eq!(policy.rules.len(), 2);
        assert_eq!(policy.rules[0].action, Decision::RejectFatal);
        assert_eq!(
            policy.params.kyc_tier_caps_usd.get("L1"),
            Some(&Decimal::new(5000, 0))
        );
    }

    #[test]
    fn test_rule_classification() {
        let inline_rule = RuleDef {
            id: "R1".to_string(),
            rule_type: RuleType::OfacAddr,
            action: Decision::RejectFatal,
            blocked_countries: vec![],
        };
        assert!(inline_rule.is_inline());
        assert!(!inline_rule.is_streaming());

        let streaming_rule = RuleDef {
            id: "R4".to_string(),
            rule_type: RuleType::DailyUsdVolume,
            action: Decision::HoldAuto,
            blocked_countries: vec![],
        };
        assert!(!streaming_rule.is_inline());
        assert!(streaming_rule.is_streaming());
    }
}
