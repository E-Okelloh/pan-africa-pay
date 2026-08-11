//! Application configuration.
//!
//! Configuration is layered: embedded defaults, then environment
//! variables. Precedence is increasing order, so the process
//! environment wins over defaults. Variables follow the `.env.example`
//! contract (`APP_*`, `DATABASE_*`, `REDIS_*`, `LOG_*`).

use figment::providers::{Env, Format, Toml};
use figment::Figment;
use serde::Deserialize;

use pan_africa_pay_storage::DatabaseConfig;

/// Default HTTP bind port.
pub const DEFAULT_PORT: u16 = 3000;

/// Runtime environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    #[default]
    Development,
    Test,
    Production,
}

impl std::fmt::Display for Environment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let s = match self {
            Self::Development => "development",
            Self::Test => "test",
            Self::Production => "production",
        };
        write!(f, "{s}")
    }
}

/// HTTP server settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct ServerConfig {
    /// Address to bind (e.g. `0.0.0.0` or `127.0.0.1`).
    pub host: String,
    /// Port to listen on.
    pub port: u16,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".to_string(),
            port: DEFAULT_PORT,
        }
    }
}

/// Logging settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct LoggingConfig {
    /// Filter directive for `tracing_subscriber` (e.g. `debug`).
    pub level: String,
    /// Emit JSON logs (structured output for production).
    pub json: bool,
}

impl Default for LoggingConfig {
    fn default() -> Self {
        Self {
            level: "info".to_string(),
            json: false,
        }
    }
}

/// Top-level application configuration.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct AppConfig {
    /// Runtime environment.
    pub env: Environment,
    /// HTTP server settings.
    pub server: ServerConfig,
    /// Database and cache settings.
    pub database: DatabaseConfig,
    /// Logging settings.
    pub logging: LoggingConfig,
}

impl AppConfig {
    /// Load configuration from defaults and environment variables.
    pub fn load() -> anyhow::Result<Self> {
        Self::from_figment(default_figment())
    }

    /// Build configuration from a custom figment (used in tests).
    pub fn from_figment(figment: Figment) -> anyhow::Result<Self> {
        let config: AppConfig = figment
            .extract()
            .map_err(|e| anyhow::anyhow!("invalid configuration: {e}"))?;
        Ok(config)
    }

    /// True if running in a non-production environment.
    pub fn is_non_production(&self) -> bool {
        self.env != Environment::Production
    }
}

/// A figment seeded with defaults and environment providers.
///
/// Values are read from `APP_*`, `DATABASE_*`, `REDIS_*`, and `LOG_*`
/// variables. Keys are mapped from uppercase to their nested config
/// paths, e.g. `DATABASE_URL` -> `database.url`. Missing keys fall
/// back to the `Default` implementations of each config section.
pub fn default_figment() -> Figment {
    Figment::new()
        .merge(Toml::file("app.toml").nested())
        .merge(Env::prefixed("APP_").split("_").map(|k| {
            let key = k.to_string().to_lowercase();
            match key.as_str() {
                "host" => "server.host".to_string(),
                "port" => "server.port".to_string(),
                other => other.to_string(),
            }
            .into()
        }))
        .merge(
            Env::prefixed("LOG_")
                .split("_")
                .map(|k| format!("logging.{}", k.to_string().to_lowercase()).into()),
        )
        .merge(
            Env::prefixed("DATABASE_")
                .split("_")
                .map(|k| format!("database.{}", k.to_string().to_lowercase()).into()),
        )
        .merge(
            Env::prefixed("REDIS_")
                .split("_")
                .map(|k| format!("database.redis_{}", k.to_string().to_lowercase()).into()),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    #[test]
    fn defaults_apply_then_environment_overrides() {
        let _guard = ENV_LOCK.lock().expect("env lock");
        std::env::remove_var("APP_PORT");
        std::env::remove_var("REDIS_URL");

        let config = AppConfig::from_figment(default_figment()).expect("config");
        assert_eq!(config.server.host, "0.0.0.0");
        assert_eq!(config.server.port, DEFAULT_PORT);
        assert_eq!(config.env, Environment::Development);
        assert_eq!(config.database.max_connections, 10);
        assert_eq!(config.database.redis_max_connections, 10);
        assert_eq!(config.logging.level, "info");

        unsafe {
            std::env::set_var("APP_PORT", "7000");
            std::env::set_var("REDIS_URL", "redis://override:6379");
        }
        let config = AppConfig::from_figment(default_figment()).expect("config");
        assert_eq!(config.server.port, 7000);
        assert_eq!(config.database.redis_url, "redis://override:6379");
        assert_eq!(config.server.host, "0.0.0.0");

        unsafe {
            std::env::remove_var("APP_PORT");
            std::env::remove_var("REDIS_URL");
        }
    }
}
