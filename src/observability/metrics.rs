use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Instant;

/// Metrics registry for the application.
#[derive(Debug, Default)]
pub struct MetricsRegistry {
    /// Total decision requests processed
    pub decisions_total: AtomicU64,

    /// Decision requests by outcome
    pub decisions_allow: AtomicU64,
    pub decisions_soft_deny: AtomicU64,
    pub decisions_hold: AtomicU64,
    pub decisions_review: AtomicU64,
    pub decisions_reject: AtomicU64,

    /// Decision latency buckets (microseconds)
    pub latency_under_1ms: AtomicU64,
    pub latency_1_5ms: AtomicU64,
    pub latency_5_10ms: AtomicU64,
    pub latency_10_50ms: AtomicU64,
    pub latency_50_100ms: AtomicU64,
    pub latency_over_100ms: AtomicU64,

    /// Rule evaluation counts
    pub rules_evaluated_total: AtomicU64,
    pub rules_triggered_total: AtomicU64,

    /// WAL operations
    pub wal_writes_total: AtomicU64,
    pub wal_write_errors: AtomicU64,

    /// Policy reloads
    pub policy_reloads_total: AtomicU64,
    pub policy_reload_errors: AtomicU64,
}

impl MetricsRegistry {
    /// Create a new metrics registry.
    pub fn new() -> Self {
        MetricsRegistry::default()
    }

    /// Record a decision outcome.
    pub fn record_decision(&self, decision: &crate::domain::Decision) {
        self.decisions_total.fetch_add(1, Ordering::Relaxed);

        match decision {
            crate::domain::Decision::Allow => {
                self.decisions_allow.fetch_add(1, Ordering::Relaxed);
            }
            crate::domain::Decision::SoftDenyRetry => {
                self.decisions_soft_deny.fetch_add(1, Ordering::Relaxed);
            }
            crate::domain::Decision::HoldAuto => {
                self.decisions_hold.fetch_add(1, Ordering::Relaxed);
            }
            crate::domain::Decision::Review => {
                self.decisions_review.fetch_add(1, Ordering::Relaxed);
            }
            crate::domain::Decision::RejectFatal => {
                self.decisions_reject.fetch_add(1, Ordering::Relaxed);
            }
        }
    }

    /// Record decision latency.
    pub fn record_latency(&self, start: Instant) {
        let micros = start.elapsed().as_micros() as u64;

        if micros < 1000 {
            self.latency_under_1ms.fetch_add(1, Ordering::Relaxed);
        } else if micros < 5000 {
            self.latency_1_5ms.fetch_add(1, Ordering::Relaxed);
        } else if micros < 10000 {
            self.latency_5_10ms.fetch_add(1, Ordering::Relaxed);
        } else if micros < 50000 {
            self.latency_10_50ms.fetch_add(1, Ordering::Relaxed);
        } else if micros < 100000 {
            self.latency_50_100ms.fetch_add(1, Ordering::Relaxed);
        } else {
            self.latency_over_100ms.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a rule evaluation.
    pub fn record_rule_evaluation(&self, triggered: bool) {
        self.rules_evaluated_total.fetch_add(1, Ordering::Relaxed);
        if triggered {
            self.rules_triggered_total.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a WAL write.
    pub fn record_wal_write(&self, success: bool) {
        self.wal_writes_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.wal_write_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Record a policy reload.
    pub fn record_policy_reload(&self, success: bool) {
        self.policy_reloads_total.fetch_add(1, Ordering::Relaxed);
        if !success {
            self.policy_reload_errors.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Export metrics in Prometheus format.
    pub fn to_prometheus(&self) -> String {
        format!(
            r#"# HELP riskr_decisions_total Total number of decision requests
# TYPE riskr_decisions_total counter
riskr_decisions_total {}

# HELP riskr_decisions Decision requests by outcome
# TYPE riskr_decisions counter
riskr_decisions{{outcome="allow"}} {}
riskr_decisions{{outcome="soft_deny"}} {}
riskr_decisions{{outcome="hold"}} {}
riskr_decisions{{outcome="review"}} {}
riskr_decisions{{outcome="reject"}} {}

# HELP riskr_decision_latency_bucket Decision latency histogram
# TYPE riskr_decision_latency_bucket counter
riskr_decision_latency_bucket{{le="0.001"}} {}
riskr_decision_latency_bucket{{le="0.005"}} {}
riskr_decision_latency_bucket{{le="0.01"}} {}
riskr_decision_latency_bucket{{le="0.05"}} {}
riskr_decision_latency_bucket{{le="0.1"}} {}
riskr_decision_latency_bucket{{le="+Inf"}} {}

# HELP riskr_rules_evaluated_total Total rule evaluations
# TYPE riskr_rules_evaluated_total counter
riskr_rules_evaluated_total {}

# HELP riskr_rules_triggered_total Total rules that triggered
# TYPE riskr_rules_triggered_total counter
riskr_rules_triggered_total {}

# HELP riskr_wal_writes_total Total WAL write operations
# TYPE riskr_wal_writes_total counter
riskr_wal_writes_total {}

# HELP riskr_wal_write_errors_total WAL write errors
# TYPE riskr_wal_write_errors_total counter
riskr_wal_write_errors_total {}

# HELP riskr_policy_reloads_total Policy reload operations
# TYPE riskr_policy_reloads_total counter
riskr_policy_reloads_total {}

# HELP riskr_policy_reload_errors_total Policy reload errors
# TYPE riskr_policy_reload_errors_total counter
riskr_policy_reload_errors_total {}
"#,
            self.decisions_total.load(Ordering::Relaxed),
            self.decisions_allow.load(Ordering::Relaxed),
            self.decisions_soft_deny.load(Ordering::Relaxed),
            self.decisions_hold.load(Ordering::Relaxed),
            self.decisions_review.load(Ordering::Relaxed),
            self.decisions_reject.load(Ordering::Relaxed),
            self.latency_under_1ms.load(Ordering::Relaxed),
            self.latency_1_5ms.load(Ordering::Relaxed),
            self.latency_5_10ms.load(Ordering::Relaxed),
            self.latency_10_50ms.load(Ordering::Relaxed),
            self.latency_50_100ms.load(Ordering::Relaxed),
            self.latency_over_100ms.load(Ordering::Relaxed),
            self.rules_evaluated_total.load(Ordering::Relaxed),
            self.rules_triggered_total.load(Ordering::Relaxed),
            self.wal_writes_total.load(Ordering::Relaxed),
            self.wal_write_errors.load(Ordering::Relaxed),
            self.policy_reloads_total.load(Ordering::Relaxed),
            self.policy_reload_errors.load(Ordering::Relaxed),
        )
    }
}

/// Guard for timing operations.
pub struct TimingGuard<'a> {
    registry: &'a MetricsRegistry,
    start: Instant,
}

impl<'a> TimingGuard<'a> {
    pub fn new(registry: &'a MetricsRegistry) -> Self {
        TimingGuard {
            registry,
            start: Instant::now(),
        }
    }
}

impl<'a> Drop for TimingGuard<'a> {
    fn drop(&mut self) {
        self.registry.record_latency(self.start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Decision;
    use std::thread::sleep;
    use std::time::Duration;

    #[test]
    fn test_record_decision() {
        let metrics = MetricsRegistry::new();

        metrics.record_decision(&Decision::Allow);
        metrics.record_decision(&Decision::Allow);
        metrics.record_decision(&Decision::RejectFatal);

        assert_eq!(metrics.decisions_total.load(Ordering::Relaxed), 3);
        assert_eq!(metrics.decisions_allow.load(Ordering::Relaxed), 2);
        assert_eq!(metrics.decisions_reject.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_record_latency() {
        let metrics = MetricsRegistry::new();

        let start = Instant::now();
        // Very fast operation
        metrics.record_latency(start);

        assert!(metrics.latency_under_1ms.load(Ordering::Relaxed) >= 1);
    }

    #[test]
    fn test_prometheus_format() {
        let metrics = MetricsRegistry::new();
        metrics.record_decision(&Decision::Allow);

        let output = metrics.to_prometheus();

        assert!(output.contains("riskr_decisions_total 1"));
        assert!(output.contains("riskr_decisions{outcome=\"allow\"} 1"));
    }
}
