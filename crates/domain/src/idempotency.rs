//! Idempotency primitives for safe retries.
//!
//! Payment APIs must be safe to retry: a client that loses its
//! connection after initiating a payment should be able to retry the
//! same request and get the same result, without creating a second
//! payment. This module provides the key, request fingerprint, and
//! record types that make that possible.

use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Default lifetime of an idempotency record (24 hours).
pub const DEFAULT_TTL_SECS: u64 = 86_400;

/// Maximum size of an idempotency key (header value bound).
pub const MAX_KEY_LENGTH: usize = 128;

/// A client-supplied idempotency key.
///
/// Keys are opaque strings, typically a UUID or a business reference
/// such as an order id. The platform stores the key together with a
/// fingerprint of the request body so it can detect when the same key
/// is reused with a *different* request (a conflict).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IdempotencyKey(String);

impl IdempotencyKey {
    /// Generate a new random idempotency key.
    pub fn new() -> Self {
        Self(Uuid::new_v4().to_string())
    }

    /// Validate and construct a key from a raw header value.
    pub fn parse(raw: &str) -> AppResult<Self> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(AppError::validation("Idempotency key must not be empty"));
        }
        if trimmed.len() > MAX_KEY_LENGTH {
            return Err(AppError::validation(format!(
                "Idempotency key exceeds maximum length of {MAX_KEY_LENGTH}"
            )));
        }
        // Restrict to URL-safe printable characters.
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        {
            return Err(AppError::validation(
                "Idempotency key must be URL-safe (alphanumeric, '-', '_', '.')",
            ));
        }
        Ok(Self(trimmed.to_string()))
    }

    /// The raw key value.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for IdempotencyKey {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for IdempotencyKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A fingerprint of a request body.
///
/// Used to detect idempotency conflicts: when a key is reused with a
/// different request body, the stored hash will not match and the
/// platform rejects the request instead of silently returning a
/// response for a different operation.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RequestHash(String);

impl RequestHash {
    /// Compute a hash over a canonical JSON representation of the body.
    ///
    /// Canonicalizing via `serde_json::to_vec` ensures that field
    /// ordering and whitespace differences do not produce false
    /// conflicts.
    pub fn compute(body: &impl Serialize) -> Self {
        let canonical = serde_json::to_vec(body).unwrap_or_default();
        Self(blake3::hash(&canonical).to_hex().to_string())
    }

    /// Compute a hash directly from serialized bytes.
    pub fn compute_bytes(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }

    /// The raw hex digest.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A stored idempotency record (survivor of a completed request).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredIdempotencyRecord {
    pub key: IdempotencyKey,
    pub request_hash: RequestHash,
    pub response_body: serde_json::Value,
    pub status_code: u16,
    /// Unix timestamp (seconds) at which the record expires.
    pub expires_at: u64,
}

impl StoredIdempotencyRecord {
    /// Create a new record with the default TTL.
    pub fn new(
        key: IdempotencyKey,
        request_hash: RequestHash,
        response_body: serde_json::Value,
        status_code: u16,
    ) -> Self {
        Self {
            key,
            request_hash,
            response_body,
            status_code,
            expires_at: unix_now() + DEFAULT_TTL_SECS,
        }
    }

    /// True if the record has not expired yet.
    pub fn is_valid(&self) -> bool {
        unix_now() < self.expires_at
    }

    /// The remaining lifetime in seconds (0 if expired).
    pub fn ttl_secs(&self) -> u64 {
        self.expires_at.saturating_sub(unix_now())
    }
}

/// Outcome of attempting to claim an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IdempotencyOutcome {
    /// First time seeing this key: proceed with the request.
    Fresh,
    /// Key exists and the request fingerprint matches: replay the stored response.
    Replay(StoredIdempotencyRecord),
    /// Key exists but the fingerprint differs: caller error.
    Conflict(StoredIdempotencyRecord),
    /// Key exists but its record expired: safe to reuse.
    Expired,
}

/// Decide how to handle an idempotency key against a stored record.
///
/// # Arguments
/// - `existing`: the record previously stored for this key, if any.
/// - `incoming_hash`: fingerprint of the current request body.
pub fn resolve_idempotency(
    existing: Option<&StoredIdempotencyRecord>,
    incoming_hash: &RequestHash,
) -> IdempotencyOutcome {
    let Some(record) = existing else {
        return IdempotencyOutcome::Fresh;
    };

    if !record.is_valid() {
        return IdempotencyOutcome::Expired;
    }

    if record.request_hash == *incoming_hash {
        IdempotencyOutcome::Replay(record.clone())
    } else {
        IdempotencyOutcome::Conflict(record.clone())
    }
}

/// Current Unix time in seconds.
fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Serialize)]
    struct TestBody {
        amount: i64,
        phone: String,
    }

    #[test]
    fn key_parse_accepts_valid_keys() {
        assert!(IdempotencyKey::parse("order-123_ABC").is_ok());
        assert!(IdempotencyKey::parse(Uuid::new_v4().to_string().as_str()).is_ok());
    }

    #[test]
    fn key_parse_rejects_invalid_keys() {
        assert!(IdempotencyKey::parse("").is_err());
        assert!(IdempotencyKey::parse("with space").is_err());
        assert!(IdempotencyKey::parse("with/slash").is_err());
        assert!(IdempotencyKey::parse(&"a".repeat(MAX_KEY_LENGTH + 1)).is_err());
    }

    #[test]
    fn hash_is_stable_for_same_body() {
        let body = TestBody {
            amount: 1000,
            phone: "+254712345678".to_string(),
        };
        let h1 = RequestHash::compute(&body);
        let h2 = RequestHash::compute(&body);
        assert_eq!(h1, h2);
    }

    #[test]
    fn hash_differs_for_different_body() {
        let a = RequestHash::compute(&TestBody {
            amount: 1000,
            phone: "+254712345678".to_string(),
        });
        let b = RequestHash::compute(&TestBody {
            amount: 2000,
            phone: "+254712345678".to_string(),
        });
        assert_ne!(a, b);
    }

    #[test]
    fn resolve_fresh_when_no_record() {
        let hash = RequestHash::compute_bytes(b"{}");
        assert_eq!(resolve_idempotency(None, &hash), IdempotencyOutcome::Fresh);
    }

    #[test]
    fn resolve_replay_on_match() {
        let body = TestBody {
            amount: 1000,
            phone: "+254712345678".to_string(),
        };
        let hash = RequestHash::compute(&body);
        let record = StoredIdempotencyRecord::new(
            IdempotencyKey::parse("key-1").unwrap(),
            hash.clone(),
            serde_json::json!({"ok": true}),
            200,
        );
        match resolve_idempotency(Some(&record), &hash) {
            IdempotencyOutcome::Replay(r) => assert_eq!(r, record),
            other => panic!("expected Replay, got {other:?}"),
        }
    }

    #[test]
    fn resolve_conflict_on_mismatch() {
        let body = TestBody {
            amount: 1000,
            phone: "+254712345678".to_string(),
        };
        let hash_a = RequestHash::compute(&body);
        let hash_b = RequestHash::compute_bytes(b"different");
        let record = StoredIdempotencyRecord::new(
            IdempotencyKey::parse("key-1").unwrap(),
            hash_a,
            serde_json::json!({"ok": true}),
            200,
        );
        assert!(matches!(
            resolve_idempotency(Some(&record), &hash_b),
            IdempotencyOutcome::Conflict(_)
        ));
    }

    #[test]
    fn stored_record_ttl_counts_down() {
        let record = StoredIdempotencyRecord::new(
            IdempotencyKey::parse("key-1").unwrap(),
            RequestHash::compute_bytes(b"x"),
            serde_json::json!({}),
            200,
        );
        assert!(record.is_valid());
        assert!(record.ttl_secs() <= DEFAULT_TTL_SECS);
        assert!(record.ttl_secs() > DEFAULT_TTL_SECS - 60);
    }
}
