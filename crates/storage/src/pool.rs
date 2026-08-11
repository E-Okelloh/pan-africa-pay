//! Database connection pooling.
//!
//! Centralizes construction of PostgreSQL and Redis pools so the API
//! and service layers never deal with connection details directly.

use deadpool_redis::Pool as RedisPool;
use redis::ConnectionManager;
use sqlx::pg::PgPoolOptions;
use sqlx::PgPool;
use std::time::Duration;
use tracing::info;

use pan_africa_pay_domain::error::{AppError, AppResult};

/// Database connection settings.
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct DatabaseConfig {
    /// PostgreSQL connection string.
    pub url: String,
    /// Maximum number of concurrent connections to PostgreSQL.
    pub max_connections: u32,
    /// Redis connection string.
    pub redis_url: String,
    /// Maximum number of concurrent connections to Redis.
    pub redis_max_connections: usize,
    /// Seconds to wait for a connection before failing.
    pub connect_timeout_secs: u64,
}

impl Default for DatabaseConfig {
    fn default() -> Self {
        Self {
            url: "postgres://postgres:postgres@localhost:5432/pan_africa_pay".to_string(),
            max_connections: 10,
            redis_url: "redis://localhost:6379".to_string(),
            redis_max_connections: 10,
            connect_timeout_secs: 5,
        }
    }
}

/// Bundled database pools for the application.
#[derive(Clone)]
pub struct DatabasePool {
    pub pg: PgPool,
    pub redis: RedisPool,
}

impl DatabasePool {
    /// Build both pools from configuration.
    pub async fn connect(config: &DatabaseConfig) -> AppResult<Self> {
        let pg = connect_pg(config).await?;
        let redis = connect_redis(config).await?;
        Ok(Self { pg, redis })
    }

    /// Run SQL migrations from the embedded migrations directory.
    pub async fn run_migrations(&self) -> AppResult<()> {
        sqlx::migrate!("./migrations")
            .run(&self.pg)
            .await
            .map_err(|e| AppError::internal(format!("migration failed: {e}")))?;
        info!("database migrations applied");
        Ok(())
    }

    /// Run a health check against both stores.
    pub async fn health_check(&self) -> AppResult<()> {
        sqlx::query("SELECT 1")
            .execute(&self.pg)
            .await
            .map_err(|e| AppError::service_unavailable(format!("postgres unhealthy: {e}")))?;
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| AppError::service_unavailable(format!("redis unhealthy: {e}")))?;
        let pong: String = redis::cmd("PING")
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::service_unavailable(format!("redis unhealthy: {e}")))?;
        if pong != "PONG" {
            return Err(AppError::service_unavailable("redis returned unexpected PING response"));
        }
        Ok(())
    }
}

/// Build the PostgreSQL connection pool.
async fn connect_pg(config: &DatabaseConfig) -> AppResult<PgPool> {
    PgPoolOptions::new()
        .max_connections(config.max_connections)
        .acquire_timeout(Duration::from_secs(config.connect_timeout_secs))
        .connect(&config.url)
        .await
        .map_err(|e| AppError::configuration(format!("failed to connect to postgres: {e}")))
}

/// Build the Redis connection pool using the deadpool-managed pool.
async fn connect_redis(config: &DatabaseConfig) -> AppResult<RedisPool> {
    let client = redis::Client::open(config.redis_url.clone())
        .map_err(|e| AppError::configuration(format!("invalid redis url: {e}")))?;
    let manager: ConnectionManager = client
        .get_connection_manager()
        .await
        .map_err(|e| AppError::configuration(format!("failed to connect to redis: {e}")))?;
    let pool = RedisPool::builder(manager)
        .max_size(config.redis_max_connections)
        .build();
    Ok(pool)
}
