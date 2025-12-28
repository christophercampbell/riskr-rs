// src/storage/mock.rs
use async_trait::async_trait;
use chrono::Duration;
use parking_lot::Mutex;
use rust_decimal::Decimal;
use std::collections::HashMap;
use uuid::Uuid;

use crate::domain::{Policy, Subject};

use super::traits::{DecisionRecord, Storage, TransactionRecord};

/// Mock storage for testing.
#[derive(Debug, Default)]
pub struct MockStorage {
    subjects: Mutex<HashMap<String, (Uuid, Subject)>>,
    rolling_volumes: Mutex<HashMap<Uuid, Decimal>>,
    small_tx_counts: Mutex<HashMap<Uuid, u32>>,
    sanctions: Mutex<Vec<String>>,
    active_policy: Mutex<Option<Policy>>,
    recorded_transactions: Mutex<Vec<TransactionRecord>>,
    recorded_decisions: Mutex<Vec<DecisionRecord>>,
}

impl MockStorage {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the rolling volume for a subject (for testing).
    pub fn set_rolling_volume(&self, subject_id: Uuid, volume: Decimal) {
        self.rolling_volumes.lock().insert(subject_id, volume);
    }

    /// Set the small tx count for a subject (for testing).
    pub fn set_small_tx_count(&self, subject_id: Uuid, count: u32) {
        self.small_tx_counts.lock().insert(subject_id, count);
    }

    /// Add a sanctioned address (for testing).
    pub fn add_sanction(&self, address: String) {
        self.sanctions.lock().push(address.to_lowercase());
    }

    /// Set active policy (for testing).
    pub fn set_policy(&self, policy: Policy) {
        *self.active_policy.lock() = Some(policy);
    }

    /// Add a subject (for testing).
    pub fn add_subject(&self, subject: Subject) -> Uuid {
        let id = Uuid::new_v4();
        let user_id = subject.user_id.as_str().to_string();
        self.subjects.lock().insert(user_id, (id, subject));
        id
    }

    /// Get recorded transactions (for assertions).
    pub fn get_recorded_transactions(&self) -> Vec<TransactionRecord> {
        self.recorded_transactions.lock().clone()
    }

    /// Get recorded decisions (for assertions).
    pub fn get_recorded_decisions(&self) -> Vec<DecisionRecord> {
        self.recorded_decisions.lock().clone()
    }
}

#[async_trait]
impl Storage for MockStorage {
    async fn get_subject_by_user_id(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Option<(Uuid, Subject)>> {
        Ok(self.subjects.lock().get(user_id).cloned())
    }

    async fn upsert_subject(&self, subject: &Subject) -> anyhow::Result<Uuid> {
        let user_id = subject.user_id.as_str().to_string();
        let mut subjects = self.subjects.lock();

        if let Some((id, _)) = subjects.get(&user_id) {
            let id = *id;
            subjects.insert(user_id, (id, subject.clone()));
            Ok(id)
        } else {
            let id = Uuid::new_v4();
            subjects.insert(user_id, (id, subject.clone()));
            Ok(id)
        }
    }

    async fn record_transaction(&self, tx: &TransactionRecord) -> anyhow::Result<Uuid> {
        self.recorded_transactions.lock().push(tx.clone());
        Ok(Uuid::new_v4())
    }

    async fn get_rolling_volume(
        &self,
        subject_id: Uuid,
        _window: Duration,
    ) -> anyhow::Result<Decimal> {
        Ok(self
            .rolling_volumes
            .lock()
            .get(&subject_id)
            .copied()
            .unwrap_or(Decimal::ZERO))
    }

    async fn get_small_tx_count(
        &self,
        subject_id: Uuid,
        _window: Duration,
        _threshold: Decimal,
    ) -> anyhow::Result<u32> {
        Ok(self
            .small_tx_counts
            .lock()
            .get(&subject_id)
            .copied()
            .unwrap_or(0))
    }

    async fn get_all_sanctions(&self) -> anyhow::Result<Vec<String>> {
        Ok(self.sanctions.lock().clone())
    }

    async fn is_sanctioned(&self, address: &str) -> anyhow::Result<bool> {
        let normalized = address.to_lowercase();
        Ok(self.sanctions.lock().iter().any(|s| s == &normalized))
    }

    async fn get_active_policy(&self) -> anyhow::Result<Option<Policy>> {
        Ok(self.active_policy.lock().clone())
    }

    async fn set_active_policy(&self, policy: &Policy) -> anyhow::Result<()> {
        *self.active_policy.lock() = Some(policy.clone());
        Ok(())
    }

    async fn record_decision(&self, decision: &DecisionRecord) -> anyhow::Result<Uuid> {
        self.recorded_decisions.lock().push(decision.clone());
        Ok(Uuid::new_v4())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, UserId};
    use smallvec::smallvec;

    fn test_subject() -> Subject {
        Subject {
            user_id: UserId::new("U1"),
            account_id: AccountId::new("A1"),
            addresses: smallvec![Address::new("0xabc")],
            geo_iso: CountryCode::new("US"),
            kyc_tier: KycTier::L1,
        }
    }

    #[tokio::test]
    async fn test_subject_upsert_and_get() {
        let storage = MockStorage::new();
        let subject = test_subject();

        let id = storage.upsert_subject(&subject).await.unwrap();
        let (retrieved_id, retrieved) =
            storage.get_subject_by_user_id("U1").await.unwrap().unwrap();

        assert_eq!(id, retrieved_id);
        assert_eq!(retrieved.user_id.as_str(), "U1");
    }

    #[tokio::test]
    async fn test_sanctions_check() {
        let storage = MockStorage::new();
        storage.add_sanction("0xDEAD".to_string());

        assert!(storage.is_sanctioned("0xdead").await.unwrap());
        assert!(storage.is_sanctioned("0xDEAD").await.unwrap());
        assert!(!storage.is_sanctioned("0xbeef").await.unwrap());
    }

    #[tokio::test]
    async fn test_rolling_volume() {
        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();

        storage.set_rolling_volume(subject_id, Decimal::new(45000, 0));

        let volume = storage
            .get_rolling_volume(subject_id, Duration::hours(24))
            .await
            .unwrap();
        assert_eq!(volume, Decimal::new(45000, 0));
    }
}
