use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use smallvec::SmallVec;

use crate::domain::event::{Asset, Chain, Direction, EventId, TxEvent, SCHEMA_VERSION};
use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
use chrono::Utc;

/// Request for a decision check.
#[derive(Debug, Serialize, Deserialize)]
pub struct DecisionRequest {
    /// Subject information
    pub subject: SubjectRequest,

    /// Transaction details
    pub tx: TxRequest,

    /// Additional context (optional)
    #[serde(default)]
    pub context: serde_json::Value,
}

/// Subject portion of the request.
#[derive(Debug, Serialize, Deserialize)]
pub struct SubjectRequest {
    pub user_id: String,
    pub account_id: String,
    #[serde(default)]
    pub addresses: Vec<String>,
    pub geo_iso: String,
    #[serde(rename = "kyc_level")]
    pub kyc_tier: String,
}

/// Transaction portion of the request.
#[derive(Debug, Serialize, Deserialize)]
pub struct TxRequest {
    /// Transaction type (withdraw, deposit, etc.)
    #[serde(rename = "type")]
    pub tx_type: String,

    /// Asset being transferred
    pub asset: String,

    /// Amount in base units (string for precision)
    #[serde(default)]
    pub amount: String,

    /// USD value of the transaction
    pub usd_value: f64,

    /// Destination address (for withdrawals)
    #[serde(default)]
    pub dest_address: Option<String>,
}

impl DecisionRequest {
    /// Convert to a TxEvent for rule evaluation.
    pub fn to_tx_event(&self) -> TxEvent {
        let now = Utc::now();

        // Parse KYC tier
        let kyc_tier = KycTier::from_str(&self.subject.kyc_tier).unwrap_or_default();

        // Convert addresses
        let addresses: SmallVec<[Address; 4]> = self
            .subject
            .addresses
            .iter()
            .map(Address::new)
            .collect();

        // Determine direction from tx type
        let direction = if self.tx.tx_type.to_lowercase().contains("withdraw") {
            Direction::Outbound
        } else {
            Direction::Inbound
        };

        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: now,
            observed_at: now,
            subject: Subject {
                user_id: UserId::new(&self.subject.user_id),
                account_id: AccountId::new(&self.subject.account_id),
                addresses,
                geo_iso: CountryCode::new(&self.subject.geo_iso),
                kyc_tier,
            },
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction,
            asset: Asset::new(&self.tx.asset),
            amount: self.tx.amount.clone(),
            usd_value: Decimal::from_f64_retain(self.tx.usd_value).unwrap_or(Decimal::ZERO),
            confirmations: 0,
            max_finality_depth: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_request_deserialization() {
        let json = r#"{
            "subject": {
                "user_id": "U123",
                "account_id": "A456",
                "addresses": ["0xabc", "0xdef"],
                "geo_iso": "US",
                "kyc_level": "L1"
            },
            "tx": {
                "type": "withdraw",
                "asset": "USDC",
                "amount": "1000000",
                "usd_value": 1000.00,
                "dest_address": "0x1234"
            },
            "context": {}
        }"#;

        let req: DecisionRequest = serde_json::from_str(json).unwrap();

        assert_eq!(req.subject.user_id, "U123");
        assert_eq!(req.tx.usd_value, 1000.0);
        assert_eq!(req.subject.addresses.len(), 2);
    }

    #[test]
    fn test_to_tx_event() {
        let json = r#"{
            "subject": {
                "user_id": "U123",
                "account_id": "A456",
                "addresses": ["0xABC"],
                "geo_iso": "us",
                "kyc_level": "L2"
            },
            "tx": {
                "type": "withdraw",
                "asset": "USDC",
                "usd_value": 5000.50
            }
        }"#;

        let req: DecisionRequest = serde_json::from_str(json).unwrap();
        let event = req.to_tx_event();

        assert_eq!(event.subject.user_id.as_str(), "U123");
        assert_eq!(event.subject.geo_iso.as_str(), "US");
        assert_eq!(event.subject.kyc_tier, KycTier::L2);
        assert_eq!(event.direction, Direction::Outbound);
        // Address should be normalized to lowercase
        assert_eq!(event.subject.addresses[0].as_str(), "0xabc");
    }
}
