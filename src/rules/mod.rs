pub mod inline;
pub mod streaming;
pub mod traits;

pub use inline::{JurisdictionRule, KycCapRule, OfacRule};
pub use streaming::{DailyVolumeRule, StructuringRule};
pub use traits::{InlineRule, StreamingRule};

use crate::domain::{Policy, RuleType};
use std::collections::HashSet;
use std::sync::Arc;

/// Collection of compiled rules ready for evaluation.
pub struct RuleSet {
    pub inline: Vec<Arc<dyn InlineRule>>,
    pub streaming: Vec<Arc<dyn StreamingRule>>,
    pub policy_version: String,
}

impl RuleSet {
    /// Build rules from a policy and sanctions list.
    pub fn from_policy(policy: &Policy, sanctions: HashSet<String>) -> Self {
        let mut inline: Vec<Arc<dyn InlineRule>> = Vec::new();
        let mut streaming: Vec<Arc<dyn StreamingRule>> = Vec::new();

        for rule_def in &policy.rules {
            match rule_def.rule_type {
                RuleType::OfacAddr => {
                    inline.push(Arc::new(OfacRule::new(
                        rule_def.id.clone(),
                        rule_def.action,
                        sanctions.clone(),
                    )));
                }
                RuleType::JurisdictionBlock => {
                    let blocked: HashSet<String> = rule_def
                        .blocked_countries
                        .iter()
                        .map(|c| c.to_uppercase())
                        .collect();
                    inline.push(Arc::new(JurisdictionRule::new(
                        rule_def.id.clone(),
                        rule_def.action,
                        blocked,
                    )));
                }
                RuleType::KycTierTxCap => {
                    inline.push(Arc::new(KycCapRule::new(
                        rule_def.id.clone(),
                        rule_def.action,
                        policy.params.kyc_tier_caps_usd.clone(),
                    )));
                }
                RuleType::DailyUsdVolume => {
                    if let Some(limit) = policy.params.daily_volume_limit_usd {
                        streaming.push(Arc::new(DailyVolumeRule::new(
                            rule_def.id.clone(),
                            rule_def.action,
                            limit,
                        )));
                    }
                }
                RuleType::StructuringSmallTx => {
                    if let (Some(threshold), Some(count)) = (
                        policy.params.structuring_small_usd,
                        policy.params.structuring_small_count,
                    ) {
                        streaming.push(Arc::new(StructuringRule::new(
                            rule_def.id.clone(),
                            rule_def.action,
                            threshold,
                            count,
                        )));
                    }
                }
            }
        }

        RuleSet {
            inline,
            streaming,
            policy_version: policy.version.clone(),
        }
    }

    /// Create an empty rule set.
    pub fn empty() -> Self {
        RuleSet {
            inline: Vec::new(),
            streaming: Vec::new(),
            policy_version: "0.0.0".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::{Decision, Policy, RuleDef, RuleParams, RuleType};
    use rust_decimal::Decimal;
    use std::collections::HashMap;

    #[test]
    fn test_ruleset_from_policy() {
        let mut kyc_caps = HashMap::new();
        kyc_caps.insert("L0".to_string(), Decimal::new(1000, 0));

        let policy = Policy {
            version: "test-1".to_string(),
            params: RuleParams {
                kyc_tier_caps_usd: kyc_caps,
                daily_volume_limit_usd: Some(Decimal::new(50000, 0)),
                structuring_small_usd: Some(Decimal::new(10000, 0)),
                structuring_small_count: Some(5),
            },
            rules: vec![
                RuleDef {
                    id: "R1".to_string(),
                    rule_type: RuleType::OfacAddr,
                    action: Decision::RejectFatal,
                    blocked_countries: vec![],
                },
                RuleDef {
                    id: "R4".to_string(),
                    rule_type: RuleType::DailyUsdVolume,
                    action: Decision::HoldAuto,
                    blocked_countries: vec![],
                },
            ],
            signature: String::new(),
        };

        let sanctions = HashSet::from(["0xdead".to_string()]);
        let ruleset = RuleSet::from_policy(&policy, sanctions);

        assert_eq!(ruleset.inline.len(), 1);
        assert_eq!(ruleset.streaming.len(), 1);
        assert_eq!(ruleset.policy_version, "test-1");
    }
}
