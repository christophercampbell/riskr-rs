// src/storage/postgres.rs
use async_trait::async_trait;
use chrono::Duration;
use rust_decimal::Decimal;
use sqlx::postgres::PgPoolOptions;
use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::domain::subject::{AccountId, Address, CountryCode, KycTier, UserId};
use crate::domain::{Policy, Subject};

use super::traits::{DecisionRecord, Storage, TransactionRecord};

/// PostgreSQL implementation of the Storage trait.
pub struct PostgresStorage {
    pool: PgPool,
}

impl PostgresStorage {
    /// Create a new PostgresStorage instance with a connection pool.
    pub async fn connect(
        database_url: &str,
        min_connections: u32,
        max_connections: u32,
    ) -> anyhow::Result<Self> {
        let pool = PgPoolOptions::new()
            .min_connections(min_connections)
            .max_connections(max_connections)
            .connect(database_url)
            .await?;

        Ok(Self { pool })
    }

    /// Run database migrations.
    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        sqlx::migrate!("./migrations").run(&self.pool).await?;
        Ok(())
    }

    /// Get a reference to the connection pool.
    pub fn pool(&self) -> &PgPool {
        &self.pool
    }
}

#[async_trait]
impl Storage for PostgresStorage {
    async fn get_subject_by_user_id(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Option<(Uuid, Subject)>> {
        let row = sqlx::query(
            r#"
            SELECT id, user_id, account_id, kyc_level, geo_iso
            FROM subjects
            WHERE user_id = $1
            "#,
        )
        .bind(user_id)
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let subject_id: Uuid = row.get("id");
        let user_id: String = row.get("user_id");
        let account_id: String = row.get("account_id");
        let kyc_level: String = row.get("kyc_level");
        let geo_iso: String = row.get("geo_iso");

        // Fetch addresses for this subject
        let addresses = sqlx::query(
            r#"
            SELECT address
            FROM subject_addresses
            WHERE subject_id = $1
            "#,
        )
        .bind(subject_id)
        .fetch_all(&self.pool)
        .await?
        .into_iter()
        .map(|row| {
            let addr: String = row.get("address");
            Address::new(addr)
        })
        .collect();

        let subject = Subject {
            user_id: UserId::new(user_id),
            account_id: AccountId::new(account_id),
            addresses,
            geo_iso: CountryCode::new(geo_iso),
            kyc_tier: KycTier::from_str(&kyc_level).unwrap_or_default(),
        };

        Ok(Some((subject_id, subject)))
    }

    async fn upsert_subject(&self, subject: &Subject) -> anyhow::Result<Uuid> {
        // Upsert the subject record
        let subject_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO subjects (user_id, account_id, kyc_level, geo_iso, updated_at)
            VALUES ($1, $2, $3, $4, now())
            ON CONFLICT (user_id)
            DO UPDATE SET
                account_id = EXCLUDED.account_id,
                kyc_level = EXCLUDED.kyc_level,
                geo_iso = EXCLUDED.geo_iso,
                updated_at = now()
            RETURNING id
            "#,
        )
        .bind(subject.user_id.as_str())
        .bind(&subject.account_id.0)
        .bind(subject.kyc_tier.as_str())
        .bind(subject.geo_iso.as_str())
        .fetch_one(&self.pool)
        .await?;

        // Upsert addresses
        for address in &subject.addresses {
            sqlx::query(
                r#"
                INSERT INTO subject_addresses (subject_id, address)
                VALUES ($1, $2)
                ON CONFLICT (subject_id, address) DO NOTHING
                "#,
            )
            .bind(subject_id)
            .bind(address.as_str())
            .execute(&self.pool)
            .await?;
        }

        Ok(subject_id)
    }

    async fn record_transaction(&self, tx: &TransactionRecord) -> anyhow::Result<Uuid> {
        let tx_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO transactions (subject_id, tx_type, asset, amount, usd_value, dest_address)
            VALUES ($1, $2, $3, $4, $5, $6)
            RETURNING id
            "#,
        )
        .bind(tx.subject_id)
        .bind(&tx.tx_type)
        .bind(&tx.asset)
        .bind(tx.amount)
        .bind(tx.usd_value)
        .bind(&tx.dest_address)
        .fetch_one(&self.pool)
        .await?;

        Ok(tx_id)
    }

    async fn get_rolling_volume(
        &self,
        subject_id: Uuid,
        window: Duration,
    ) -> anyhow::Result<Decimal> {
        let window_secs = window.num_seconds();

        let volume: Option<Decimal> = sqlx::query_scalar(
            r#"
            SELECT COALESCE(SUM(usd_value), 0)
            FROM transactions
            WHERE subject_id = $1
              AND created_at > now() - ($2 || ' seconds')::interval
            "#,
        )
        .bind(subject_id)
        .bind(window_secs.to_string())
        .fetch_one(&self.pool)
        .await?;

        Ok(volume.unwrap_or(Decimal::ZERO))
    }

    async fn get_small_tx_count(
        &self,
        subject_id: Uuid,
        window: Duration,
        threshold: Decimal,
    ) -> anyhow::Result<u32> {
        let window_secs = window.num_seconds();

        let count: i64 = sqlx::query_scalar(
            r#"
            SELECT COUNT(*)
            FROM transactions
            WHERE subject_id = $1
              AND created_at > now() - ($2 || ' seconds')::interval
              AND usd_value < $3
            "#,
        )
        .bind(subject_id)
        .bind(window_secs.to_string())
        .bind(threshold)
        .fetch_one(&self.pool)
        .await?;

        Ok(count as u32)
    }

    async fn get_all_sanctions(&self) -> anyhow::Result<Vec<String>> {
        let addresses = sqlx::query_scalar(
            r#"
            SELECT address
            FROM sanctions
            "#,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(addresses)
    }

    async fn is_sanctioned(&self, address: &str) -> anyhow::Result<bool> {
        let exists: bool = sqlx::query_scalar(
            r#"
            SELECT EXISTS(
                SELECT 1
                FROM sanctions
                WHERE LOWER(address) = LOWER($1)
            )
            "#,
        )
        .bind(address)
        .fetch_one(&self.pool)
        .await?;

        Ok(exists)
    }

    async fn get_active_policy(&self) -> anyhow::Result<Option<Policy>> {
        let row = sqlx::query(
            r#"
            SELECT config
            FROM policies
            WHERE active = true
            LIMIT 1
            "#,
        )
        .fetch_optional(&self.pool)
        .await?;

        let Some(row) = row else {
            return Ok(None);
        };

        let config: serde_json::Value = row.get("config");
        let policy: Policy = serde_json::from_value(config)?;

        Ok(Some(policy))
    }

    async fn set_active_policy(&self, policy: &Policy) -> anyhow::Result<()> {
        // Start a transaction
        let mut tx = self.pool.begin().await?;

        // Deactivate all existing policies
        sqlx::query(
            r#"
            UPDATE policies
            SET active = false
            WHERE active = true
            "#,
        )
        .execute(&mut *tx)
        .await?;

        // Insert or update the new policy
        let config = serde_json::to_value(policy)?;

        sqlx::query(
            r#"
            INSERT INTO policies (version, config, active)
            VALUES ($1, $2, true)
            ON CONFLICT (version)
            DO UPDATE SET
                config = EXCLUDED.config,
                active = true
            "#,
        )
        .bind(&policy.version)
        .bind(config)
        .execute(&mut *tx)
        .await?;

        // Commit the transaction
        tx.commit().await?;

        Ok(())
    }

    async fn record_decision(&self, decision: &DecisionRecord) -> anyhow::Result<Uuid> {
        let evidence = serde_json::to_value(&decision.evidence)?;

        let decision_id: Uuid = sqlx::query_scalar(
            r#"
            INSERT INTO decisions (
                subject_id,
                request,
                decision,
                decision_code,
                policy_version,
                evidence,
                latency_ms
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7)
            RETURNING id
            "#,
        )
        .bind(decision.subject_id)
        .bind(&decision.request)
        .bind(format!("{:?}", decision.decision))
        .bind(&decision.decision_code)
        .bind(&decision.policy_version)
        .bind(evidence)
        .bind(decision.latency_ms as i32)
        .fetch_one(&self.pool)
        .await?;

        Ok(decision_id)
    }
}
