# PostgreSQL Storage Refactor Design

## Overview

Replace the in-memory actor model with PostgreSQL-backed storage. This enables horizontal scaling without sticky sessions—any instance can serve any request.

## Architecture

```
                    ┌─────────────────┐
                    │  Load Balancer  │
                    └────────┬────────┘
           ┌─────────────────┼─────────────────┐
           ▼                 ▼                 ▼
    ┌─────────────┐   ┌─────────────┐   ┌─────────────┐
    │  riskr (1)  │   │  riskr (2)  │   │  riskr (N)  │
    │  stateless  │   │  stateless  │   │  stateless  │
    └──────┬──────┘   └──────┬──────┘   └──────┬──────┘
           │                 │                 │
           └─────────────────┼─────────────────┘
                             ▼
              ┌──────────────────────────┐
              │       PostgreSQL         │
              │  (+ read replicas opt.)  │
              └──────────────────────────┘
```

**Unchanged:**
- Two-phase pipeline (inline rules, then streaming rules)
- Bloom filter for OFAC screening (loaded from DB into memory)
- Hot reload of policies (poll DB)
- HTTP API contract

**Changed:**
- User state lives in PostgreSQL, not in-memory actors
- Streaming rules query the `transactions` table
- No WAL/snapshot—PostgreSQL handles durability
- Connection pool (sqlx) replaces actor pool

## Storage Trait

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    // Subjects
    async fn get_subject_by_user_id(&self, user_id: &str) -> Result<Option<Subject>>;
    async fn upsert_subject(&self, subject: &Subject) -> Result<Uuid>;

    // Transactions (for streaming rules)
    async fn record_transaction(&self, tx: &TransactionRecord) -> Result<Uuid>;
    async fn get_rolling_volume(&self, subject_id: Uuid, window: Duration) -> Result<Decimal>;
    async fn get_small_tx_count(&self, subject_id: Uuid, window: Duration, threshold: Decimal) -> Result<u32>;

    // Sanctions
    async fn get_all_sanctions(&self) -> Result<Vec<String>>;
    async fn is_sanctioned(&self, address: &str) -> Result<bool>;

    // Policies
    async fn get_active_policy(&self) -> Result<Option<Policy>>;
    async fn set_active_policy(&self, policy: &Policy) -> Result<()>;

    // Decisions (audit log)
    async fn record_decision(&self, decision: &DecisionRecord) -> Result<Uuid>;
}
```

**Implementations:**
- `PostgresStorage` - Real implementation using sqlx connection pool
- `MockStorage` - In-memory HashMap-based implementation for unit tests

## Database Schema

```sql
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

## Project Structure

```
src/
├── storage/
│   ├── mod.rs           # Storage trait definition
│   ├── postgres.rs      # PostgresStorage implementation
│   └── mock.rs          # MockStorage for tests
├── rules/
│   ├── inline/          # (unchanged - stateless rules)
│   └── streaming/
│       ├── mod.rs       # Updated to use Storage trait
│       ├── daily_volume.rs
│       └── structuring.rs
├── domain/              # (unchanged)
├── api/                 # (unchanged)
├── policy/
│   ├── mod.rs
│   └── loader.rs        # Updated to load from DB
├── config/              # Add database_url
├── observability/       # (unchanged)
├── lib.rs
└── main.rs

migrations/
├── 0001_initial_schema.sql

docker/
└── docker-compose.yml   # PostgreSQL for local dev
```

**Deleted:**
- `src/actor/` (entire directory)
- `src/storage/wal.rs`
- `src/storage/snapshot.rs`
- `src/storage/recovery.rs`

**Dependencies:**

Added:
```toml
sqlx = { version = "0.8", features = ["runtime-tokio", "postgres", "uuid", "chrono", "rust_decimal"] }
```

Removed:
- `crc32fast`
- `memmap2`

## Streaming Rules

Rules become stateless—they evaluate against storage.

```rust
impl DailyVolumeRule {
    pub async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> Result<RuleResult> {
        let current_volume = storage
            .get_rolling_volume(subject_id, Duration::hours(24))
            .await?;

        let new_total = current_volume + event.usd_value;

        if new_total > self.limit {
            Ok(RuleResult::triggered(
                self.id.clone(),
                self.action,
                Evidence::new("rolling_24h_usd", new_total, self.limit),
            ))
        } else {
            Ok(RuleResult::allow())
        }
    }
}
```

**Decision flow:**
1. Inline rules (stateless, unchanged)
2. Lookup or create subject in DB
3. Evaluate streaming rules against DB
4. Record transaction to DB
5. Record decision to audit log
6. Return response

## Configuration

**New CLI arguments:**
```
--database-url <URL>     PostgreSQL connection string (required)
                         env: RISKR_DATABASE_URL
--db-pool-min <N>        Min connections (default: 2)
--db-pool-max <N>        Max connections (default: 10)
--run-migrations         Run migrations on startup (default: false)
```

**Removed CLI arguments:**
- `--wal-path`
- `--snapshot-path`
- `--actor-idle-secs`

**Startup sequence:**
1. Parse config
2. Connect to PostgreSQL, create connection pool
3. Optionally run migrations (`--run-migrations`)
4. Load active policy from DB
5. Load sanctions into bloom filter (refresh periodically)
6. Start HTTP server

**Policy hot reload:**
- Poll `policies` table every N seconds for active policy changes

**Sanctions refresh:**
- Load full sanctions list into bloom filter + hash set on startup
- Refresh every 60 seconds (configurable)

## Testing

Unit tests use `MockStorage`:

```rust
#[tokio::test]
async fn daily_volume_triggers_when_limit_exceeded() {
    let mut mock = MockStorage::new();
    mock.set_rolling_volume(subject_id, dec!(45000));

    let rule = DailyVolumeRule::new("R1", Decision::HoldAuto, dec!(50000));
    let event = TxEvent { usd_value: dec!(6000), .. };

    let result = rule.evaluate(&event, subject_id, &mock).await.unwrap();

    assert_eq!(result.decision, Decision::HoldAuto);
}
```

**MockStorage:**
- HashMap-based, stores preset return values
- Methods like `set_rolling_volume()`, `set_small_tx_count()`, `add_sanction()`
- Tracks calls for assertion

**Coverage:**
- Each rule in isolation with mock
- Decision engine flow with mock
- API handlers with mock storage injected
