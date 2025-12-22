use std::path::PathBuf;
use std::time::Duration;

use clap::Parser;

/// Risk engine configuration.
#[derive(Debug, Clone, Parser)]
#[command(name = "riskr")]
#[command(about = "High-performance risk decision engine")]
pub struct Config {
    /// HTTP server listen address
    #[arg(long, default_value = "0.0.0.0:8080", env = "RISKR_LISTEN_ADDR")]
    pub listen_addr: String,

    /// Path to policy YAML file
    #[arg(long, default_value = "policy.yaml", env = "RISKR_POLICY_PATH")]
    pub policy_path: PathBuf,

    /// Path to sanctions list file
    #[arg(long, default_value = "sanctions.txt", env = "RISKR_SANCTIONS_PATH")]
    pub sanctions_path: PathBuf,

    /// Path to WAL directory (optional, disables WAL if not set)
    #[arg(long, env = "RISKR_WAL_PATH")]
    pub wal_path: Option<PathBuf>,

    /// Path to snapshot directory (optional)
    #[arg(long, env = "RISKR_SNAPSHOT_PATH")]
    pub snapshot_path: Option<PathBuf>,

    /// Policy reload check interval in seconds
    #[arg(long, default_value = "30", env = "RISKR_POLICY_RELOAD_SECS")]
    pub policy_reload_secs: u64,

    /// Latency budget in milliseconds for decision endpoint
    #[arg(long, default_value = "100", env = "RISKR_LATENCY_BUDGET_MS")]
    pub latency_budget_ms: u64,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, default_value = "info", env = "RUST_LOG")]
    pub log_level: String,

    /// Maximum entries per user state (for memory bounds)
    #[arg(long, default_value = "1000", env = "RISKR_MAX_ENTRIES_PER_USER")]
    pub max_entries_per_user: usize,

    /// Actor pool stripe count for lock contention reduction (power of 2 recommended)
    #[arg(long, default_value = "64", env = "RISKR_STRIPE_COUNT")]
    pub stripe_count: usize,

    /// Idle actor eviction timeout in seconds
    #[arg(long, default_value = "3600", env = "RISKR_ACTOR_IDLE_SECS")]
    pub actor_idle_secs: u64,

    /// Enable graceful shutdown
    #[arg(long, default_value = "true", env = "RISKR_GRACEFUL_SHUTDOWN")]
    pub graceful_shutdown: bool,

    /// Graceful shutdown timeout in seconds
    #[arg(long, default_value = "30", env = "RISKR_SHUTDOWN_TIMEOUT_SECS")]
    pub shutdown_timeout_secs: u64,
}

impl Config {
    /// Get policy reload interval as Duration.
    pub fn policy_reload_interval(&self) -> Duration {
        Duration::from_secs(self.policy_reload_secs)
    }

    /// Get shutdown timeout as Duration.
    pub fn shutdown_timeout(&self) -> Duration {
        Duration::from_secs(self.shutdown_timeout_secs)
    }

    /// Get actor idle timeout as Duration.
    pub fn actor_idle_timeout(&self) -> Duration {
        Duration::from_secs(self.actor_idle_secs)
    }
}

impl Default for Config {
    fn default() -> Self {
        Config {
            listen_addr: "0.0.0.0:8080".to_string(),
            policy_path: PathBuf::from("policy.yaml"),
            sanctions_path: PathBuf::from("sanctions.txt"),
            wal_path: None,
            snapshot_path: None,
            policy_reload_secs: 30,
            latency_budget_ms: 100,
            log_level: "info".to_string(),
            max_entries_per_user: 1000,
            stripe_count: 64,
            actor_idle_secs: 3600,
            graceful_shutdown: true,
            shutdown_timeout_secs: 30,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();

        assert_eq!(config.listen_addr, "0.0.0.0:8080");
        assert_eq!(config.latency_budget_ms, 100);
        assert_eq!(config.stripe_count, 64);
    }

    #[test]
    fn test_duration_helpers() {
        let config = Config {
            policy_reload_secs: 60,
            shutdown_timeout_secs: 15,
            actor_idle_secs: 1800,
            ..Default::default()
        };

        assert_eq!(config.policy_reload_interval(), Duration::from_secs(60));
        assert_eq!(config.shutdown_timeout(), Duration::from_secs(15));
        assert_eq!(config.actor_idle_timeout(), Duration::from_secs(1800));
    }
}
