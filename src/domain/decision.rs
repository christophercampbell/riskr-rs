use serde::{Deserialize, Serialize};
use std::fmt;

/// Risk decision outcome with severity ordering.
///
/// Decisions are ordered by severity from least to most severe.
/// When multiple rules trigger, the most severe decision wins.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
#[repr(u8)]
pub enum Decision {
    /// Transaction approved
    Allow = 0,
    /// Temporary denial, client may retry
    SoftDenyRetry = 1,
    /// Automatic hold for processing
    HoldAuto = 2,
    /// Requires manual review
    Review = 3,
    /// Permanently rejected (fatal compliance violation)
    RejectFatal = 4,
}

impl Decision {
    /// Returns the more severe of two decisions.
    #[inline]
    pub fn max(self, other: Self) -> Self {
        std::cmp::max(self, other)
    }

    /// Returns true if this is a fatal rejection.
    #[inline]
    pub fn is_fatal(&self) -> bool {
        *self == Decision::RejectFatal
    }

    /// Returns true if this decision allows the transaction.
    #[inline]
    pub fn is_allowed(&self) -> bool {
        *self == Decision::Allow
    }

    /// Returns true if this decision requires some form of hold or review.
    #[inline]
    pub fn requires_action(&self) -> bool {
        matches!(self, Decision::HoldAuto | Decision::Review)
    }

    /// Returns the severity rank (0-4).
    #[inline]
    pub fn severity(&self) -> u8 {
        *self as u8
    }

    /// Parse from string representation.
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "ALLOW" => Some(Decision::Allow),
            "SOFT_DENY_RETRY" => Some(Decision::SoftDenyRetry),
            "HOLD_AUTO" => Some(Decision::HoldAuto),
            "REVIEW" => Some(Decision::Review),
            "REJECT_FATAL" => Some(Decision::RejectFatal),
            _ => None,
        }
    }
}

impl Default for Decision {
    fn default() -> Self {
        Decision::Allow
    }
}

impl fmt::Display for Decision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Decision::Allow => write!(f, "ALLOW"),
            Decision::SoftDenyRetry => write!(f, "SOFT_DENY_RETRY"),
            Decision::HoldAuto => write!(f, "HOLD_AUTO"),
            Decision::Review => write!(f, "REVIEW"),
            Decision::RejectFatal => write!(f, "REJECT_FATAL"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decision_ordering() {
        assert!(Decision::Allow < Decision::SoftDenyRetry);
        assert!(Decision::SoftDenyRetry < Decision::HoldAuto);
        assert!(Decision::HoldAuto < Decision::Review);
        assert!(Decision::Review < Decision::RejectFatal);
    }

    #[test]
    fn test_decision_max() {
        assert_eq!(Decision::Allow.max(Decision::HoldAuto), Decision::HoldAuto);
        assert_eq!(Decision::RejectFatal.max(Decision::Allow), Decision::RejectFatal);
        assert_eq!(Decision::Review.max(Decision::Review), Decision::Review);
    }

    #[test]
    fn test_decision_serialization() {
        let json = serde_json::to_string(&Decision::RejectFatal).unwrap();
        assert_eq!(json, "\"REJECT_FATAL\"");

        let parsed: Decision = serde_json::from_str("\"HOLD_AUTO\"").unwrap();
        assert_eq!(parsed, Decision::HoldAuto);
    }
}
