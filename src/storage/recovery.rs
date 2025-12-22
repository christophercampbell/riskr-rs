use std::collections::HashMap;
use std::path::Path;
use thiserror::Error;
use tracing::{info, warn};

use crate::actor::pool::ActorPool;
use crate::actor::state::{TxEntry, UserState};

use super::snapshot::{Snapshot, SnapshotError, SnapshotWriter};
use super::wal::{WalEntry, WalError, WalReader};

/// Errors that can occur during recovery.
#[derive(Error, Debug)]
pub enum RecoveryError {
    #[error("WAL error: {0}")]
    Wal(#[from] WalError),

    #[error("Snapshot error: {0}")]
    Snapshot(#[from] SnapshotError),
}

/// Statistics from a recovery operation.
#[derive(Debug)]
pub struct RecoveryStats {
    /// Number of users recovered from snapshot
    pub snapshot_users: usize,
    /// Number of transactions replayed from WAL
    pub wal_transactions: usize,
    /// Number of errors encountered
    pub errors: usize,
    /// Total users after recovery
    pub total_users: usize,
}

/// State recovery from WAL and snapshots.
pub struct StateRecovery {
    snapshot_dir: String,
    wal_path: String,
}

impl StateRecovery {
    /// Create a new recovery instance.
    pub fn new(snapshot_dir: impl Into<String>, wal_path: impl Into<String>) -> Self {
        StateRecovery {
            snapshot_dir: snapshot_dir.into(),
            wal_path: wal_path.into(),
        }
    }

    /// Recover state into an actor pool.
    ///
    /// Recovery process:
    /// 1. Load latest snapshot (if available)
    /// 2. Replay WAL entries after the snapshot
    /// 3. Insert all recovered state into the actor pool
    pub fn recover(&self, pool: &ActorPool) -> Result<RecoveryStats, RecoveryError> {
        let mut stats = RecoveryStats {
            snapshot_users: 0,
            wal_transactions: 0,
            errors: 0,
            total_users: 0,
        };

        // Step 1: Load from snapshot
        let mut states: HashMap<String, UserState> = HashMap::new();
        let mut last_checkpoint: Option<String> = None;

        let snapshot_writer = SnapshotWriter::new(&self.snapshot_dir)?;
        if let Some(snapshot) = snapshot_writer.load_latest()? {
            info!(
                "Loaded snapshot {} with {} users",
                snapshot.id,
                snapshot.states.len()
            );
            last_checkpoint = Some(snapshot.id);
            stats.snapshot_users = snapshot.states.len();
            states = snapshot.states;
        } else {
            info!("No snapshot found, starting from empty state");
        }

        // Step 2: Replay WAL
        if Path::new(&self.wal_path).exists() {
            let reader = WalReader::open(&self.wal_path)?;
            let mut replay_active = last_checkpoint.is_none();

            for entry_result in reader {
                match entry_result {
                    Ok(entry) => match entry {
                        WalEntry::Checkpoint { snapshot_id } => {
                            if Some(&snapshot_id) == last_checkpoint.as_ref() {
                                // Start replaying after this checkpoint
                                replay_active = true;
                                info!("Found matching checkpoint {}, starting replay", snapshot_id);
                            }
                        }
                        WalEntry::Transaction {
                            user_id,
                            timestamp,
                            usd_value,
                        } => {
                            if replay_active {
                                let state = states
                                    .entry(user_id.clone())
                                    .or_insert_with(|| UserState::new(user_id));

                                state.add_tx(TxEntry::new(timestamp, usd_value));
                                stats.wal_transactions += 1;
                            }
                        }
                    },
                    Err(e) => {
                        warn!("Error reading WAL entry: {}", e);
                        stats.errors += 1;
                    }
                }
            }
        } else {
            info!("No WAL file found at {}", self.wal_path);
        }

        // Step 3: Prune expired entries and insert into pool
        for (_, mut state) in states {
            state.prune_expired();
            if !state.is_empty() {
                pool.insert_with_state(state);
            }
        }

        stats.total_users = pool.actor_count();

        info!(
            "Recovery complete: {} users from snapshot, {} WAL transactions, {} total users",
            stats.snapshot_users, stats.wal_transactions, stats.total_users
        );

        Ok(stats)
    }

    /// Create a snapshot from the current actor pool state.
    pub fn create_snapshot(
        &self,
        pool: &ActorPool,
        snapshot_id: String,
    ) -> Result<String, RecoveryError> {
        // Collect all user states
        let states = HashMap::new();

        // This is a simplified approach - in production you'd want
        // to iterate through shards more efficiently
        let pool_stats = pool.stats();
        info!("Creating snapshot of {} users", pool_stats.total_actors);

        // Note: In a real implementation, you'd need a way to iterate
        // all actors without knowing their IDs. For now, this is a placeholder.
        // The actual implementation would need the pool to expose an iterator.

        let snapshot = Snapshot::new(snapshot_id.clone(), states);
        let snapshot_writer = SnapshotWriter::new(&self.snapshot_dir)?;
        snapshot_writer.write(&snapshot)?;

        Ok(snapshot_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::streaming::DailyVolumeRule;
    use crate::domain::Decision;
    use crate::storage::wal::WalWriter;
    use chrono::Utc;
    use rust_decimal::Decimal;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn test_rules() -> Vec<Arc<dyn crate::rules::StreamingRule>> {
        vec![Arc::new(DailyVolumeRule::new(
            "R4".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        ))]
    }

    #[test]
    fn test_recovery_from_wal() {
        let temp_dir = TempDir::new().unwrap();
        let wal_path = temp_dir.path().join("test.wal");
        let snapshot_dir = temp_dir.path().join("snapshots");

        // Write WAL entries
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();

            writer
                .append(&WalEntry::transaction(
                    "user1".to_string(),
                    Utc::now(),
                    Decimal::new(1000, 0),
                ))
                .unwrap();

            writer
                .append(&WalEntry::transaction(
                    "user1".to_string(),
                    Utc::now(),
                    Decimal::new(2000, 0),
                ))
                .unwrap();

            writer
                .append(&WalEntry::transaction(
                    "user2".to_string(),
                    Utc::now(),
                    Decimal::new(5000, 0),
                ))
                .unwrap();

            writer.sync().unwrap();
        }

        // Recover
        let pool = ActorPool::new(test_rules());
        let recovery = StateRecovery::new(
            snapshot_dir.to_string_lossy(),
            wal_path.to_string_lossy(),
        );

        let stats = recovery.recover(&pool).unwrap();

        assert_eq!(stats.wal_transactions, 3);
        assert_eq!(stats.total_users, 2);

        // Verify state
        let actor1 = pool.get("user1").unwrap();
        let state1 = actor1.lock();
        assert_eq!(state1.state().rolling_usd_24h(), Decimal::new(3000, 0));

        let actor2 = pool.get("user2").unwrap();
        let state2 = actor2.lock();
        assert_eq!(state2.state().rolling_usd_24h(), Decimal::new(5000, 0));
    }

    #[test]
    fn test_recovery_with_checkpoint() {
        let temp_dir = TempDir::new().unwrap();
        let wal_path = temp_dir.path().join("test.wal");
        let snapshot_dir = temp_dir.path().join("snapshots");

        // Create snapshot
        std::fs::create_dir_all(&snapshot_dir).unwrap();
        let mut states = HashMap::new();
        let mut user1_state = UserState::new("user1".to_string());
        user1_state.add_tx(TxEntry::new(Utc::now(), Decimal::new(10000, 0)));
        states.insert("user1".to_string(), user1_state);

        let snapshot = Snapshot::new("snap_001".to_string(), states);
        let snapshot_writer = SnapshotWriter::new(&snapshot_dir).unwrap();
        snapshot_writer.write(&snapshot).unwrap();

        // Write WAL with checkpoint and new entries
        {
            let mut writer = WalWriter::open(&wal_path).unwrap();

            // Old entry before checkpoint
            writer
                .append(&WalEntry::transaction(
                    "user1".to_string(),
                    Utc::now(),
                    Decimal::new(100, 0),
                ))
                .unwrap();

            writer
                .append(&WalEntry::checkpoint("snap_001".to_string()))
                .unwrap();

            // New entry after checkpoint
            writer
                .append(&WalEntry::transaction(
                    "user1".to_string(),
                    Utc::now(),
                    Decimal::new(5000, 0),
                ))
                .unwrap();

            writer.sync().unwrap();
        }

        // Recover
        let pool = ActorPool::new(test_rules());
        let recovery = StateRecovery::new(
            snapshot_dir.to_string_lossy(),
            wal_path.to_string_lossy(),
        );

        let stats = recovery.recover(&pool).unwrap();

        // Should have 1 user from snapshot + 1 WAL transaction after checkpoint
        assert_eq!(stats.snapshot_users, 1);
        assert_eq!(stats.wal_transactions, 1);

        let actor = pool.get("user1").unwrap();
        let state = actor.lock();
        // Snapshot: 10000 + WAL: 5000 = 15000
        assert_eq!(state.state().rolling_usd_24h(), Decimal::new(15000, 0));
    }
}
