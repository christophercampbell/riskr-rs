// src/storage/mod.rs
pub mod traits;

pub use traits::{DecisionRecord, Storage, TransactionRecord};

// Keep old modules for now (will remove later)
pub mod recovery;
pub mod snapshot;
pub mod wal;

pub use recovery::StateRecovery;
pub use snapshot::SnapshotWriter;
pub use wal::{WalEntry, WalReader, WalWriter};
