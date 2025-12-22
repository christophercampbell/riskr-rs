use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{BufReader, BufWriter};
use std::path::{Path, PathBuf};
use thiserror::Error;

use crate::actor::state::UserState;

/// Errors that can occur during snapshot operations.
#[derive(Error, Debug)]
pub enum SnapshotError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

/// A complete snapshot of all user states.
#[derive(Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Unique snapshot identifier
    pub id: String,

    /// Timestamp when snapshot was created
    pub created_at: chrono::DateTime<chrono::Utc>,

    /// User states keyed by user ID
    pub states: HashMap<String, UserState>,
}

impl Snapshot {
    /// Create a new snapshot with the given states.
    pub fn new(id: String, states: HashMap<String, UserState>) -> Self {
        Snapshot {
            id,
            created_at: chrono::Utc::now(),
            states,
        }
    }
}

/// Writer for creating state snapshots.
pub struct SnapshotWriter {
    directory: PathBuf,
}

impl SnapshotWriter {
    /// Create a new snapshot writer.
    pub fn new(directory: impl AsRef<Path>) -> Result<Self, SnapshotError> {
        let dir = directory.as_ref().to_path_buf();
        fs::create_dir_all(&dir)?;

        Ok(SnapshotWriter { directory: dir })
    }

    /// Write a snapshot to disk.
    ///
    /// Returns the path to the written snapshot file.
    pub fn write(&self, snapshot: &Snapshot) -> Result<PathBuf, SnapshotError> {
        let filename = format!("snapshot_{}.json", snapshot.id);
        let path = self.directory.join(&filename);

        // Write to temp file first, then rename for atomicity
        let temp_path = self.directory.join(format!(".{}.tmp", filename));

        {
            let file = File::create(&temp_path)?;
            let writer = BufWriter::new(file);
            serde_json::to_writer(writer, snapshot)?;
        }

        fs::rename(&temp_path, &path)?;

        Ok(path)
    }

    /// List all snapshot files in the directory.
    pub fn list_snapshots(&self) -> Result<Vec<PathBuf>, SnapshotError> {
        let mut snapshots = Vec::new();

        for entry in fs::read_dir(&self.directory)? {
            let entry = entry?;
            let path = entry.path();

            if path.extension().map(|e| e == "json").unwrap_or(false) {
                if let Some(name) = path.file_name() {
                    if name.to_string_lossy().starts_with("snapshot_") {
                        snapshots.push(path);
                    }
                }
            }
        }

        // Sort by modification time (newest first)
        snapshots.sort_by(|a, b| {
            let a_time = fs::metadata(a).and_then(|m| m.modified()).ok();
            let b_time = fs::metadata(b).and_then(|m| m.modified()).ok();
            b_time.cmp(&a_time)
        });

        Ok(snapshots)
    }

    /// Load the most recent snapshot.
    pub fn load_latest(&self) -> Result<Option<Snapshot>, SnapshotError> {
        let snapshots = self.list_snapshots()?;

        if let Some(path) = snapshots.first() {
            let snapshot = Self::load_snapshot(path)?;
            Ok(Some(snapshot))
        } else {
            Ok(None)
        }
    }

    /// Load a snapshot from a file.
    pub fn load_snapshot(path: impl AsRef<Path>) -> Result<Snapshot, SnapshotError> {
        let file = File::open(path)?;
        let reader = BufReader::new(file);
        let snapshot = serde_json::from_reader(reader)?;
        Ok(snapshot)
    }

    /// Delete old snapshots, keeping only the N most recent.
    pub fn cleanup(&self, keep: usize) -> Result<usize, SnapshotError> {
        let snapshots = self.list_snapshots()?;
        let mut deleted = 0;

        for path in snapshots.into_iter().skip(keep) {
            fs::remove_file(path)?;
            deleted += 1;
        }

        Ok(deleted)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_write_and_load_snapshot() {
        let temp_dir = TempDir::new().unwrap();
        let writer = SnapshotWriter::new(temp_dir.path()).unwrap();

        let mut states = HashMap::new();
        states.insert("user1".to_string(), UserState::new("user1".to_string()));
        states.insert("user2".to_string(), UserState::new("user2".to_string()));

        let snapshot = Snapshot::new("test_001".to_string(), states);
        let path = writer.write(&snapshot).unwrap();

        assert!(path.exists());

        let loaded = SnapshotWriter::load_snapshot(&path).unwrap();
        assert_eq!(loaded.id, "test_001");
        assert_eq!(loaded.states.len(), 2);
        assert!(loaded.states.contains_key("user1"));
        assert!(loaded.states.contains_key("user2"));
    }

    #[test]
    fn test_load_latest() {
        let temp_dir = TempDir::new().unwrap();
        let writer = SnapshotWriter::new(temp_dir.path()).unwrap();

        // Write multiple snapshots
        for i in 1..=3 {
            let states = HashMap::new();
            let snapshot = Snapshot::new(format!("{:03}", i), states);
            writer.write(&snapshot).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let latest = writer.load_latest().unwrap().unwrap();
        assert_eq!(latest.id, "003");
    }

    #[test]
    fn test_cleanup() {
        let temp_dir = TempDir::new().unwrap();
        let writer = SnapshotWriter::new(temp_dir.path()).unwrap();

        // Write 5 snapshots
        for i in 1..=5 {
            let states = HashMap::new();
            let snapshot = Snapshot::new(format!("{:03}", i), states);
            writer.write(&snapshot).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }

        let deleted = writer.cleanup(2).unwrap();
        assert_eq!(deleted, 3);

        let remaining = writer.list_snapshots().unwrap();
        assert_eq!(remaining.len(), 2);
    }

    #[test]
    fn test_empty_directory() {
        let temp_dir = TempDir::new().unwrap();
        let writer = SnapshotWriter::new(temp_dir.path()).unwrap();

        let snapshots = writer.list_snapshots().unwrap();
        assert!(snapshots.is_empty());

        let latest = writer.load_latest().unwrap();
        assert!(latest.is_none());
    }
}
