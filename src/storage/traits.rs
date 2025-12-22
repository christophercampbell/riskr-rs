// src/storage/traits.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::{Decision, Evidence, Policy, Subject};

/// Record of a transaction for storage.
#[derive(Debug, Clone)]
pub struct TransactionRecord {
    pub subject_id: Uuid,
    pub tx_type: String,
    pub asset: String,
    pub amount: Decimal,
    pub usd_value: Decimal,
    pub dest_address: Option<String>,
}

/// Record of a decision for audit logging.
#[derive(Debug, Clone)]
pub struct DecisionRecord {
    pub subject_id: Option<Uuid>,
    pub request: serde_json::Value,
    pub decision: Decision,
    pub decision_code: String,
    pub policy_version: String,
    pub evidence: Vec<Evidence>,
    pub latency_ms: u32,
}

/// Storage trait for persistence operations.
#[async_trait]
pub trait Storage: Send + Sync {
    // Subjects
    async fn get_subject_by_user_id(&self, user_id: &str) -> anyhow::Result<Option<(Uuid, Subject)>>;
    async fn upsert_subject(&self, subject: &Subject) -> anyhow::Result<Uuid>;

    // Transactions (for streaming rules)
    async fn record_transaction(&self, tx: &TransactionRecord) -> anyhow::Result<Uuid>;
    async fn get_rolling_volume(&self, subject_id: Uuid, window: Duration) -> anyhow::Result<Decimal>;
    async fn get_small_tx_count(&self, subject_id: Uuid, window: Duration, threshold: Decimal) -> anyhow::Result<u32>;

    // Sanctions
    async fn get_all_sanctions(&self) -> anyhow::Result<Vec<String>>;
    async fn is_sanctioned(&self, address: &str) -> anyhow::Result<bool>;

    // Policies
    async fn get_active_policy(&self) -> anyhow::Result<Option<Policy>>;
    async fn set_active_policy(&self, policy: &Policy) -> anyhow::Result<()>;

    // Decisions (audit log)
    async fn record_decision(&self, decision: &DecisionRecord) -> anyhow::Result<Uuid>;
}
