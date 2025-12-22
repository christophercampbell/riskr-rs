use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::path::Path;
use thiserror::Error;

/// Errors that can occur during WAL operations.
#[derive(Error, Debug)]
pub enum WalError {
    #[error("IO error: {0}")]
    Io(#[from] io::Error),

    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),

    #[error("Checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: u32, actual: u32 },

    #[error("Invalid WAL entry format")]
    InvalidFormat,
}

/// A single entry in the write-ahead log.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum WalEntry {
    /// A transaction was processed for a user
    #[serde(rename = "tx")]
    Transaction {
        user_id: String,
        timestamp: DateTime<Utc>,
        usd_value: Decimal,
    },

    /// A checkpoint/snapshot was taken
    #[serde(rename = "checkpoint")]
    Checkpoint { snapshot_id: String },
}

impl WalEntry {
    /// Create a transaction entry.
    pub fn transaction(user_id: String, timestamp: DateTime<Utc>, usd_value: Decimal) -> Self {
        WalEntry::Transaction {
            user_id,
            timestamp,
            usd_value,
        }
    }

    /// Create a checkpoint entry.
    pub fn checkpoint(snapshot_id: String) -> Self {
        WalEntry::Checkpoint { snapshot_id }
    }
}

/// Writer for appending entries to the WAL.
pub struct WalWriter {
    writer: BufWriter<File>,
    path: String,
    entries_written: u64,
}

impl WalWriter {
    /// Open or create a WAL file for writing.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let path_str = path.as_ref().to_string_lossy().to_string();

        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)?;

        Ok(WalWriter {
            writer: BufWriter::new(file),
            path: path_str,
            entries_written: 0,
        })
    }

    /// Append an entry to the WAL.
    ///
    /// Each entry is written as a single line of JSON followed by a CRC32 checksum.
    pub fn append(&mut self, entry: &WalEntry) -> Result<(), WalError> {
        let json = serde_json::to_string(entry)?;
        let checksum = crc32fast::hash(json.as_bytes());

        // Write: JSON\tCRC32\n
        writeln!(self.writer, "{}\t{:08x}", json, checksum)?;
        self.entries_written += 1;

        Ok(())
    }

    /// Sync the WAL to disk.
    pub fn sync(&mut self) -> Result<(), WalError> {
        self.writer.flush()?;
        self.writer.get_ref().sync_data()?;
        Ok(())
    }

    /// Get the number of entries written.
    pub fn entries_written(&self) -> u64 {
        self.entries_written
    }

    /// Get the WAL file path.
    pub fn path(&self) -> &str {
        &self.path
    }
}

/// Reader for iterating over WAL entries.
pub struct WalReader {
    reader: BufReader<File>,
    line_buffer: String,
    entries_read: u64,
    errors: u64,
}

impl WalReader {
    /// Open a WAL file for reading.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        let file = File::open(path)?;

        Ok(WalReader {
            reader: BufReader::new(file),
            line_buffer: String::with_capacity(1024),
            entries_read: 0,
            errors: 0,
        })
    }

    /// Read the next entry from the WAL.
    ///
    /// Returns None at end of file.
    /// Skips entries with checksum errors (logs warning).
    pub fn next_entry(&mut self) -> Result<Option<WalEntry>, WalError> {
        loop {
            self.line_buffer.clear();
            let bytes_read = self.reader.read_line(&mut self.line_buffer)?;

            if bytes_read == 0 {
                return Ok(None); // EOF
            }

            let line = self.line_buffer.trim();
            if line.is_empty() {
                continue; // Skip empty lines
            }

            // Parse: JSON\tCRC32
            let parts: Vec<&str> = line.splitn(2, '\t').collect();
            if parts.len() != 2 {
                self.errors += 1;
                tracing::warn!("Invalid WAL line format, skipping");
                continue;
            }

            let json = parts[0];
            let expected_checksum = u32::from_str_radix(parts[1], 16)
                .map_err(|_| WalError::InvalidFormat)?;

            let actual_checksum = crc32fast::hash(json.as_bytes());
            if actual_checksum != expected_checksum {
                self.errors += 1;
                tracing::warn!(
                    "WAL checksum mismatch: expected {:08x}, got {:08x}",
                    expected_checksum,
                    actual_checksum
                );
                continue;
            }

            let entry: WalEntry = serde_json::from_str(json)?;
            self.entries_read += 1;

            return Ok(Some(entry));
        }
    }

    /// Get the number of entries read.
    pub fn entries_read(&self) -> u64 {
        self.entries_read
    }

    /// Get the number of errors encountered.
    pub fn errors(&self) -> u64 {
        self.errors
    }
}

/// Iterator adapter for WalReader.
impl Iterator for WalReader {
    type Item = Result<WalEntry, WalError>;

    fn next(&mut self) -> Option<Self::Item> {
        match self.next_entry() {
            Ok(Some(entry)) => Some(Ok(entry)),
            Ok(None) => None,
            Err(e) => Some(Err(e)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    #[test]
    fn test_write_and_read() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        // Write entries
        {
            let mut writer = WalWriter::open(path).unwrap();

            writer
                .append(&WalEntry::transaction(
                    "user1".to_string(),
                    Utc::now(),
                    Decimal::new(1000, 0),
                ))
                .unwrap();

            writer
                .append(&WalEntry::transaction(
                    "user2".to_string(),
                    Utc::now(),
                    Decimal::new(2000, 0),
                ))
                .unwrap();

            writer.sync().unwrap();
            assert_eq!(writer.entries_written(), 2);
        }

        // Read entries
        {
            let mut reader = WalReader::open(path).unwrap();

            let entry1 = reader.next_entry().unwrap().unwrap();
            match entry1 {
                WalEntry::Transaction { user_id, usd_value, .. } => {
                    assert_eq!(user_id, "user1");
                    assert_eq!(usd_value, Decimal::new(1000, 0));
                }
                _ => panic!("Expected transaction entry"),
            }

            let entry2 = reader.next_entry().unwrap().unwrap();
            match entry2 {
                WalEntry::Transaction { user_id, .. } => {
                    assert_eq!(user_id, "user2");
                }
                _ => panic!("Expected transaction entry"),
            }

            assert!(reader.next_entry().unwrap().is_none());
            assert_eq!(reader.entries_read(), 2);
            assert_eq!(reader.errors(), 0);
        }
    }

    #[test]
    fn test_checkpoint_entry() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        {
            let mut writer = WalWriter::open(path).unwrap();
            writer
                .append(&WalEntry::checkpoint("snap_001".to_string()))
                .unwrap();
            writer.sync().unwrap();
        }

        {
            let mut reader = WalReader::open(path).unwrap();
            let entry = reader.next_entry().unwrap().unwrap();

            match entry {
                WalEntry::Checkpoint { snapshot_id } => {
                    assert_eq!(snapshot_id, "snap_001");
                }
                _ => panic!("Expected checkpoint entry"),
            }
        }
    }

    #[test]
    fn test_iterator() {
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        {
            let mut writer = WalWriter::open(path).unwrap();
            for i in 0..5 {
                writer
                    .append(&WalEntry::transaction(
                        format!("user{}", i),
                        Utc::now(),
                        Decimal::new(i * 1000, 0),
                    ))
                    .unwrap();
            }
            writer.sync().unwrap();
        }

        let reader = WalReader::open(path).unwrap();
        let entries: Vec<_> = reader.filter_map(Result::ok).collect();

        assert_eq!(entries.len(), 5);
    }
}
