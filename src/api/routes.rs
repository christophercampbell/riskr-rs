use axum::{
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Json, Router,
};
use std::sync::Arc;
use std::time::Instant;
use tokio::sync::watch;
use tracing::{info, warn};

use crate::actor::pool::ActorPool;
use crate::domain::Decision;
use crate::rules::RuleSet;
use crate::storage::wal::WalWriter;

use super::request::DecisionRequest;
use super::response::{DecisionResponse, ErrorResponse, HealthResponse, ReadyResponse};

/// Shared application state.
pub struct AppState {
    /// Actor pool for user state management
    pub actor_pool: Arc<ActorPool>,

    /// Current rule set (updated via watch channel)
    pub ruleset_rx: watch::Receiver<Arc<RuleSet>>,

    /// WAL writer for durability
    pub wal_writer: Option<Arc<parking_lot::Mutex<WalWriter>>>,

    /// Application start time
    pub start_time: Instant,

    /// Application version
    pub version: String,

    /// Latency budget in milliseconds
    pub latency_budget_ms: u64,
}

/// Create the application router.
pub fn create_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/v1/decision/check", post(handle_decision))
        .route("/health", get(handle_health))
        .route("/ready", get(handle_ready))
        .route("/metrics", get(handle_metrics))
        .with_state(state)
}

/// Handle decision check requests.
async fn handle_decision(
    State(state): State<Arc<AppState>>,
    Json(req): Json<DecisionRequest>,
) -> impl IntoResponse {
    let start = Instant::now();

    // Convert request to TxEvent
    let event = req.to_tx_event();
    let user_id = event.subject.user_id.as_str();

    // Get current ruleset
    let ruleset = state.ruleset_rx.borrow().clone();

    // Phase 1: Evaluate inline rules (stateless)
    let mut final_decision = Decision::Allow;
    let mut evidence = Vec::new();

    for rule in &ruleset.inline {
        let result = rule.evaluate(&event);
        if result.hit {
            if result.decision > final_decision {
                final_decision = result.decision;
            }
            if let Some(ev) = result.evidence {
                evidence.push(ev);
            }
        }
    }

    // Short-circuit if fatal decision from inline rules
    if final_decision.is_fatal() {
        let elapsed = start.elapsed();
        if elapsed.as_millis() > state.latency_budget_ms as u128 {
            warn!(
                user_id = user_id,
                latency_ms = elapsed.as_millis(),
                "Decision latency exceeded budget"
            );
        }

        return (
            StatusCode::OK,
            Json(DecisionResponse::new(
                final_decision,
                ruleset.policy_version.clone(),
                evidence,
            )),
        );
    }

    // Phase 2: Evaluate streaming rules (stateful)
    let actor = state.actor_pool.get_or_create(user_id);
    let actor_guard = actor.lock();

    for rule in &ruleset.streaming {
        let result = rule.evaluate(&event, actor_guard.state());
        if result.hit {
            if result.decision > final_decision {
                final_decision = result.decision;
            }
            if let Some(ev) = result.evidence {
                evidence.push(ev);
            }
        }
    }

    // Update actor state with this transaction
    let _ = actor_guard.state().clone(); // Touch to update last_access
    drop(actor_guard);

    // Get a fresh lock to call evaluate which updates state
    let actor = state.actor_pool.get_or_create(user_id);
    let mut actor_guard = actor.lock();

    // Re-evaluate to update state (this is a bit redundant but ensures state is updated)
    // In a production system, we'd refactor to separate state update from evaluation
    let _ = actor_guard.evaluate(&event);
    drop(actor_guard);

    // Write to WAL for durability
    if let Some(ref wal) = state.wal_writer {
        let entry = crate::storage::wal::WalEntry::transaction(
            user_id.to_string(),
            event.occurred_at,
            event.usd_value,
        );

        if let Err(e) = wal.lock().append(&entry) {
            warn!(user_id = user_id, error = %e, "Failed to write to WAL");
        }
    }

    // Check latency budget
    let elapsed = start.elapsed();
    if elapsed.as_millis() > state.latency_budget_ms as u128 {
        warn!(
            user_id = user_id,
            latency_ms = elapsed.as_millis(),
            budget_ms = state.latency_budget_ms,
            "Decision latency exceeded budget"
        );
    }

    info!(
        user_id = user_id,
        decision = %final_decision,
        latency_ms = elapsed.as_millis(),
        "Decision completed"
    );

    (
        StatusCode::OK,
        Json(DecisionResponse::new(
            final_decision,
            ruleset.policy_version.clone(),
            evidence,
        )),
    )
}

/// Health check endpoint.
async fn handle_health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let ruleset = state.ruleset_rx.borrow();

    Json(HealthResponse {
        status: "healthy".to_string(),
        version: state.version.clone(),
        policy_version: ruleset.policy_version.clone(),
        uptime_secs: state.start_time.elapsed().as_secs(),
    })
}

/// Readiness check endpoint.
async fn handle_ready(State(state): State<Arc<AppState>>) -> axum::response::Response {
    let ruleset = state.ruleset_rx.borrow();

    // Check if we have rules loaded
    if ruleset.inline.is_empty() && ruleset.streaming.is_empty() {
        return (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse::new("No rules loaded", "NOT_READY")),
        )
            .into_response();
    }

    (
        StatusCode::OK,
        Json(ReadyResponse {
            ready: true,
            policy_version: ruleset.policy_version.clone(),
            inline_rules: ruleset.inline.len(),
            streaming_rules: ruleset.streaming.len(),
        }),
    )
        .into_response()
}

/// Metrics endpoint (Prometheus format).
async fn handle_metrics(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let pool_stats = state.actor_pool.stats();
    let ruleset = state.ruleset_rx.borrow();

    let metrics = format!(
        r#"# HELP riskr_actors_total Total number of user actors
# TYPE riskr_actors_total gauge
riskr_actors_total {}

# HELP riskr_entries_total Total transaction entries across all actors
# TYPE riskr_entries_total gauge
riskr_entries_total {}

# HELP riskr_uptime_seconds Application uptime in seconds
# TYPE riskr_uptime_seconds counter
riskr_uptime_seconds {}

# HELP riskr_inline_rules Number of inline rules loaded
# TYPE riskr_inline_rules gauge
riskr_inline_rules {}

# HELP riskr_streaming_rules Number of streaming rules loaded
# TYPE riskr_streaming_rules gauge
riskr_streaming_rules {}
"#,
        pool_stats.total_actors,
        pool_stats.total_entries,
        state.start_time.elapsed().as_secs(),
        ruleset.inline.len(),
        ruleset.streaming.len(),
    );

    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        metrics,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::rules::{DailyVolumeRule, OfacRule};
    use rust_decimal::Decimal;
    use std::collections::HashSet;

    fn test_app_state() -> Arc<AppState> {
        let mut sanctions = HashSet::new();
        sanctions.insert("0xdead".to_string());

        let inline_rules: Vec<Arc<dyn crate::rules::InlineRule>> = vec![Arc::new(OfacRule::new(
            "R1_OFAC".to_string(),
            Decision::RejectFatal,
            sanctions,
        ))];

        let streaming_rules: Vec<Arc<dyn crate::rules::StreamingRule>> =
            vec![Arc::new(DailyVolumeRule::new(
                "R4_DAILY".to_string(),
                Decision::HoldAuto,
                Decimal::new(50000, 0),
            ))];

        let ruleset = Arc::new(RuleSet {
            inline: inline_rules,
            streaming: streaming_rules.clone(),
            policy_version: "test-v1".to_string(),
        });

        let (tx, rx) = watch::channel(ruleset);
        let pool = Arc::new(ActorPool::new(streaming_rules));

        Arc::new(AppState {
            actor_pool: pool,
            ruleset_rx: rx,
            wal_writer: None,
            start_time: Instant::now(),
            version: "0.1.0-test".to_string(),
            latency_budget_ms: 100,
        })
    }

    #[tokio::test]
    async fn test_health_endpoint() {
        let state = test_app_state();
        let app = create_router(state);

        let response = axum::http::Request::builder()
            .uri("/health")
            .body(axum::body::Body::empty())
            .unwrap();

        let response = tower::ServiceExt::oneshot(app, response).await.unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
