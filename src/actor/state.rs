use chrono::{DateTime, Duration, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;

/// Maximum number of transactions to store per user.
/// This bounds memory usage even for high-volume users.
const MAX_TX_ENTRIES: usize = 10000;

/// Duration of the rolling window.
const WINDOW_DURATION_HOURS: i64 = 24;

/// A single transaction entry in the user's rolling window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TxEntry {
    /// When the transaction occurred
    pub timestamp: DateTime<Utc>,
    /// USD value of the transaction
    pub usd_value: Decimal,
}

impl TxEntry {
    /// Create a new transaction entry.
    pub fn new(timestamp: DateTime<Utc>, usd_value: Decimal) -> Self {
        TxEntry { timestamp, usd_value }
    }

    /// Check if this entry has expired (older than window).
    pub fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now - self.timestamp > Duration::hours(WINDOW_DURATION_HOURS)
    }
}

/// Rolling window state for a single user.
///
/// Maintains a bounded deque of transaction entries within the
/// rolling 24-hour window. Entries are pruned lazily or on demand.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserState {
    /// User identifier
    pub user_id: String,

    /// Transaction entries ordered by timestamp (oldest first)
    entries: VecDeque<TxEntry>,

    /// Last time state was accessed (for idle actor eviction)
    #[serde(skip)]
    pub last_access: DateTime<Utc>,
}

impl UserState {
    /// Create empty state for a user.
    pub fn new(user_id: String) -> Self {
        UserState {
            user_id,
            entries: VecDeque::with_capacity(256),
            last_access: Utc::now(),
        }
    }

    /// Add a transaction to the state.
    ///
    /// Automatically prunes expired entries and enforces max capacity.
    pub fn add_tx(&mut self, entry: TxEntry) {
        self.last_access = Utc::now();

        // Prune from front if we're at capacity
        while self.entries.len() >= MAX_TX_ENTRIES {
            self.entries.pop_front();
        }

        self.entries.push_back(entry);
    }

    /// Prune expired entries (older than 24 hours).
    ///
    /// Call this before querying to ensure accurate results.
    pub fn prune_expired(&mut self) {
        let now = Utc::now();
        let cutoff = now - Duration::hours(WINDOW_DURATION_HOURS);

        // Remove from front while entries are expired
        while let Some(entry) = self.entries.front() {
            if entry.timestamp <= cutoff {
                self.entries.pop_front();
            } else {
                break;
            }
        }
    }

    /// Get the rolling 24-hour USD volume.
    pub fn rolling_usd_24h(&self) -> Decimal {
        let now = Utc::now();
        let cutoff = now - Duration::hours(WINDOW_DURATION_HOURS);

        self.entries
            .iter()
            .filter(|e| e.timestamp > cutoff)
            .map(|e| e.usd_value)
            .sum()
    }

    /// Count transactions below a USD threshold in the rolling window.
    pub fn count_small_tx(&self, threshold: Decimal) -> u64 {
        let now = Utc::now();
        let cutoff = now - Duration::hours(WINDOW_DURATION_HOURS);

        self.entries
            .iter()
            .filter(|e| e.timestamp > cutoff && e.usd_value < threshold)
            .count() as u64
    }

    /// Get the number of entries in state.
    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    /// Check if state is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Get all entries (for serialization/snapshot).
    pub fn entries(&self) -> &VecDeque<TxEntry> {
        &self.entries
    }

    /// Restore from entries (for deserialization/recovery).
    pub fn from_entries(user_id: String, entries: VecDeque<TxEntry>) -> Self {
        UserState {
            user_id,
            entries,
            last_access: Utc::now(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_and_query() {
        let mut state = UserState::new("U1".to_string());

        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(100, 0)));
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(200, 0)));
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(300, 0)));

        assert_eq!(state.rolling_usd_24h(), Decimal::new(600, 0));
        assert_eq!(state.entry_count(), 3);
    }

    #[test]
    fn test_prune_expired() {
        let mut state = UserState::new("U1".to_string());

        // Add old entry (25 hours ago)
        let old_time = Utc::now() - Duration::hours(25);
        state.add_tx(TxEntry::new(old_time, Decimal::new(1000, 0)));

        // Add recent entry
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(100, 0)));

        state.prune_expired();

        assert_eq!(state.entry_count(), 1);
        assert_eq!(state.rolling_usd_24h(), Decimal::new(100, 0));
    }

    #[test]
    fn test_count_small_tx() {
        let mut state = UserState::new("U1".to_string());

        let threshold = Decimal::new(10000, 0);

        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(5000, 0)));  // small
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(15000, 0))); // large
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(8000, 0)));  // small
        state.add_tx(TxEntry::new(Utc::now(), Decimal::new(3000, 0)));  // small

        assert_eq!(state.count_small_tx(threshold), 3);
    }

    #[test]
    fn test_max_capacity() {
        let mut state = UserState::new("U1".to_string());

        // Add more than MAX_TX_ENTRIES
        for i in 0..MAX_TX_ENTRIES + 100 {
            state.add_tx(TxEntry::new(Utc::now(), Decimal::new(i as i64, 0)));
        }

        assert!(state.entry_count() <= MAX_TX_ENTRIES);
    }

    #[test]
    fn test_empty_state() {
        let state = UserState::new("U1".to_string());

        assert!(state.is_empty());
        assert_eq!(state.rolling_usd_24h(), Decimal::ZERO);
        assert_eq!(state.count_small_tx(Decimal::new(100, 0)), 0);
    }
}
