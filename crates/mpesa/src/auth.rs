//! OAuth2 token acquisition and caching for Daraja.
//!
//! Daraja issues short-lived tokens (default ~3,600 s). We cache the
//! token in memory alongside its expiry and refresh it before use when
//! it is close to expiring, so calls in flight never wait on a
//! synchronous refresh.

use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use reqwest::Client;
use tracing::debug;

use crate::config::Environment;
use crate::error::{MpesaError, MpesaResult};

/// Deserialize an optional integer that Daraja occasionally sends as a
/// string (`"expires_in": "3599"`).
fn de_string_or_u64<'de, D>(deserializer: D) -> Result<Option<u64>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::de::Error;
    use serde::Deserialize;

    let value = Option::<serde_json::Value>::deserialize(deserializer)?;
    match value {
        None => Ok(None),
        Some(serde_json::Value::Number(n)) => n
            .as_u64()
            .map(Some)
            .ok_or_else(|| D::Error::custom(format!("expires_in is not a valid u64: {n}"))),
        Some(serde_json::Value::String(s)) => s
            .parse::<u64>()
            .map(Some)
            .map_err(|_| D::Error::custom(format!("expires_in is not a valid u64: {s:?}"))),
        Some(other) => Err(D::Error::custom(format!(
            "expires_in has unexpected type: {other}"
        ))),
    }
}

/// Threshold: refresh the token when less than this fraction of its
/// lifetime remains.
const REFRESH_EARLY_FRACTION: f64 = 0.9;

/// In-memory OAuth token cache with refresh-on-demand.
pub struct TokenCache {
    inner: Arc<TokenCacheInner>,
}

struct TokenCacheInner {
    client: Client,
    base_url: String,
    consumer_key: String,
    consumer_secret: String,
    /// Unix timestamp (seconds) of the cached token's expiry, 0 if none.
    expires_at: AtomicI64,
    token: parking_lot::Mutex<Option<String>>,
    fallback_ttl_secs: u64,
}

impl TokenCache {
    /// Create a new cache bound to the given client and credentials.
    pub fn new(
        client: Client,
        env: Environment,
        consumer_key: String,
        consumer_secret: String,
        fallback_ttl_secs: u64,
        base_url_override: String,
    ) -> Self {
        let base_url = if base_url_override.is_empty() {
            env.base_url().to_string()
        } else {
            base_url_override
        };
        Self {
            inner: Arc::new(TokenCacheInner {
                client,
                base_url,
                consumer_key,
                consumer_secret,
                expires_at: AtomicI64::new(0),
                token: parking_lot::Mutex::new(None),
                fallback_ttl_secs,
            }),
        }
    }

    /// Return a valid bearer token, refreshing if needed.
    pub async fn token(&self) -> MpesaResult<String> {
        if self.is_valid() {
            if let Some(token) = &self.inner.token.lock().clone() {
                return Ok(token.clone());
            }
        }
        self.fetch().await
    }

    /// True if the cached token is cached and not near expiry.
    fn is_valid(&self) -> bool {
        let now = unix_now();
        let expires_at = self.inner.expires_at.load(Ordering::Acquire);
        if expires_at == 0 {
            return false;
        }
        let remaining = expires_at - now as i64;
        let lifetime = self.inner.fallback_ttl_secs as i64;
        remaining > (lifetime as f64 * (1.0 - REFRESH_EARLY_FRACTION)) as i64 && remaining > 60
    }

    /// Fetch a fresh token from Daraja and cache it.
    async fn fetch(&self) -> MpesaResult<String> {
        let inner = &*self.inner;
        let basic = base64::engine::general_purpose::STANDARD
            .encode(format!("{}:{}", inner.consumer_key, inner.consumer_secret));

        debug!(provider = "mpesa", "requesting Daraja OAuth token");

        let response = inner
            .client
            .get(format!("{}/oauth/v1/generate", inner.base_url))
            .query(&[("grant_type", "client_credentials")])
            .header("Authorization", format!("Basic {basic}"))
            .send()
            .await
            .map_err(MpesaError::from)?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            let body = response.text().await.unwrap_or_default();
            return Err(MpesaError::Authentication(format!(
                "token endpoint returned HTTP {status}: {body}"
            )));
        }

        #[derive(serde::Deserialize)]
        struct TokenResponse {
            access_token: String,
            #[serde(default, deserialize_with = "de_string_or_u64")]
            expires_in: Option<u64>,
        }

        let payload: TokenResponse = response
            .json()
            .await
            .map_err(|e| MpesaError::Decode(format!("token response: {e}")))?;

        let expires_in = payload.expires_in.unwrap_or(inner.fallback_ttl_secs);
        let expires_at = unix_now() + expires_in;

        inner.expires_at.store(expires_at as i64, Ordering::Release);
        *inner.token.lock() = Some(payload.access_token.clone());

        debug!(
            provider = "mpesa",
            expires_in_secs = expires_in,
            "cached Daraja OAuth token"
        );
        Ok(payload.access_token)
    }

    /// Force-drop the cached token (used on auth failures).
    pub fn invalidate(&self) {
        self.inner.expires_at.store(0, Ordering::Release);
        *self.inner.token.lock() = None;
    }
}

impl Clone for TokenCache {
    fn clone(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
        }
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[allow(dead_code)]
fn _assert_send_sync() {
    fn assert_send_sync<T: Send + Sync>() {}
    assert_send_sync::<TokenCache>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_starts_invalid() {
        let cache = TokenCache::new(
            Client::new(),
            Environment::Sandbox,
            "k".to_string(),
            "s".to_string(),
            3_500,
            String::new(),
        );
        assert!(!cache.is_valid());
        cache.invalidate();
    }

    #[test]
    fn freshly_fetched_token_not_retried_while_valid() {
        // Behavior test: once a token is loaded, `is_valid` is true and
        // a second fetch is not triggered until expiry. We drive the
        // internal state directly to avoid network access.
        let cache = TokenCache::new(
            Client::new(),
            Environment::Sandbox,
            "k".to_string(),
            "s".to_string(),
            3_500,
            String::new(),
        );
        let now = unix_now();
        cache
            .inner
            .expires_at
            .store((now + 3_000) as i64, Ordering::Release);
        *cache.inner.token.lock() = Some("cached".to_string());
        assert!(cache.is_valid());
        assert_eq!(cache.inner.token.lock().as_deref(), Some("cached"));
    }

    #[test]
    fn token_cache_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<TokenCache>();
    }

    #[test]
    fn token_response_accepts_string_or_number_expires_in() {
        use serde::Deserialize;

        #[derive(Deserialize)]
        struct TokenResponse {
            #[serde(default, deserialize_with = "super::de_string_or_u64")]
            expires_in: Option<u64>,
        }

        let from_string: TokenResponse =
            serde_json::from_str(r#"{"expires_in":"3599"}"#).expect("string form");
        assert_eq!(from_string.expires_in, Some(3599));

        let from_number: TokenResponse =
            serde_json::from_str(r#"{"expires_in":3600}"#).expect("number form");
        assert_eq!(from_number.expires_in, Some(3600));

        let missing: TokenResponse = serde_json::from_str(r#"{}"#).expect("absent");
        assert_eq!(missing.expires_in, None);
    }
}
