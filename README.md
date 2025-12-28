# riskr-rs

High-performance risk decision engine for cryptocurrency transactions.

## Overview

riskr-rs evaluates transactions against configurable compliance rules in two phases:

1. **Inline rules** (stateless, <1ms) - OFAC sanctions, jurisdiction blocks, KYC caps
2. **Streaming rules** (stateful) - Daily volume limits, structuring detection

Decisions follow severity ordering: `Allow < SoftDenyRetry < HoldAuto < Review < RejectFatal`

State is persisted to PostgreSQL, making the service stateless and horizontally scalable.

## Quick Start

```bash
# Build
cargo build --release

# Run with PostgreSQL
./target/release/riskr \
  --database-url postgres://user:pass@localhost/riskr \
  --run-migrations

# Run without database (in-memory mock storage)
./target/release/riskr

# Full configuration
./target/release/riskr \
  --listen-addr 0.0.0.0:8080 \
  --policy-path policy.yaml \
  --sanctions-path sanctions.txt \
  --database-url postgres://localhost/riskr
```

## API

### POST /v1/decision/check

Evaluate a transaction:

```bash
curl -X POST http://localhost:8080/v1/decision/check \
  -H "Content-Type: application/json" \
  -d '{
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
  }'
```

Response:

```json
{
  "decision": "ALLOW",
  "decision_code": "OK",
  "policy_version": "v1.0.0",
  "evidence": []
}
```

When a rule triggers:

```json
{
  "decision": "REJECT_FATAL",
  "decision_code": "R1_OFAC",
  "policy_version": "v1.0.0",
  "evidence": [
    {
      "rule_id": "R1_OFAC",
      "key": "address",
      "value": "0xdeadbeef..."
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
  "policy_version": "v1.0.0",
  "inline_rules": 3,
  "streaming_rules": 2
}
```

### GET /metrics

Prometheus format metrics.

## Configuration

All options available via CLI flags or environment variables:

| Flag | Env | Default | Description |
|------|-----|---------|-------------|
| `--listen-addr` | `RISKR_LISTEN_ADDR` | `0.0.0.0:8080` | HTTP listen address |
| `--policy-path` | `RISKR_POLICY_PATH` | `policy.yaml` | Policy file path |
| `--sanctions-path` | `RISKR_SANCTIONS_PATH` | `sanctions.txt` | Sanctions list path |
| `--database-url` | `RISKR_DATABASE_URL` | (disabled) | PostgreSQL connection string |
| `--db-pool-min` | `RISKR_DB_POOL_MIN` | `2` | Min database connections |
| `--db-pool-max` | `RISKR_DB_POOL_MAX` | `10` | Max database connections |
| `--run-migrations` | `RISKR_RUN_MIGRATIONS` | `false` | Run migrations on startup |
| `--policy-reload-secs` | `RISKR_POLICY_RELOAD_SECS` | `30` | Policy check interval |
| `--latency-budget-ms` | `RISKR_LATENCY_BUDGET_MS` | `100` | Latency warning threshold |
| `--log-level` | `RUST_LOG` | `info` | Log level |

## Policy Format

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
    L3: 100000

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

## Rule Types

| Type | Phase | Description |
|------|-------|-------------|
| `ofac_addr` | Inline | Block sanctioned addresses |
| `jurisdiction_block` | Inline | Block transactions from specified countries |
| `kyc_tier_tx_cap` | Inline | Enforce per-transaction limits by KYC tier |
| `daily_usd_volume` | Streaming | Limit 24-hour rolling volume |
| `structuring_small_tx` | Streaming | Detect structuring patterns |

## Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      HTTP API (axum)                        │
│                   POST /v1/decision/check                   │
└─────────────────────────┬───────────────────────────────────┘
                          │
          ┌───────────────┴───────────────┐
          ▼                               ▼
┌─────────────────────┐       ┌─────────────────────┐
│   Inline Rules      │       │  Streaming Rules    │
│   (stateless)       │       │   (stateful)        │
│                     │       │                     │
│ • OFAC              │       │ • Daily Volume      │
│ • Jurisdiction      │       │ • Structuring       │
│ • KYC Cap           │       │                     │
└─────────────────────┘       └──────────┬──────────┘
                                         │
                              ┌──────────▼──────────┐
                              │   Storage Layer     │
                              │   (PostgreSQL)      │
                              │                     │
                              │ • subjects          │
                              │ • transactions      │
                              │ • decisions         │
                              │ • sanctions         │
                              └─────────────────────┘
```

The service is stateless—all state lives in PostgreSQL. This enables horizontal scaling without sticky sessions.

## Development

```bash
# Run tests
cargo test

# Run benchmarks
cargo bench

# Build release
cargo build --release
```

## License

MIT
