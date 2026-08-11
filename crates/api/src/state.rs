//! Shared application state passed to route handlers.

use std::sync::Arc;

use pan_africa_pay_storage::DatabasePool;

use crate::config::AppConfig;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// Immutable application configuration.
    pub config: Arc<AppConfig>,
    /// Database and cache pools.
    pub pool: DatabasePool,
}

impl AppState {
    /// Build state from configuration, establishing pools eagerly.
    pub async fn new(config: AppConfig) -> anyhow::Result<Self> {
        let pool = DatabasePool::connect(&config.database)
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to stores: {e}"))?;
        Ok(Self {
            config: Arc::new(config),
            pool,
        })
    }

    /// Apply pending database migrations.
    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        self.pool
            .run_migrations()
            .await
            .map_err(|e| anyhow::anyhow!("migration failed: {e}"))
    }
}
