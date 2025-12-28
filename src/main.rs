use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use clap::Parser;
use tokio::signal;
use tracing::info;

use riskr::api::routes::{create_router, AppState};
use riskr::config::Config;
use riskr::observability::init_tracing;
use riskr::policy::{PolicyLoader, PolicyWatcher};
use riskr::storage::{MockStorage, PostgresStorage, Storage};

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

    // Create storage backend
    let storage: Arc<dyn Storage> = if let Some(ref database_url) = config.database_url {
        info!("Connecting to PostgreSQL...");
        let pg_storage = PostgresStorage::connect(
            database_url,
            config.db_pool_min,
            config.db_pool_max,
        )
        .await?;

        if config.run_migrations {
            info!("Running database migrations...");
            pg_storage.run_migrations().await?;
        }

        info!("PostgreSQL storage initialized");
        Arc::new(pg_storage)
    } else {
        info!("No database configured, using in-memory mock storage");
        Arc::new(MockStorage::new())
    };

    // Create application state
    let state = Arc::new(AppState {
        storage,
        ruleset_rx,
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
