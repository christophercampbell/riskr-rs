# riskr-rs Design Document

## Overview

**riskr-rs** is a high-performance compliance risk decision engine for cryptocurrency transactions. It evaluates transactions against configurable rules to determine whether to allow, hold, review, or reject them.

### Problem Statement

Financial institutions need to screen cryptocurrency transactions against compliance rules in real-time without introducing unacceptable latency. Rules fall into two categories:

1. **Stateless checks** - OFAC sanctions, jurisdiction blocking, per-transaction KYC caps
2. **Stateful checks** - Rolling volume limits, structuring detection (patterns over time)

### Solution

A dual-phase architecture separating fast stateless rules from stateful rules that maintain per-user rolling windows. Uses sharded actor pools and bloom filters to achieve sub-100ms decision latency at scale.

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
│                      (Stateful, via Actor Pool)                 │
│  ┌──────────────────────┐  ┌─────────────────────────────────┐  │
│  │   Daily USD Volume   │  │   Structuring Detection         │  │
│  │   (24h rolling)      │  │   (small tx pattern)            │  │
│  └──────────────────────┘  └─────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────┘
                                 │
                                 ▼
┌─────────────────────────────────────────────────────────────────┐
│                        Actor Pool                               │
│                      (64 shards)                                │
│  ┌─────────┐ ┌─────────┐ ┌─────────┐         ┌─────────┐       │
│  │ Shard 0 │ │ Shard 1 │ │ Shard 2 │   ...   │Shard 63 │       │
│  │ Users   │ │ Users   │ │ Users   │         │ Users   │       │
│  └─────────┘ └─────────┘ └─────────┘         └─────────┘       │
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

### Actor System (`src/actor/`)

Each user has one `UserActor` managing their transaction history:

```
ActorPool
├── 64 shards (hash-distributed by user_id)
│   └── RwLock<HashMap<user_id, Arc<Mutex<UserActor>>>>
│
UserActor
├── UserState (rolling 24h window, max 10k entries)
└── streaming_rules reference
```

**Concurrency model:**
- Read lock for actor lookup (fast path)
- Write lock only when creating new actor
- Per-actor mutex for state mutations
- Minimal cross-user contention

### Policy Management (`src/policy/`)

- `loader.rs` - Loads policy YAML and sanctions lists
- `hot_reload.rs` - Watches files, broadcasts updates via tokio watch channels

Policies reload without restart. Actor state is preserved; only rules change.

### Storage Layer (`src/storage/`)

- `wal.rs` - Write-ahead log for durability (optional)
- `snapshot.rs` - Full state snapshots for recovery
- `recovery.rs` - State reconstruction on startup

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

3. Phase 2: Streaming Rules
   ├── Hash user_id → shard index
   ├── Get or create UserActor
   ├── Lock actor, prune expired entries
   ├── Evaluate daily volume
   ├── Evaluate structuring
   └── Update state with new transaction

4. Combine Results
   └── Take maximum severity decision

5. Optional: Write to WAL

6. HTTP Response
   └── DecisionResponse JSON
```

### Actor State Management

```
UserState
├── user_id: String
├── entries: VecDeque<TxEntry>  // bounded, 10k max
│   └── TxEntry { timestamp, usd_value }
└── last_access: DateTime       // for idle eviction

On each evaluation:
1. Prune entries older than 24h (pop_front)
2. Query rolling totals
3. Push new entry (push_back)
4. Enforce capacity limit
```

---

## Key Design Decisions

### Two-Phase Pipeline

Stateless rules run first because:
- Most violations are caught early (sanctions, jurisdiction)
- No state access needed for common rejections
- Reduces load on actor pool

### 64-Shard Actor Pool

Why 64 shards:
- Balances memory overhead vs. contention
- Hash distribution spreads load evenly
- Matches typical server core counts

### Bloom Filter for OFAC

Two-tier lookup:
1. Bloom filter: O(1), may have false positives
2. Hash set: Definitive verification

Most addresses are clean. Bloom filter says "definitely not in set" immediately for the common case.

### Decimal Arithmetic

Uses `rust_decimal::Decimal` for all money values. Prevents floating-point errors in financial calculations.

### Rolling Window with Lazy Pruning

No per-entry expiration timers. On each access:
- Prune expired entries from front of deque
- Bounded memory per user
- Amortized O(1) operations

---

## Configuration

### CLI Arguments

```
riskr [OPTIONS]

--listen-addr <ADDR>          Listen address (default: 0.0.0.0:8080)
--policy-path <PATH>          Policy YAML file (default: policy.yaml)
--sanctions-path <PATH>       Sanctions list (default: sanctions.txt)
--wal-path <PATH>             WAL directory (optional)
--snapshot-path <PATH>        Snapshot directory (optional)
--policy-reload-secs <SECS>   Reload interval (default: 30)
--latency-budget-ms <MS>      Latency warning threshold (default: 100)
--actor-idle-secs <SECS>      Idle eviction timeout (default: 3600)
```

All arguments support environment variables with `RISKR_` prefix.

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
- `riskr_actors_total` - Active user actors
- `riskr_entries_total` - Transaction entries in memory
- `riskr_uptime_seconds` - Server uptime
- `riskr_decisions_total{decision="..."}` - Decision counts

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

Same pattern, but implement `StreamingRule` trait:
```rust
impl StreamingRule for MyStreamingRule {
    fn id(&self) -> &str { &self.id }

    fn evaluate(&self, event: &TxEvent, state: &UserState) -> RuleResult {
        // Access state.entries for historical data
    }
}
```

---

## Performance

| Operation | Typical Latency |
|-----------|-----------------|
| Inline rule evaluation | <1ms |
| Actor lookup (hot) | <100µs |
| Actor creation (cold) | ~1ms |
| State query (volume/structuring) | <10µs |
| OFAC bloom filter check | <1µs |
| **Total decision** | **<10ms typical** |

**Memory per user:** ~50KB (256 entries max)

**Throughput:** ~1000 decisions/sec per core

---

## Deployment

### Resource Requirements

- **Memory:** 500MB baseline + 50KB per concurrent user
- **CPU:** Scales linearly with cores
- **Storage:** Optional WAL/snapshots

### Scaling

- Stateless design allows horizontal scaling
- Each instance maintains independent actor pool
- No inter-instance communication required
- Shared policy/sanctions lists (read-only)

### Health Checks

- `/health` - Liveness probe
- `/ready` - Readiness probe (rules loaded)
- `/metrics` - Prometheus scraping
