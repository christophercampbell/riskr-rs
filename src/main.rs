use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use tokio::signal;
use tracing::{error, info};

use riskr::actor::pool::ActorPool;
use riskr::api::routes::{create_router, AppState};
use riskr::config::Config;
use riskr::observability::init_tracing;
use riskr::policy::{PolicyLoader, PolicyWatcher};
use riskr::storage::wal::WalWriter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Parse configuration
    let config = Config::parse();

    // Initialize tracing
    init_tracing(&config.log_level);

    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Starting riskr decision engine"
    );

    // Load initial policy
    let loader = PolicyLoader::new(
        config.policy_path.to_string_lossy(),
        config.sanctions_path.to_string_lossy(),
    );

    // Start policy watcher
    let watcher = PolicyWatcher::new(loader, config.policy_reload_interval());
    let (ruleset_rx, policy_handle) = watcher.start();

    // Get initial ruleset for actor pool
    let initial_ruleset = ruleset_rx.borrow().clone();

    // Create actor pool
    let actor_pool = Arc::new(ActorPool::new(initial_ruleset.streaming.clone()));

    // Create WAL writer (optional)
    let wal_writer = if let Some(ref wal_path) = config.wal_path {
        match WalWriter::open(wal_path) {
            Ok(writer) => {
                info!(path = %wal_path.display(), "WAL enabled");
                Some(Arc::new(parking_lot::Mutex::new(writer)))
            }
            Err(e) => {
                error!(error = %e, "Failed to create WAL writer, continuing without WAL");
                None
            }
        }
    } else {
        info!("WAL disabled (no path configured)");
        None
    };

    // Create application state
    let state = Arc::new(AppState {
        actor_pool,
        ruleset_rx,
        wal_writer,
        start_time: Instant::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        latency_budget_ms: config.latency_budget_ms,
    });

    // Create router
    let app = create_router(state);

    // Parse listen address
    let addr: SocketAddr = config.listen_addr.parse()?;

    info!(addr = %addr, "Starting HTTP server");

    // Create TCP listener
    let listener = tokio::net::TcpListener::bind(addr).await?;

    // Run server with graceful shutdown
    if config.graceful_shutdown {
        axum::serve(listener, app)
            .with_graceful_shutdown(shutdown_signal())
            .await?;
    } else {
        axum::serve(listener, app).await?;
    }

    // Cleanup
    info!("Shutting down...");
    policy_handle.abort();

    info!("Shutdown complete");
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c()
            .await
            .expect("Failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("Failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("Received shutdown signal");
}
