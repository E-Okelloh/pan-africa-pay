//! Pan-Africa Pay API server.
//!
//! Boot sequence: load configuration, initialize tracing, connect to
//! PostgreSQL and Redis, apply migrations, then serve HTTP with
//! graceful shutdown on SIGINT/SIGTERM.

use std::net::SocketAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tracing::info;
use tracing_subscriber::EnvFilter;

use pan_africa_pay_api::config::AppConfig;
use pan_africa_pay_api::routes::build_router;
use pan_africa_pay_api::state::AppState;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load `.env` if present (values never override the process env).
    dotenvy::dotenv().ok();

    let config = AppConfig::load()?;

    init_tracing(&config.logging.level, config.logging.json);
    info!(env = %config.env, "starting pan-africa-pay API");

    let bind_host = config.server.host.clone();
    let bind_port = config.server.port;
    let state = AppState::new(config.clone()).await?;
    state.run_migrations().await?;

    // Spawn the reconciliation sweeper (settles payments whose provider
    // callback was missed).
    let sweeper = pan_africa_pay_api::reconciliation::ReconciliationSweeper::new(
        state.payments.clone(),
        state.mpesa.clone(),
        state.kotani.clone(),
        config.sweep.interval_secs,
        config.sweep.stale_minutes,
    );
    tokio::spawn(async move {
        sweeper.run().await;
    });

    let app = build_router(state);
    let addr = SocketAddr::new(
        bind_host
            .parse()
            .map_err(|e| anyhow::anyhow!("invalid APP_HOST '{bind_host}': {e}"))?,
        bind_port,
    );

    let listener = TcpListener::bind(addr)
        .await
        .map_err(|e| anyhow::anyhow!("failed to bind {addr}: {e}"))?;
    info!(%addr, "pan-africa-pay API listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| anyhow::anyhow!("server error: {e}"))?;

    info!("pan-africa-pay API stopped cleanly");
    Ok(())
}

/// Initialize the tracing subscriber with an env-filter.
fn init_tracing(level: &str, json: bool) {
    let filter = EnvFilter::try_from_default_env()
        .or_else(|_| EnvFilter::try_new(level))
        .expect("valid log filter");

    let builder = tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false);

    if json {
        builder.json().init();
    } else {
        builder.init();
    }
}

/// Resolve on SIGINT (Ctrl-C) or SIGTERM to trigger graceful shutdown.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    info!("shutdown signal received, draining connections");
    tokio::time::sleep(Duration::from_millis(100)).await;
}
