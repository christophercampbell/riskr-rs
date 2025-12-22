use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::domain::{Decision, Evidence};

/// Response from a decision check.
#[derive(Debug, Serialize)]
pub struct DecisionResponse {
    /// The decision outcome
    pub decision: Decision,

    /// Human-readable decision code
    pub decision_code: String,

    /// Policy version used for this decision
    pub policy_version: String,

    /// Evidence from triggered rules
    pub evidence: Vec<Evidence>,

    /// When this decision expires (if applicable)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<DateTime<Utc>>,
}

impl DecisionResponse {
    /// Create a new decision response.
    pub fn new(
        decision: Decision,
        policy_version: String,
        evidence: Vec<Evidence>,
    ) -> Self {
        let decision_code = if evidence.is_empty() {
            "OK".to_string()
        } else {
            evidence[0].rule_id.clone()
        };

        DecisionResponse {
            decision,
            decision_code,
            policy_version,
            evidence,
            expires_at: None,
        }
    }

    /// Create an allow response with no evidence.
    pub fn allow(policy_version: String) -> Self {
        DecisionResponse {
            decision: Decision::Allow,
            decision_code: "OK".to_string(),
            policy_version,
            evidence: Vec::new(),
            expires_at: None,
        }
    }
}

/// Health check response.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub version: String,
    pub policy_version: String,
    pub uptime_secs: u64,
}

/// Readiness check response.
#[derive(Debug, Serialize)]
pub struct ReadyResponse {
    pub ready: bool,
    pub policy_version: String,
    pub inline_rules: usize,
    pub streaming_rules: usize,
}

/// Error response.
#[derive(Debug, Serialize)]
pub struct ErrorResponse {
    pub error: String,
    pub code: String,
}

impl ErrorResponse {
    pub fn new(error: impl Into<String>, code: impl Into<String>) -> Self {
        ErrorResponse {
            error: error.into(),
            code: code.into(),
        }
    }

    pub fn bad_request(message: impl Into<String>) -> Self {
        ErrorResponse::new(message, "BAD_REQUEST")
    }

    pub fn internal_error(message: impl Into<String>) -> Self {
        ErrorResponse::new(message, "INTERNAL_ERROR")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_response_serialization() {
        let resp = DecisionResponse::new(
            Decision::HoldAuto,
            "v1.0".to_string(),
            vec![Evidence::new("R3_KYC", "usd_value", "5000")],
        );

        let json = serde_json::to_string(&resp).unwrap();

        assert!(json.contains("HOLD_AUTO"));
        assert!(json.contains("R3_KYC"));
        assert!(json.contains("v1.0"));
    }

    #[test]
    fn test_allow_response() {
        let resp = DecisionResponse::allow("v1.0".to_string());

        assert_eq!(resp.decision, Decision::Allow);
        assert_eq!(resp.decision_code, "OK");
        assert!(resp.evidence.is_empty());
    }
}
