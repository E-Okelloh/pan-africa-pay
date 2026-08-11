//! Configuration for the M-Pesa Daraja client.

use serde::Deserialize;

/// Daraja API environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Environment {
    /// Sandbox (https://sandbox.safaricom.co.ke)
    Sandbox,
    /// Production (https://api.safaricom.co.ke)
    Production,
}

impl Environment {
    /// Base URL for this environment.
    pub fn base_url(self) -> &'static str {
        match self {
            Self::Sandbox => "https://sandbox.safaricom.co.ke",
            Self::Production => "https://api.safaricom.co.ke",
        }
    }
}

/// Default HTTP timeout for Daraja calls (seconds).
pub const DEFAULT_TIMEOUT_SECS: u64 = 30;

/// Default OAuth token lifetime assumed when the response omits one (seconds).
pub const DEFAULT_TOKEN_TTL_SECS: u64 = 3_500;

/// M-Pesa Daraja client settings.
#[derive(Debug, Clone, Deserialize)]
#[serde(default)]
pub struct MpesaConfig {
    /// Consumer key from the Daraja developer portal.
    pub consumer_key: String,
    /// Consumer secret from the Daraja developer portal.
    pub consumer_secret: String,
    /// Lipa Na M-Pesa online passkey.
    pub passkey: String,
    /// Business short code (paybill or till number).
    pub short_code: String,
    /// Public URL that Daraja calls back on completion.
    pub callback_url: String,
    /// Sandbox or production.
    pub environment: Environment,
    /// Request timeout in seconds.
    pub timeout_secs: u64,
    /// Fallback OAuth token TTL in seconds.
    pub token_ttl_secs: u64,
    /// Override for the Daraja base URL (testing against mocks).
    ///
    /// Defaults to the environment's base URL when empty.
    pub base_url_override: String,
}

impl Default for MpesaConfig {
    fn default() -> Self {
        Self {
            consumer_key: String::new(),
            consumer_secret: String::new(),
            passkey: String::new(),
            short_code: String::new(),
            callback_url: String::new(),
            environment: Environment::Sandbox,
            timeout_secs: DEFAULT_TIMEOUT_SECS,
            token_ttl_secs: DEFAULT_TOKEN_TTL_SECS,
            base_url_override: String::new(),
        }
    }
}

impl MpesaConfig {
    /// Effective Daraja base URL (override wins when set).
    pub fn base_url(&self) -> &str {
        if self.base_url_override.is_empty() {
            self.environment.base_url()
        } else {
            &self.base_url_override
        }
    }

    /// True if sandbox environment with placeholder credentials.
    pub fn is_sandbox(&self) -> bool {
        self.environment == Environment::Sandbox
    }

    /// True if the config has all required values populated.
    pub fn is_configured(&self) -> bool {
        !self.consumer_key.is_empty()
            && !self.consumer_secret.is_empty()
            && !self.short_code.is_empty()
            && !self.callback_url.is_empty()
    }

    /// Validate the configuration, returning which field is missing.
    pub fn validate(&self) -> MpesaResult<()> {
        let required = [
            ("consumer_key", &self.consumer_key),
            ("consumer_secret", &self.consumer_secret),
            ("passkey", &self.passkey),
            ("short_code", &self.short_code),
            ("callback_url", &self.callback_url),
        ];
        for (name, value) in required {
            if value.trim().is_empty() {
                return Err(MpesaError::configuration(format!(
                    "M-Pesa {name} is not configured"
                )));
            }
        }
        Ok(())
    }
}

use crate::error::{MpesaError, MpesaResult};

#[cfg(test)]
mod tests {
    use super::*;

    fn sandbox_config() -> MpesaConfig {
        MpesaConfig {
            consumer_key: "key".to_string(),
            consumer_secret: "secret".to_string(),
            passkey: "passkey".to_string(),
            short_code: "174379".to_string(),
            callback_url: "https://example.com/cb".to_string(),
            ..MpesaConfig::default()
        }
    }

    #[test]
    fn validate_accepts_complete_config() {
        let config = sandbox_config();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn validate_rejects_missing_fields() {
        let config = MpesaConfig::default();
        assert!(config.validate().is_err());
        assert!(!config.is_configured());
    }

    #[test]
    fn environments_have_distinct_base_urls() {
        assert_ne!(
            Environment::Sandbox.base_url(),
            Environment::Production.base_url()
        );
        assert!(Environment::Sandbox.base_url().contains("sandbox"));
    }

    #[test]
    fn base_url_override_wins() {
        let config = MpesaConfig {
            base_url_override: "http://localhost:8080".to_string(),
            ..sandbox_config()
        };
        assert_eq!(config.base_url(), "http://localhost:8080");
    }
}
