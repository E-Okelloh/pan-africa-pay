//! Configuration for the Kotani Pay client.

use serde::Deserialize;

/// Default HTTP timeout for Kotani calls (seconds).
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Sandbox base URL (matches Kotani docs).
pub const SANDBOX_BASE_URL: &str = "https://sandbox-api.kotanipay.io";

/// Production base URL.
pub const PRODUCTION_BASE_URL: &str = "https://api.kotanipay.io";

/// Kotani Pay client settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct KotaniConfig {
    /// API key from the Kotani Pay dashboard (used as a Bearer token).
    pub api_key: String,
    /// API secret, kept for integrations that need it.
    pub api_secret: String,
    /// Base URL (sandbox or production).
    pub base_url: String,
    /// Secret used to verify `X-Kotani-Signature` webhook headers.
    pub webhook_secret: String,
    /// Public HTTPS URL Kotani posts callbacks to.
    pub callback_url: String,
    /// Crypto wallet id used for deposits (from `GET /wallet/crypto`).
    pub wallet_id: String,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
}

impl Default for KotaniConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            api_secret: String::new(),
            base_url: SANDBOX_BASE_URL.to_string(),
            webhook_secret: String::new(),
            callback_url: String::new(),
            wallet_id: String::new(),
            timeout_secs: DEFAULT_TIMEOUT_SECS,
        }
    }
}

impl KotaniConfig {
    /// True if the config has the required values populated.
    pub fn is_configured(&self) -> bool {
        !self.api_key.is_empty()
    }

    /// Validate the configuration, returning which field is missing.
    pub fn validate(&self) -> crate::KotaniResult<()> {
        let required = [("api_key", &self.api_key), ("base_url", &self.base_url)];
        for (name, value) in required {
            if value.trim().is_empty() {
                return Err(crate::KotaniError::configuration(format!(
                    "Kotani {name} is not configured"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> KotaniConfig {
        KotaniConfig {
            api_key: "key".to_string(),
            base_url: SANDBOX_BASE_URL.to_string(),
            callback_url: "https://example.com/webhooks/kotani".to_string(),
            ..KotaniConfig::default()
        }
    }

    #[test]
    fn validate_accepts_complete_config() {
        assert!(config().validate().is_ok());
        assert!(config().is_configured());
    }

    #[test]
    fn validate_rejects_missing_key() {
        let cfg = KotaniConfig::default();
        assert!(cfg.validate().is_err());
        assert!(!cfg.is_configured());
    }

    #[test]
    fn defaults_point_at_sandbox() {
        assert_eq!(KotaniConfig::default().base_url, SANDBOX_BASE_URL);
        assert_ne!(SANDBOX_BASE_URL, PRODUCTION_BASE_URL);
    }
}
