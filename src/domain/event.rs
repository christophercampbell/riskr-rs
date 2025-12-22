use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::evidence::Evidence;
use super::subject::Subject;
use super::Decision;

/// Unique event identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub String);

impl EventId {
    pub fn new() -> Self {
        EventId(Uuid::new_v4().to_string())
    }

    pub fn from_string(s: impl Into<String>) -> Self {
        EventId(s.into())
    }
}

impl Default for EventId {
    fn default() -> Self {
        EventId::new()
    }
}

/// Blockchain/chain identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Chain(pub String);

impl Chain {
    pub fn new(chain: impl Into<String>) -> Self {
        Chain(chain.into())
    }

    /// Inline chain (not blockchain-specific).
    pub fn inline() -> Self {
        Chain("INLINE".to_string())
    }
}

/// Asset identifier (e.g., "USDC", "ETH").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Asset(pub String);

impl Asset {
    pub fn new(asset: impl Into<String>) -> Self {
        Asset(asset.into())
    }
}

/// Transaction direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Direction {
    Inbound,
    Outbound,
}

/// Schema version for event compatibility.
pub const SCHEMA_VERSION: &str = "v1";

/// Transaction event representing an observed transfer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxEvent {
    /// Schema version for forward compatibility
    pub schema_version: String,

    /// Unique event identifier
    pub event_id: EventId,

    /// When the transaction occurred on-chain
    pub occurred_at: DateTime<Utc>,

    /// When we observed the transaction
    pub observed_at: DateTime<Utc>,

    /// Subject (user/account) of the transaction
    pub subject: Subject,

    /// Blockchain chain identifier
    pub chain: Chain,

    /// Transaction hash (empty for inline requests)
    #[serde(default)]
    pub tx_hash: String,

    /// Direction of the transfer
    pub direction: Direction,

    /// Asset being transferred
    pub asset: Asset,

    /// Amount in base units (string for precision)
    pub amount: String,

    /// USD value at observation time
    #[serde(with = "rust_decimal::serde::str")]
    pub usd_value: Decimal,

    /// Number of confirmations
    #[serde(default)]
    pub confirmations: u32,

    /// Maximum finality depth for the chain
    #[serde(default)]
    pub max_finality_depth: u32,
}

impl TxEvent {
    /// Create a new transaction event with current timestamps.
    pub fn new(subject: Subject, asset: Asset, usd_value: Decimal, direction: Direction) -> Self {
        let now = Utc::now();
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: now,
            observed_at: now,
            subject,
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction,
            asset,
            amount: String::new(),
            usd_value,
            confirmations: 0,
            max_finality_depth: 0,
        }
    }
}

/// Decision stage in the processing pipeline.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DecisionStage {
    /// Initial fast-path decision
    Provisional,
    /// Final decision after full analysis
    Final,
    /// Manual or system override
    Override,
}

/// Decision event recording a risk decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DecisionEvent {
    /// Schema version for forward compatibility
    pub schema_version: String,

    /// Unique decision identifier
    pub decision_id: EventId,

    /// Correlated transaction event ID
    pub event_id: EventId,

    /// When the decision was issued
    pub issued_at: DateTime<Utc>,

    /// Stage of the decision
    pub stage: DecisionStage,

    /// The decision outcome
    pub decision: Decision,

    /// Human-readable decision code
    pub decision_code: String,

    /// Policy version used for this decision
    pub policy_version: String,

    /// Evidence from triggered rules
    pub evidence: Vec<Evidence>,
}

impl DecisionEvent {
    /// Create a new final decision event.
    pub fn new(
        event_id: EventId,
        decision: Decision,
        policy_version: impl Into<String>,
        evidence: Vec<Evidence>,
    ) -> Self {
        DecisionEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            decision_id: EventId::new(),
            event_id,
            issued_at: Utc::now(),
            stage: DecisionStage::Final,
            decision,
            decision_code: Self::pick_code(&evidence),
            policy_version: policy_version.into(),
            evidence,
        }
    }

    /// Pick decision code from evidence.
    fn pick_code(evidence: &[Evidence]) -> String {
        evidence
            .first()
            .map(|e| e.rule_id.clone())
            .unwrap_or_else(|| "OK".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, UserId};
    use smallvec::smallvec;

    fn test_subject() -> Subject {
        Subject {
            user_id: UserId::new("U123"),
            account_id: AccountId::new("A456"),
            addresses: smallvec![Address::new("0xabc")],
            geo_iso: CountryCode::new("US"),
            kyc_tier: KycTier::L1,
        }
    }

    #[test]
    fn test_tx_event_creation() {
        let subject = test_subject();
        let event = TxEvent::new(
            subject,
            Asset::new("USDC"),
            Decimal::new(10000, 2), // $100.00
            Direction::Outbound,
        );

        assert_eq!(event.schema_version, "v1");
        assert_eq!(event.chain.0, "INLINE");
        assert_eq!(event.usd_value, Decimal::new(10000, 2));
    }

    #[test]
    fn test_decision_event_pick_code() {
        let evidence = vec![
            Evidence::new("R1_OFAC", "address", "0xdead"),
            Evidence::new("R2_JURISDICTION", "geo", "IR"),
        ];

        let event = DecisionEvent::new(
            EventId::new(),
            Decision::RejectFatal,
            "2025-01-01.1",
            evidence,
        );

        assert_eq!(event.decision_code, "R1_OFAC");
    }
}
