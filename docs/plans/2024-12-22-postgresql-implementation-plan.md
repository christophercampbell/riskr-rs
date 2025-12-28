# PostgreSQL Storage Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Replace in-memory actor model with PostgreSQL-backed storage for horizontal scaling.

**Architecture:** Stateless instances query PostgreSQL for subject state. Streaming rules become async, querying the `transactions` table. Connection pooling via sqlx replaces actor pooling.

**Tech Stack:** sqlx 0.8 (async PostgreSQL), Docker Compose for local dev, existing Axum/Tokio stack.

---

## Task 1: Add sqlx Dependencies

**Files:**
- Modify: `Cargo.toml`

**Step 1: Add sqlx and remove obsolete deps**

```toml
# In [dependencies], add:
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "rust_decimal"] }

# Remove these lines:
# crc32fast = "1.4"
# memmap2 = "0.9"
```

**Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles (warnings about unused code ok)

**Step 3: Commit**

```bash
git add Cargo.toml
git commit -m "deps: add sqlx, remove wal-related crates"
```

---

## Task 2: Create Database Schema Migration

**Files:**
- Create: `migrations/0001_initial_schema.sql`

**Step 1: Create migrations directory**

Run: `mkdir -p migrations`

**Step 2: Write the migration file**

```sql
-- migrations/0001_initial_schema.sql

-- Subjects (users/accounts)
CREATE TABLE subjects (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    user_id TEXT NOT NULL UNIQUE,
    account_id TEXT,
    kyc_level TEXT NOT NULL DEFAULT 'L0',
    geo_iso TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Addresses linked to subjects
CREATE TABLE subject_addresses (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id) ON DELETE CASCADE,
    address TEXT NOT NULL,
    chain TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE(subject_id, address)
);
CREATE INDEX idx_subject_addresses_address ON subject_addresses(address);

-- Transaction history (for streaming rules)
CREATE TABLE transactions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID NOT NULL REFERENCES subjects(id),
    tx_type TEXT NOT NULL,
    asset TEXT NOT NULL,
    amount NUMERIC NOT NULL,
    usd_value NUMERIC NOT NULL,
    dest_address TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_transactions_subject_time ON transactions(subject_id, created_at DESC);

-- Sanctions list
CREATE TABLE sanctions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    address TEXT NOT NULL UNIQUE,
    source TEXT,
    added_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Policies (JSONB for flexibility)
CREATE TABLE policies (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    version TEXT NOT NULL UNIQUE,
    config JSONB NOT NULL,
    active BOOLEAN NOT NULL DEFAULT false,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE UNIQUE INDEX idx_policies_single_active ON policies(active) WHERE active = true;

-- Decision audit log
CREATE TABLE decisions (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    subject_id UUID REFERENCES subjects(id),
    request JSONB NOT NULL,
    decision TEXT NOT NULL,
    decision_code TEXT NOT NULL,
    policy_version TEXT NOT NULL,
    evidence JSONB,
    latency_ms INTEGER,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_decisions_subject_time ON decisions(subject_id, created_at DESC);
```

**Step 3: Commit**

```bash
git add migrations/
git commit -m "schema: add initial PostgreSQL migration"
```

---

## Task 3: Create Docker Compose for Local PostgreSQL

**Files:**
- Create: `docker/docker-compose.yml`

**Step 1: Create docker directory**

Run: `mkdir -p docker`

**Step 2: Write docker-compose.yml**

```yaml
# docker/docker-compose.yml
version: '3.8'

services:
  postgres:
    image: postgres:16-alpine
    container_name: riskr-postgres
    environment:
      POSTGRES_USER: riskr
      POSTGRES_PASSWORD: riskr_dev
      POSTGRES_DB: riskr
    ports:
      - "5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data
      - ../migrations:/docker-entrypoint-initdb.d:ro
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U riskr"]
      interval: 5s
      timeout: 5s
      retries: 5

volumes:
  postgres_data:
```

**Step 3: Test PostgreSQL starts**

Run: `docker compose -f docker/docker-compose.yml up -d && sleep 5 && docker compose -f docker/docker-compose.yml ps`
Expected: postgres container running, healthy

**Step 4: Commit**

```bash
git add docker/
git commit -m "infra: add docker-compose for local PostgreSQL"
```

---

## Task 4: Define Storage Trait

**Files:**
- Create: `src/storage/traits.rs`
- Modify: `src/storage/mod.rs`

**Step 1: Write the Storage trait**

```rust
// src/storage/traits.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::{Decision, Evidence, Policy, Subject, TxEvent};

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
```

**Step 2: Update storage/mod.rs**

```rust
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
```

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles (need to add async-trait dep)

**Step 4: Add async-trait dependency**

In `Cargo.toml`, add:
```toml
async-trait = "0.1"
```

**Step 5: Run cargo check again**

Run: `cargo check`
Expected: Compiles successfully

**Step 6: Commit**

```bash
git add src/storage/traits.rs src/storage/mod.rs Cargo.toml
git commit -m "feat(storage): define Storage trait for PostgreSQL"
```

---

## Task 5: Implement MockStorage for Tests

**Files:**
- Create: `src/storage/mock.rs`
- Modify: `src/storage/mod.rs`

**Step 1: Write MockStorage**

```rust
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
    async fn get_subject_by_user_id(&self, user_id: &str) -> anyhow::Result<Option<(Uuid, Subject)>> {
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

    async fn get_rolling_volume(&self, subject_id: Uuid, _window: Duration) -> anyhow::Result<Decimal> {
        Ok(self.rolling_volumes.lock().get(&subject_id).copied().unwrap_or(Decimal::ZERO))
    }

    async fn get_small_tx_count(&self, subject_id: Uuid, _window: Duration, _threshold: Decimal) -> anyhow::Result<u32> {
        Ok(self.small_tx_counts.lock().get(&subject_id).copied().unwrap_or(0))
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
        let (retrieved_id, retrieved) = storage.get_subject_by_user_id("U1").await.unwrap().unwrap();

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

        let volume = storage.get_rolling_volume(subject_id, Duration::hours(24)).await.unwrap();
        assert_eq!(volume, Decimal::new(45000, 0));
    }
}
```

**Step 2: Update storage/mod.rs**

```rust
// src/storage/mod.rs
pub mod mock;
pub mod traits;

pub use mock::MockStorage;
pub use traits::{DecisionRecord, Storage, TransactionRecord};

// Keep old modules for now (will remove later)
pub mod recovery;
pub mod snapshot;
pub mod wal;

pub use recovery::StateRecovery;
pub use snapshot::SnapshotWriter;
pub use wal::{WalEntry, WalReader, WalWriter};
```

**Step 3: Run tests**

Run: `cargo test storage::mock`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/storage/mock.rs src/storage/mod.rs
git commit -m "feat(storage): add MockStorage for testing"
```

---

## Task 6: Implement PostgresStorage

**Files:**
- Create: `src/storage/postgres.rs`
- Modify: `src/storage/mod.rs`

**Step 1: Write PostgresStorage**

```rust
// src/storage/postgres.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::{Policy, Subject};
use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, UserId};

use super::traits::{DecisionRecord, Storage, TransactionRecord};

/// PostgreSQL storage implementation.
#[derive(Debug, Clone)]
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Create a new PostgresStorage with the given connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Connect to PostgreSQL and create storage.
    pub async fn connect(database_url: &str, min_connections: u32, max_connections: u32) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .min_connections(min_connections)
            .max_connections(max_connections)
            .connect(database_url)
            .await?;

        Ok(Self::new(pool))
    }

    /// Run migrations.
    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Get the connection pool (for health checks).
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn get_subject_by_user_id(&self, user_id: &str) -> anyhow::Result<Option<(Uuid, Subject)>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, account_id, kyc_level, geo_iso
            FROM subjects
            WHERE user_id = $1
            "#
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let id: Uuid = row.get("id");
        let user_id_str: String = row.get("user_id");
        let account_id: Option<String> = row.get("account_id");
        let kyc_level: String = row.get("kyc_level");
        let geo_iso: Option<String> = row.get("geo_iso");

        // Fetch addresses
        let address_rows = sqlx::query(
            "SELECT address FROM subject_addresses WHERE subject_id = $1"
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await?;

        let addresses: smallvec::SmallVec<[Address; 4]> = address_rows
            .iter()
            .map(|r| Address::new(r.get::<String, _>("address")))
            .collect();

        let subject = Subject {
            user_id: UserId::new(user_id_str),
            account_id: AccountId::new(account_id.unwrap_or_default()),
            addresses,
            geo_iso: CountryCode::new(geo_iso.unwrap_or_else(|| "XX".to_string())),
            kyc_tier: KycTier::from_str(&kyc_level).unwrap_or_default(),
        };

        Ok(Some((id, subject)))
    }

    async fn upsert_subject(&self, subject: &Subject) -> anyhow::Result<Uuid> {
        let user_id = subject.user_id.as_str();
        let account_id = subject.account_id.0.as_str();
        let kyc_level = subject.kyc_tier.as_str();
        let geo_iso = subject.geo_iso.as_str();

        // Upsert subject
        let row = sqlx::query(
            r#"
            INSERT INTO subjects (user_id, account_id, kyc_level, geo_iso)
            VALUES ($1, $2, $3, $4)
            ON CONFLICT (user_id) DO UPDATE SET
                account_id = EXCLUDED.account_id,
                kyc_level = EXCLUDED.kyc_level,
                geo_iso = EXCLUDED.geo_iso,
                updated_at = now()
            RETURNING id
            "#
        )
        .bind(user_id)
        .bind(account_id)
        .bind(kyc_level)
        .bind(geo_iso)
        .fetch_one(&self.pool)
        .await?;

        let subject_id: Uuid = row.get("id");

        // Upsert addresses
        for addr in &subject.addresses {
            sqlx::query(
                r#"
                INSERT INTO subject_addresses (subject_id, address)
                VALUES ($1, $2)
                ON CONFLICT (subject_id, address) DO NOTHING
                "#
            )
            .bind(subject_id)
            .bind(addr.as_str())
            .execute(&self.pool)
            .await?;
        }

        Ok(subject_id)
    }

    async fn record_transaction(&self, tx: &TransactionRecord) -> anyhow::Result<Uuid> {
        let row = sqlx::query(
            r#"
            INSERT INTO transactions (subject_id, tx_type, asset, amount, usd_value, dest_address)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#
        )
        .bind(tx.subject_id)
        .bind(&tx.tx_type)
        .bind(&tx.asset)
        .bind(tx.amount)
        .bind(tx.usd_value)
        .bind(&tx.dest_address)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("id"))
    }

    async fn get_rolling_volume(&self, subject_id: Uuid, window: Duration) -> anyhow::Result<Decimal> {
        let interval = format!("{} hours", window.num_hours());

        let row = sqlx::query(
            r#"
            SELECT COALESCE(SUM(usd_value), 0) as total
            FROM transactions
            WHERE subject_id = $1
              AND created_at > now() - $2::interval
            "#
        )
        .bind(subject_id)
        .bind(&interval)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("total"))
    }

    async fn get_small_tx_count(&self, subject_id: Uuid, window: Duration, threshold: Decimal) -> anyhow::Result<u32> {
        let interval = format!("{} hours", window.num_hours());

        let row = sqlx::query(
            r#"
            SELECT COUNT(*) as cnt
            FROM transactions
            WHERE subject_id = $1
              AND created_at > now() - $2::interval
              AND usd_value < $3
            "#
        )
        .bind(subject_id)
        .bind(&interval)
        .bind(threshold)
        .fetch_one(&self.pool)
        .await?;

        let count: i64 = row.get("cnt");
        Ok(count as u32)
    }

    async fn get_all_sanctions(&self) -> anyhow::Result<Vec<String>> {
        let rows = sqlx::query("SELECT address FROM sanctions")
            .fetch_all(&self.pool)
            .await?;

        Ok(rows.iter().map(|r| r.get("address")).collect())
    }

    async fn is_sanctioned(&self, address: &str) -> anyhow::Result<bool> {
        let normalized = address.to_lowercase();

        let row = sqlx::query(
            "SELECT EXISTS(SELECT 1 FROM sanctions WHERE address = $1) as exists"
        )
        .bind(&normalized)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("exists"))
    }

    async fn get_active_policy(&self) -> anyhow::Result<Option<Policy>> {
        let row = sqlx::query(
            "SELECT config FROM policies WHERE active = true LIMIT 1"
        )
        .fetch_optional(&self.pool)
        .await?;

        match row {
            Some(r) => {
                let config: serde_json::Value = r.get("config");
                let policy: Policy = serde_json::from_value(config)?;
                Ok(Some(policy))
            }
            None => Ok(None),
        }
    }

    async fn set_active_policy(&self, policy: &Policy) -> anyhow::Result<()> {
        let config = serde_json::to_value(policy)?;

        // Deactivate all policies first
        sqlx::query("UPDATE policies SET active = false WHERE active = true")
            .execute(&self.pool)
            .await?;

        // Insert or update the policy
        sqlx::query(
            r#"
            INSERT INTO policies (version, config, active)
            VALUES ($1, $2, true)
            ON CONFLICT (version) DO UPDATE SET
                config = EXCLUDED.config,
                active = true
            "#
        )
        .bind(&policy.version)
        .bind(&config)
        .execute(&self.pool)
        .await?;

        Ok(())
    }

    async fn record_decision(&self, decision: &DecisionRecord) -> anyhow::Result<Uuid> {
        let evidence = serde_json::to_value(&decision.evidence)?;
        let decision_str = format!("{:?}", decision.decision);

        let row = sqlx::query(
            r#"
            INSERT INTO decisions (subject_id, request, decision, decision_code, policy_version, evidence, latency_ms)
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id
            "#
        )
        .bind(decision.subject_id)
        .bind(&decision.request)
        .bind(&decision_str)
        .bind(&decision.decision_code)
        .bind(&decision.policy_version)
        .bind(&evidence)
        .bind(decision.latency_ms as i32)
        .fetch_one(&self.pool)
        .await?;

        Ok(row.get("id"))
    }
}
```

**Step 2: Update storage/mod.rs**

```rust
// src/storage/mod.rs
pub mod mock;
pub mod postgres;
pub mod traits;

pub use mock::MockStorage;
pub use postgres::PostgresStorage;
pub use traits::{DecisionRecord, Storage, TransactionRecord};

// Keep old modules for now (will remove later)
pub mod recovery;
pub mod snapshot;
pub mod wal;

pub use recovery::StateRecovery;
pub use snapshot::SnapshotWriter;
pub use wal::{WalEntry, WalReader, WalWriter};
```

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles (may need smallvec import fix)

**Step 4: Commit**

```bash
git add src/storage/postgres.rs src/storage/mod.rs
git commit -m "feat(storage): implement PostgresStorage"
```

---

## Task 7: Update Config for Database Options

**Files:**
- Modify: `src/config/mod.rs`

**Step 1: Add database config fields**

Add these fields to the `Config` struct after line 61 (before `pub graceful_shutdown`):

```rust
    /// PostgreSQL connection string
    #[arg(long, env = "RISKR_DATABASE_URL")]
    pub database_url: Option<String>,

    /// Minimum database pool connections
    #[arg(long, default_value = "2", env = "RISKR_DB_POOL_MIN")]
    pub db_pool_min: u32,

    /// Maximum database pool connections
    #[arg(long, default_value = "10", env = "RISKR_DB_POOL_MAX")]
    pub db_pool_max: u32,

    /// Run database migrations on startup
    #[arg(long, default_value = "false", env = "RISKR_RUN_MIGRATIONS")]
    pub run_migrations: bool,
```

**Step 2: Update Default impl**

Add to the Default impl (after `actor_idle_secs: 3600,`):

```rust
            database_url: None,
            db_pool_min: 2,
            db_pool_max: 10,
            run_migrations: false,
```

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles

**Step 4: Commit**

```bash
git add src/config/mod.rs
git commit -m "feat(config): add database configuration options"
```

---

## Task 8: Refactor StreamingRule Trait to Async

**Files:**
- Modify: `src/rules/traits.rs`

**Step 1: Make StreamingRule async**

Replace the `StreamingRule` trait (lines 24-38) with:

```rust
/// Trait for stateful streaming rules.
///
/// Streaming rules query storage for historical state
/// and can make decisions based on patterns over time.
#[async_trait::async_trait]
pub trait StreamingRule: Send + Sync + Debug {
    /// Unique identifier for this rule.
    fn id(&self) -> &str;

    /// Evaluate the rule against a transaction with storage access.
    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: uuid::Uuid,
        storage: &dyn crate::storage::Storage,
    ) -> anyhow::Result<RuleResult>;
}
```

**Step 2: Add uuid import**

Add at top of file:
```rust
use uuid::Uuid;
```

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Errors in streaming rules (expected, will fix next)

**Step 4: Commit (WIP)**

```bash
git add src/rules/traits.rs
git commit -m "refactor(rules): make StreamingRule async with Storage"
```

---

## Task 9: Update DailyVolumeRule for Async Storage

**Files:**
- Modify: `src/rules/streaming/daily_volume.rs`

**Step 1: Replace the entire file**

```rust
// src/rules/streaming/daily_volume.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::StreamingRule;
use crate::storage::Storage;

/// Daily USD volume limit rule.
///
/// Tracks rolling 24-hour transaction volume per user and triggers
/// when the cumulative volume exceeds the configured threshold.
#[derive(Debug)]
pub struct DailyVolumeRule {
    id: String,
    action: Decision,
    /// Daily volume limit in USD
    limit: Decimal,
}

impl DailyVolumeRule {
    /// Create a new daily volume rule.
    pub fn new(id: String, action: Decision, limit: Decimal) -> Self {
        DailyVolumeRule { id, action, limit }
    }
}

#[async_trait]
impl StreamingRule for DailyVolumeRule {
    fn id(&self) -> &str {
        &self.id
    }

    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> anyhow::Result<RuleResult> {
        // Get current rolling 24h volume from storage
        let current_volume = storage
            .get_rolling_volume(subject_id, Duration::hours(24))
            .await?;

        // Calculate new total including this transaction
        let new_volume = current_volume + event.usd_value;

        // Check if new volume exceeds limit
        if new_volume > self.limit {
            return Ok(RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "daily_usd",
                    new_volume.to_string(),
                    self.limit.to_string(),
                ),
            ));
        }

        Ok(RuleResult::allow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use crate::storage::MockStorage;
    use chrono::Utc;
    use smallvec::smallvec;

    fn test_event(usd_value: i64) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new("U1"),
                account_id: AccountId::new("A1"),
                addresses: smallvec![Address::new("0xabc")],
                geo_iso: CountryCode::new("US"),
                kyc_tier: KycTier::L1,
            },
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction: Direction::Outbound,
            asset: Asset::new("USDC"),
            amount: usd_value.to_string(),
            usd_value: Decimal::new(usd_value, 0),
            confirmations: 0,
            max_finality_depth: 0,
        }
    }

    #[tokio::test]
    async fn test_under_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(10000, 0));

        let event = test_event(10000); // $10k, total would be $20k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit);
    }

    #[tokio::test]
    async fn test_over_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(40000, 0));

        let event = test_event(20000); // $20k, total would be $60k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(result.hit);
        assert_eq!(result.decision, Decision::HoldAuto);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.value, "60000");
        assert_eq!(ev.limit, Some("50000".to_string()));
    }

    #[tokio::test]
    async fn test_exactly_at_limit() {
        let rule = DailyVolumeRule::new(
            "R4_DAILY".to_string(),
            Decision::HoldAuto,
            Decimal::new(50000, 0),
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_rolling_volume(subject_id, Decimal::new(40000, 0));

        let event = test_event(10000); // $10k, total would be exactly $50k
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // At limit, not over
    }
}
```

**Step 2: Run cargo check**

Run: `cargo check`
Expected: Errors in structuring rule (expected, will fix next)

**Step 3: Commit**

```bash
git add src/rules/streaming/daily_volume.rs
git commit -m "refactor(rules): update DailyVolumeRule for async Storage"
```

---

## Task 10: Update StructuringRule for Async Storage

**Files:**
- Modify: `src/rules/streaming/structuring.rs`

**Step 1: Replace the entire file**

```rust
// src/rules/streaming/structuring.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use uuid::Uuid;

use crate::domain::evidence::RuleResult;
use crate::domain::{Decision, Evidence, TxEvent};
use crate::rules::traits::StreamingRule;
use crate::storage::Storage;

/// Structuring detection rule.
///
/// Detects potential structuring behavior by counting small transactions
/// within a 24-hour window. Triggers when the count exceeds a threshold.
#[derive(Debug)]
pub struct StructuringRule {
    id: String,
    action: Decision,
    /// Threshold below which a transaction is considered "small"
    amount_threshold: Decimal,
    /// Number of small transactions to trigger the rule
    count_threshold: u32,
}

impl StructuringRule {
    /// Create a new structuring detection rule.
    pub fn new(id: String, action: Decision, amount_threshold: Decimal, count_threshold: u32) -> Self {
        StructuringRule {
            id,
            action,
            amount_threshold,
            count_threshold,
        }
    }
}

#[async_trait]
impl StreamingRule for StructuringRule {
    fn id(&self) -> &str {
        &self.id
    }

    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> anyhow::Result<RuleResult> {
        // Count existing small transactions from storage
        let small_count = storage
            .get_small_tx_count(subject_id, Duration::hours(24), self.amount_threshold)
            .await?;

        // Check if current transaction is also small
        let current_is_small = event.usd_value < self.amount_threshold;

        // Calculate total including current transaction
        let total_count = if current_is_small {
            small_count + 1
        } else {
            small_count
        };

        // Trigger if count exceeds threshold (not just equals)
        if total_count > self.count_threshold {
            return Ok(RuleResult::trigger(
                self.action,
                Evidence::with_limit(
                    &self.id,
                    "small_cnt_24h",
                    total_count.to_string(),
                    self.count_threshold.to_string(),
                ),
            ));
        }

        Ok(RuleResult::allow())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::event::{Asset, Chain, Direction, EventId, SCHEMA_VERSION};
    use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use crate::storage::MockStorage;
    use chrono::Utc;
    use smallvec::smallvec;

    fn test_event(usd_value: i64) -> TxEvent {
        TxEvent {
            schema_version: SCHEMA_VERSION.to_string(),
            event_id: EventId::new(),
            occurred_at: Utc::now(),
            observed_at: Utc::now(),
            subject: Subject {
                user_id: UserId::new("U1"),
                account_id: AccountId::new("A1"),
                addresses: smallvec![Address::new("0xabc")],
                geo_iso: CountryCode::new("US"),
                kyc_tier: KycTier::L1,
            },
            chain: Chain::inline(),
            tx_hash: String::new(),
            direction: Direction::Outbound,
            asset: Asset::new("USDC"),
            amount: usd_value.to_string(),
            usd_value: Decimal::new(usd_value, 0),
            confirmations: 0,
            max_finality_depth: 0,
        }
    }

    #[tokio::test]
    async fn test_under_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0), // $10k threshold
            5,                       // 5 count threshold
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_small_tx_count(subject_id, 3); // 3 existing small txs

        let event = test_event(5000); // 4th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // 4 <= 5, should not trigger
    }

    #[tokio::test]
    async fn test_at_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_small_tx_count(subject_id, 4); // 4 existing small txs

        let event = test_event(5000); // 5th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // 5 == 5, at threshold but not over
    }

    #[tokio::test]
    async fn test_over_count_threshold() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_small_tx_count(subject_id, 5); // 5 existing small txs

        let event = test_event(5000); // 6th small tx
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(result.hit);
        assert_eq!(result.decision, Decision::Review);
        let ev = result.evidence.unwrap();
        assert_eq!(ev.value, "6");
        assert_eq!(ev.limit, Some("5".to_string()));
    }

    #[tokio::test]
    async fn test_large_tx_not_counted() {
        let rule = StructuringRule::new(
            "R5_STRUCT".to_string(),
            Decision::Review,
            Decimal::new(10000, 0),
            5,
        );

        let storage = MockStorage::new();
        let subject_id = Uuid::new_v4();
        storage.set_small_tx_count(subject_id, 5); // 5 existing small txs

        // Large transaction ($20k >= $10k threshold)
        let event = test_event(20000);
        let result = rule.evaluate(&event, subject_id, &storage).await.unwrap();

        assert!(!result.hit); // Large tx not counted, still at 5
    }
}
```

**Step 2: Run cargo check**

Run: `cargo check`
Expected: May have errors in rules/mod.rs or api/routes.rs (expected)

**Step 3: Commit**

```bash
git add src/rules/streaming/structuring.rs
git commit -m "refactor(rules): update StructuringRule for async Storage"
```

---

## Task 11: Update Rules Module Exports

**Files:**
- Modify: `src/rules/mod.rs`
- Modify: `src/rules/streaming/mod.rs`

**Step 1: Read current rules/mod.rs**

Read the file first to understand structure.

**Step 2: Update rules/streaming/mod.rs**

Remove `use crate::actor::state::UserState;` if present, keep exports.

**Step 3: Update rules/mod.rs**

Remove actor-related imports. The RuleSet struct needs updating to not hold streaming rules directly (they'll be evaluated differently now).

**Step 4: Run cargo check**

Run: `cargo check`
Expected: Errors in api/routes.rs (actor pool usage)

**Step 5: Commit**

```bash
git add src/rules/mod.rs src/rules/streaming/mod.rs
git commit -m "refactor(rules): update module exports for storage-based rules"
```

---

## Task 12: Refactor AppState for Storage

**Files:**
- Modify: `src/api/routes.rs`

**Step 1: Replace AppState and update handle_decision**

This is the largest change. Replace `ActorPool` with `Arc<dyn Storage>`. The decision flow becomes:

1. Inline rules (unchanged)
2. Upsert subject to storage, get subject_id
3. Evaluate streaming rules with storage
4. Record transaction to storage
5. Record decision to storage
6. Return response

```rust
// New AppState
pub struct AppState {
    /// Storage backend
    pub storage: Arc<dyn Storage>,

    /// Current rule set (updated via watch channel)
    pub ruleset_rx: watch::Receiver<Arc<RuleSet>>,

    /// Application start time
    pub start_time: Instant,

    /// Application version
    pub version: String,

    /// Latency budget in milliseconds
    pub latency_budget_ms: u64,
}
```

The full refactor of `handle_decision` is substantial - it needs to:
- Call `storage.upsert_subject()`
- Loop through streaming rules calling `.evaluate(&event, subject_id, storage.as_ref()).await`
- Call `storage.record_transaction()`
- Call `storage.record_decision()`

**Step 2: Update metrics endpoint**

Remove actor pool stats, add storage health check.

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Errors in main.rs

**Step 4: Commit**

```bash
git add src/api/routes.rs
git commit -m "refactor(api): update routes for Storage backend"
```

---

## Task 13: Update main.rs for PostgreSQL

**Files:**
- Modify: `src/main.rs`

**Step 1: Update startup sequence**

1. Parse config
2. If `database_url` is set, connect to PostgreSQL
3. Optionally run migrations
4. Create AppState with storage
5. Start server

**Step 2: Run cargo check**

Run: `cargo check`
Expected: Compiles

**Step 3: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 4: Commit**

```bash
git add src/main.rs
git commit -m "feat(main): integrate PostgreSQL storage"
```

---

## Task 14: Delete Actor Module

**Files:**
- Delete: `src/actor/mod.rs`
- Delete: `src/actor/pool.rs`
- Delete: `src/actor/state.rs`
- Delete: `src/actor/user.rs`
- Modify: `src/lib.rs`

**Step 1: Remove actor module from lib.rs**

Remove `pub mod actor;` line.

**Step 2: Delete actor directory**

Run: `rm -rf src/actor`

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles

**Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove actor module (replaced by Storage)"
```

---

## Task 15: Delete Old Storage Modules

**Files:**
- Delete: `src/storage/wal.rs`
- Delete: `src/storage/snapshot.rs`
- Delete: `src/storage/recovery.rs`
- Modify: `src/storage/mod.rs`

**Step 1: Update storage/mod.rs**

```rust
// src/storage/mod.rs
pub mod mock;
pub mod postgres;
pub mod traits;

pub use mock::MockStorage;
pub use postgres::PostgresStorage;
pub use traits::{DecisionRecord, Storage, TransactionRecord};
```

**Step 2: Delete old files**

Run: `rm src/storage/wal.rs src/storage/snapshot.rs src/storage/recovery.rs`

**Step 3: Run cargo check**

Run: `cargo check`
Expected: Compiles

**Step 4: Run tests**

Run: `cargo test`
Expected: All tests pass

**Step 5: Commit**

```bash
git add -A
git commit -m "refactor: remove WAL/snapshot storage (replaced by PostgreSQL)"
```

---

## Task 16: Integration Test with Real PostgreSQL

**Files:**
- Create: `tests/integration_postgres.rs`

**Step 1: Write integration test**

```rust
// tests/integration_postgres.rs
//! Integration tests requiring a running PostgreSQL instance.
//!
//! Run with: DATABASE_URL=postgres://... cargo test --test integration_postgres

use riskr::storage::{PostgresStorage, Storage, TransactionRecord};
use rust_decimal::Decimal;
use uuid::Uuid;

async fn setup_storage() -> Option<PostgresStorage> {
    let database_url = std::env::var("DATABASE_URL").ok()?;
    let storage = PostgresStorage::connect(&database_url, 1, 5).await.ok()?;
    storage.run_migrations().await.ok()?;
    Some(storage)
}

#[tokio::test]
async fn test_subject_roundtrip() {
    let Some(storage) = setup_storage().await else {
        eprintln!("Skipping: DATABASE_URL not set");
        return;
    };

    use riskr::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use smallvec::smallvec;

    let subject = Subject {
        user_id: UserId::new(format!("test-{}", Uuid::new_v4())),
        account_id: AccountId::new("A1"),
        addresses: smallvec![Address::new("0xtest123")],
        geo_iso: CountryCode::new("US"),
        kyc_tier: KycTier::L1,
    };

    let id = storage.upsert_subject(&subject).await.unwrap();
    let (retrieved_id, retrieved) = storage
        .get_subject_by_user_id(subject.user_id.as_str())
        .await
        .unwrap()
        .unwrap();

    assert_eq!(id, retrieved_id);
    assert_eq!(retrieved.kyc_tier, KycTier::L1);
}

#[tokio::test]
async fn test_rolling_volume() {
    let Some(storage) = setup_storage().await else {
        eprintln!("Skipping: DATABASE_URL not set");
        return;
    };

    use chrono::Duration;
    use riskr::domain::subject::{AccountId, Address, CountryCode, KycTier, Subject, UserId};
    use smallvec::smallvec;

    // Create subject
    let subject = Subject {
        user_id: UserId::new(format!("vol-test-{}", Uuid::new_v4())),
        account_id: AccountId::new("A1"),
        addresses: smallvec![],
        geo_iso: CountryCode::new("US"),
        kyc_tier: KycTier::L0,
    };
    let subject_id = storage.upsert_subject(&subject).await.unwrap();

    // Record transactions
    for i in 1..=3 {
        let tx = TransactionRecord {
            subject_id,
            tx_type: "withdraw".to_string(),
            asset: "USDC".to_string(),
            amount: Decimal::new(1000 * i, 0),
            usd_value: Decimal::new(1000 * i, 0),
            dest_address: None,
        };
        storage.record_transaction(&tx).await.unwrap();
    }

    // Check rolling volume
    let volume = storage.get_rolling_volume(subject_id, Duration::hours(24)).await.unwrap();
    assert_eq!(volume, Decimal::new(6000, 0)); // 1000 + 2000 + 3000
}
```

**Step 2: Run integration tests**

Run: `docker compose -f docker/docker-compose.yml up -d && DATABASE_URL=postgres://riskr:riskr_dev@localhost:5432/riskr cargo test --test integration_postgres`
Expected: Tests pass (or skip if no DB)

**Step 3: Commit**

```bash
git add tests/integration_postgres.rs
git commit -m "test: add PostgreSQL integration tests"
```

---

## Task 17: Update lib.rs Exports

**Files:**
- Modify: `src/lib.rs`

**Step 1: Update exports**

Ensure `storage` module exports `Storage`, `PostgresStorage`, `MockStorage`.

**Step 2: Run cargo doc**

Run: `cargo doc --no-deps`
Expected: Docs generate without errors

**Step 3: Commit**

```bash
git add src/lib.rs
git commit -m "docs: update lib.rs exports for new storage API"
```

---

## Task 18: Final Verification

**Step 1: Run all tests**

Run: `cargo test`
Expected: All tests pass

**Step 2: Run clippy**

Run: `cargo clippy -- -D warnings`
Expected: No warnings

**Step 3: Build release**

Run: `cargo build --release`
Expected: Builds successfully

**Step 4: Test with local PostgreSQL**

Run:
```bash
docker compose -f docker/docker-compose.yml up -d
RISKR_DATABASE_URL=postgres://riskr:riskr_dev@localhost:5432/riskr \
RISKR_RUN_MIGRATIONS=true \
cargo run --release -- --policy-path policy.yaml
```
Expected: Server starts, connects to PostgreSQL

**Step 5: Commit final state**

```bash
git add -A
git commit -m "feat: complete PostgreSQL storage migration"
```

---

## Summary

This plan migrates from in-memory actors to PostgreSQL storage in 18 tasks:

1. **Tasks 1-3:** Dependencies and infrastructure (sqlx, migrations, Docker)
2. **Tasks 4-6:** Storage trait and implementations (trait, mock, postgres)
3. **Task 7:** Configuration updates
4. **Tasks 8-11:** Streaming rules refactor to async
5. **Tasks 12-13:** API and main.rs refactor
6. **Tasks 14-15:** Delete obsolete code
7. **Tasks 16-18:** Integration tests and verification

Each task is designed to be independently testable and committable.
