# riskr-rs Design Document

## Overview

**riskr-rs** is a high-performance compliance risk decision engine for cryptocurrency transactions. It evaluates transactions against configurable rules to determine whether to allow, hold, review, or reject them.

### Problem Statement

Financial institutions need to screen cryptocurrency transactions against compliance rules in real-time without introducing unacceptable latency. Rules fall into two categories:

1. **Stateless checks** - OFAC sanctions, jurisdiction blocking, per-transaction KYC caps
2. **Stateful checks** - Rolling volume limits, structuring detection (patterns over time)

### Solution

A dual-phase architecture separating fast stateless rules from stateful rules that query rolling windows from PostgreSQL. The service is stateless—all state lives in the database, enabling horizontal scaling.

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                         HTTP API (Axum)                         │
│                    POST /v1/decision/check                      │
└─────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                    Phase 1: Inline Rules                        │
│                      (Stateless, <1ms)                          │
│  ┌──────────┐  ┌─────────────────┐  ┌────────────────────────┐  │
│  │   OFAC   │  │  Jurisdiction   │  │    KYC Tier Cap        │  │
│  │  Screen  │  │     Block       │  │   (per-transaction)    │  │
│  └──────────┘  └─────────────────┘  └────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                                 │
                    (short-circuit if REJECT_FATAL)
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                   Phase 2: Streaming Rules                      │
│                      (Stateful, via Storage)                    │
│  ┌──────────────────────┐  ┌─────────────────────────────────┐  │
│  │   Daily USD Volume   │  │   Structuring Detection         │  │
│  │   (24h rolling)      │  │   (small tx pattern)            │  │
│  └──────────────────────┘  └─────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                     Storage Layer                               │
│                     (PostgreSQL)                                │
│  ┌────────────┐ ┌──────────────┐ ┌───────────┐ ┌─────────────┐ │
│  │  subjects  │ │ transactions │ │ decisions │ │  sanctions  │ │
│  └────────────┘ └──────────────┘ └───────────┘ └─────────────┘ │
└─────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
                         Decision Response
```

---

## Core Components

### Domain Layer (`src/domain/`)

| Module | Purpose |
|--------|---------|
| `decision.rs` | Decision outcomes ordered by severity |
| `policy.rs` | Policy configuration and rule definitions |
| `subject.rs` | User/account/address information |
| `evidence.rs` | Audit trail for triggered rules |
| `event.rs` | Transaction event representation |

**Decision Severity (lowest to highest):**
```
Allow → SoftDenyRetry → HoldAuto → Review → RejectFatal
```

Rules return decisions; the engine takes the maximum severity across all rules.

### Rules Engine (`src/rules/`)

**Inline Rules** (stateless):
- `OfacRule` - Bloom filter + hash set for sanctioned address screening
- `JurisdictionRule` - Block transactions from specified countries
- `KycCapRule` - Per-transaction USD limits by KYC tier

**Streaming Rules** (stateful):
- `DailyVolumeRule` - Rolling 24-hour USD volume cap
- `StructuringRule` - Detects patterns of small transactions

### Storage Layer (`src/storage/`)

The storage layer abstracts persistence behind an async `Storage` trait:

```rust
#[async_trait]
pub trait Storage: Send + Sync {
    async fn upsert_subject(&self, subject: &Subject) -> Result<Uuid>;
    async fn get_rolling_volume(&self, subject_id: Uuid, window: Duration) -> Result<Decimal>;
    async fn get_small_tx_count(&self, subject_id: Uuid, window: Duration, threshold: Decimal) -> Result<u32>;
    async fn record_transaction(&self, tx: &TransactionRecord) -> Result<Uuid>;
    async fn record_decision(&self, decision: &DecisionRecord) -> Result<Uuid>;
    // ...
}
```

**Implementations:**
- `PostgresStorage` - Production storage with connection pooling (sqlx)
- `MockStorage` - In-memory implementation for testing and development

**Database Schema:**
```
subjects          - User/account information
├── id (UUID)
├── user_id (unique)
├── account_id
├── kyc_level
└── geo_iso

transactions      - Transaction history for rolling window queries
├── id (UUID)
├── subject_id (FK)
├── usd_value
├── created_at
└── ...

decisions         - Audit log of all decisions
├── id (UUID)
├── subject_id (FK)
├── decision
├── evidence (JSONB)
└── latency_ms
```

### Policy Management (`src/policy/`)

- `loader.rs` - Loads policy YAML and sanctions lists
- `hot_reload.rs` - Watches files, broadcasts updates via tokio watch channels

Policies reload without restart. Database state is unaffected; only rules change.

---

## Data Flow

### Request Processing

```
1. HTTP POST /v1/decision/check
   └── DecisionRequest JSON

2. Phase 1: Inline Rules
   ├── OFAC check (bloom filter fast path)
   ├── Jurisdiction check
   ├── KYC cap check
   └── Short-circuit if REJECT_FATAL

3. Phase 2: Upsert Subject
   └── storage.upsert_subject() → subject_id

4. Phase 3: Streaming Rules
   ├── Query rolling volume from storage
   ├── Query small tx count from storage
   └── Evaluate against thresholds

5. Phase 4: Record Transaction
   └── storage.record_transaction()

6. Phase 5: Record Decision
   └── storage.record_decision()

7. HTTP Response
   └── DecisionResponse JSON
```

### Rolling Window Queries

Streaming rules query the database for rolling window aggregates:

```sql
-- Daily volume (24h rolling)
SELECT COALESCE(SUM(usd_value), 0)
FROM transactions
WHERE subject_id = $1
  AND created_at > now() - interval '24 hours'

-- Structuring detection (small tx count)
SELECT COUNT(*)
FROM transactions
WHERE subject_id = $1
  AND created_at > now() - interval '24 hours'
  AND usd_value < $threshold
```

Database handles expiration automatically via timestamp filtering.

---

## Key Design Decisions

### Two-Phase Pipeline

Stateless rules run first because:
- Most violations are caught early (sanctions, jurisdiction)
- No database access needed for common rejections
- Reduces load on storage layer

### PostgreSQL for State

Why PostgreSQL over in-memory:
- Service is stateless—can scale horizontally without sticky sessions
- State survives restarts without WAL recovery
- Audit trail of all decisions persisted automatically
- Rolling window queries are efficient with proper indexing

Trade-off: Adds ~1-5ms latency for database queries vs. in-memory lookups.

### Bloom Filter for OFAC

Two-tier lookup:
1. Bloom filter: O(1), may have false positives
2. Hash set: Definitive verification

Most addresses are clean. Bloom filter says "definitely not in set" immediately for the common case.

### Decimal Arithmetic

Uses `rust_decimal::Decimal` for all money values. Prevents floating-point errors in financial calculations.

### Database Rolling Windows

No application-level expiration logic. Database handles it:
- `WHERE created_at > now() - interval '24 hours'` filters expired entries
- Index on `(subject_id, created_at)` makes queries efficient
- Old data can be archived/deleted via scheduled jobs

---

## Configuration

### CLI Arguments

```
riskr [OPTIONS]

--listen-addr <ADDR>          Listen address (default: 0.0.0.0:8080)
--policy-path <PATH>          Policy YAML file (default: policy.yaml)
--sanctions-path <PATH>       Sanctions list (default: sanctions.txt)
--database-url <URL>          PostgreSQL connection string (optional)
--db-pool-min <N>             Min pool connections (default: 2)
--db-pool-max <N>             Max pool connections (default: 10)
--run-migrations              Run migrations on startup (default: false)
--policy-reload-secs <SECS>   Reload interval (default: 30)
--latency-budget-ms <MS>      Latency warning threshold (default: 100)
```

All arguments support environment variables with `RISKR_` prefix.

Without `--database-url`, the service uses an in-memory mock storage (useful for development/testing).

### Policy Format

```yaml
policy_version: "v1.0.0"

params:
  daily_volume_limit_usd: 50000
  structuring_small_usd: 2000
  structuring_small_count: 5
  kyc_tier_caps_usd:
    L0: 100
    L1: 1000
    L2: 10000

rules:
  - id: R1_OFAC
    type: ofac_addr
    action: REJECT_FATAL

  - id: R2_JURISDICTION
    type: jurisdiction_block
    action: REJECT_FATAL
    blocked_countries: ["IR", "KP", "CU", "SY", "RU"]

  - id: R3_KYC_CAP
    type: kyc_tier_tx_cap
    action: HOLD_AUTO

  - id: R4_DAILY_VOLUME
    type: daily_usd_volume
    action: HOLD_AUTO

  - id: R5_STRUCTURING
    type: structuring_small_tx
    action: REVIEW
```

---

## API Reference

### POST /v1/decision/check

**Request:**
```json
{
  "subject": {
    "user_id": "U123",
    "account_id": "A456",
    "addresses": ["0xabc123"],
    "geo_iso": "US",
    "kyc_level": "L2"
  },
  "tx": {
    "type": "withdraw",
    "asset": "USDC",
    "amount": "500000000",
    "usd_value": 500.00,
    "dest_address": "0xdef456"
  }
}
```

**Response:**
```json
{
  "decision": "ALLOW",
  "decision_code": "OK",
  "policy_version": "v1.0.0",
  "evidence": []
}
```

**Response with triggered rule:**
```json
{
  "decision": "HOLD_AUTO",
  "decision_code": "DAILY_VOLUME_EXCEEDED",
  "policy_version": "v1.0.0",
  "evidence": [
    {
      "rule_id": "R4_DAILY_VOLUME",
      "key": "rolling_24h_usd",
      "value": "52500.00",
      "limit": "50000"
    }
  ]
}
```

### GET /health

```json
{
  "status": "healthy",
  "version": "0.1.0",
  "policy_version": "v1.0.0",
  "uptime_secs": 3600
}
```

### GET /ready

```json
{
  "ready": true,
  "inline_rules": 3,
  "streaming_rules": 2
}
```

### GET /metrics

Prometheus text format with:
- `riskr_uptime_seconds` - Server uptime
- `riskr_inline_rules` - Number of inline rules loaded
- `riskr_streaming_rules` - Number of streaming rules loaded

---

## Extending the Engine

### Adding a New Inline Rule

1. Define rule type in `domain/policy.rs`:
   ```rust
   pub enum RuleType {
       // ...existing types
       MyNewRule,
   }
   ```

2. Implement in `rules/inline/my_rule.rs`:
   ```rust
   pub struct MyRule {
       id: String,
       action: Decision,
   }

   impl InlineRule for MyRule {
       fn id(&self) -> &str { &self.id }

       fn evaluate(&self, event: &TxEvent) -> RuleResult {
           // Rule logic here
       }
   }
   ```

3. Register in `rules/mod.rs`:
   ```rust
   RuleType::MyNewRule => {
       inline.push(Arc::new(MyRule::new(def)));
   }
   ```

### Adding a New Streaming Rule

Same pattern, but implement async `StreamingRule` trait:
```rust
#[async_trait]
impl StreamingRule for MyStreamingRule {
    fn id(&self) -> &str { &self.id }

    async fn evaluate(
        &self,
        event: &TxEvent,
        subject_id: Uuid,
        storage: &dyn Storage,
    ) -> Result<RuleResult> {
        // Query storage for historical data
        let volume = storage.get_rolling_volume(subject_id, self.window).await?;
        // Evaluate against thresholds
    }
}
```

---

## Performance

| Operation | Typical Latency |
|-----------|-----------------|
| Inline rule evaluation | <1ms |
| OFAC bloom filter check | <1µs |
| Database: upsert subject | ~1-2ms |
| Database: rolling volume query | ~1-3ms |
| Database: record transaction | ~1-2ms |
| **Total decision** | **<10-15ms typical** |

**Note:** Database latency depends on network proximity and connection pooling. Co-located PostgreSQL with proper indexing achieves the lower bounds.

---

## Deployment

### Resource Requirements

- **Memory:** ~100MB baseline (stateless service)
- **CPU:** Scales linearly with cores
- **Database:** PostgreSQL 14+ with proper indexing

### Scaling

The service is stateless. All state lives in PostgreSQL.

| Scaling | Approach |
|---------|----------|
| **Horizontal** | Add more instances behind a load balancer |
| **Vertical** | Increase database connection pool size |
| **Database** | Read replicas for query scaling, partitioning for write scaling |

No sticky sessions required. Any instance can handle any request.

### Health Checks

- `/health` - Liveness probe
- `/ready` - Readiness probe (rules loaded)
- `/metrics` - Prometheus scraping

### Database Indexes

Recommended indexes for performance:
```sql
CREATE INDEX idx_transactions_subject_created
    ON transactions(subject_id, created_at DESC);

CREATE INDEX idx_subjects_user_id
    ON subjects(user_id);
```
