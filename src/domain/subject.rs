use serde::{Deserialize, Serialize};
use smallvec::SmallVec;
use std::fmt;

/// Unique user identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub String);

impl UserId {
    pub fn new(id: impl Into<String>) -> Self {
        UserId(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique account identifier.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct AccountId(pub String);

impl AccountId {
    pub fn new(id: impl Into<String>) -> Self {
        AccountId(id.into())
    }
}

/// Blockchain address (hex string, case-insensitive).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Address(String);

impl Address {
    /// Create a new address, normalizing to lowercase.
    pub fn new(addr: impl Into<String>) -> Self {
        Address(addr.into().to_lowercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Get the normalized (lowercase) form for comparison.
    pub fn normalized(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Address {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// ISO 3166-1 alpha-2 country code.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CountryCode(String);

impl CountryCode {
    /// Create a new country code, normalizing to uppercase.
    pub fn new(code: impl Into<String>) -> Self {
        CountryCode(code.into().to_uppercase())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for CountryCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// KYC verification tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum KycTier {
    /// Unverified or minimal verification
    #[default]
    #[serde(rename = "L0")]
    L0,
    /// Basic verification (ID check)
    #[serde(rename = "L1")]
    L1,
    /// Full verification (ID + address + source of funds)
    #[serde(rename = "L2")]
    L2,
}

impl KycTier {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "L0" => Some(KycTier::L0),
            "L1" => Some(KycTier::L1),
            "L2" => Some(KycTier::L2),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            KycTier::L0 => "L0",
            KycTier::L1 => "L1",
            KycTier::L2 => "L2",
        }
    }
}

impl fmt::Display for KycTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Subject of a transaction - the user/account being evaluated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Subject {
    /// Unique user identifier
    pub user_id: UserId,

    /// Account identifier within the user
    pub account_id: AccountId,

    /// Blockchain addresses associated with this subject
    /// SmallVec optimizes for the common case of 1-4 addresses
    #[serde(default)]
    pub addresses: SmallVec<[Address; 4]>,

    /// Geographic location (ISO country code)
    pub geo_iso: CountryCode,

    /// KYC verification level
    #[serde(rename = "kyc_level")]
    pub kyc_tier: KycTier,
}

impl Subject {
    /// Check if any of the subject's addresses match the given predicate.
    pub fn has_address<F>(&self, predicate: F) -> bool
    where
        F: Fn(&Address) -> bool,
    {
        self.addresses.iter().any(predicate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_address_normalization() {
        let addr = Address::new("0xABCDEF123456");
        assert_eq!(addr.normalized(), "0xabcdef123456");
    }

    #[test]
    fn test_country_code_normalization() {
        let code = CountryCode::new("us");
        assert_eq!(code.as_str(), "US");
    }

    #[test]
    fn test_kyc_tier_serialization() {
        let tier = KycTier::L2;
        let json = serde_json::to_string(&tier).unwrap();
        assert_eq!(json, "\"L2\"");

        let parsed: KycTier = serde_json::from_str("\"L1\"").unwrap();
        assert_eq!(parsed, KycTier::L1);
    }
}
