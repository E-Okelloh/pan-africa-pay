//! API-layer idempotency support.
//!
//! Every mutating endpoint accepts an optional `Idempotency-Key` header.
//! When present, the key is validated, resolved against the store, and
//! the stored response is replayed on retries so callers can never
//! double-charge. Keys reused with a different request body produce a
//! conflict.

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use std::sync::Arc;

use pan_africa_pay_domain::idempotency::{IdempotencyKey, RequestHash, DEFAULT_TTL_SECS};
use pan_africa_pay_domain::traits::IdempotencyRepository;

use crate::error::{ApiError, ApiResult};

/// Header carrying the client-supplied idempotency key.
pub const IDEMPOTENCY_HEADER: &str = "idempotency-key";

/// Outcome of claiming an idempotency key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Claim {
    /// First use of this key: proceed with the request.
    Fresh,
    /// Same key + same body seen before: return the stored response.
    Replay(pan_africa_pay_domain::traits::IdempotencyRecord),
    /// Same key, different body: caller error.
    Conflict(pan_africa_pay_domain::traits::IdempotencyRecord),
}

/// Extractor for the `Idempotency-Key` header.
///
/// `None` when the header is absent (idempotency is optional);
/// a `Some(key)` when present and valid; a 400 validation error when
/// present but malformed. Unlike `Option<Self>`, this does not swallow
/// extraction rejections.
#[derive(Clone)]
pub struct IdempotencyHeader(pub Option<IdempotencyKey>);

#[async_trait::async_trait]
impl<S: Send + Sync> FromRequestParts<S> for IdempotencyHeader {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        let Some(raw) = parts.headers.get(IDEMPOTENCY_HEADER) else {
            return Ok(Self(None));
        };
        let raw = raw.to_str().map_err(|_| {
            ApiError::from(pan_africa_pay_domain::error::AppError::validation(
                "Idempotency-Key header must be ASCII",
            ))
        })?;
        let key = IdempotencyKey::parse(raw)?;
        Ok(Self(Some(key)))
    }
}

/// Idempotency operations backed by the repository.
#[derive(Clone)]
pub struct IdempotencyService {
    repo: Arc<dyn IdempotencyRepository>,
}

impl IdempotencyService {
    /// Wrap a repository in a service.
    pub fn new(repo: Arc<dyn IdempotencyRepository>) -> Self {
        Self { repo }
    }

    /// Resolve a key + body hash against the store.
    ///
    /// The storage layer never returns expired records, so a returned
    /// record either replays (matching hash) or conflicts (mismatch).
    pub async fn claim(&self, key: &IdempotencyKey, hash: &RequestHash) -> ApiResult<Claim> {
        let record = self.repo.get(key.as_str()).await.map_err(ApiError::from)?;
        let Some(record) = record else {
            return Ok(Claim::Fresh);
        };
        if record.request_hash == hash.as_str() {
            Ok(Claim::Replay(record))
        } else {
            Ok(Claim::Conflict(record))
        }
    }

    /// Store the completed response for a key.
    ///
    /// A concurrent conflicting write surfaces as a 409 conflict.
    pub async fn complete(
        &self,
        key: &IdempotencyKey,
        hash: &RequestHash,
        response_body: serde_json::Value,
        status_code: u16,
    ) -> ApiResult<()> {
        if let Some(existing) = self
            .repo
            .store(
                key.as_str(),
                hash.as_str(),
                response_body,
                status_code,
                DEFAULT_TTL_SECS,
            )
            .await
            .map_err(ApiError::from)?
        {
            if existing.request_hash != hash.as_str() {
                return Err(ApiError::from(
                    pan_africa_pay_domain::error::AppError::idempotency_conflict(
                        "Idempotency key was used with a different request body",
                    ),
                ));
            }
        }
        Ok(())
    }

    /// Build a stored response with its original status code.
    pub fn replay_response(record: &pan_africa_pay_domain::traits::IdempotencyRecord) -> Response {
        let status = StatusCode::from_u16(record.status_code).unwrap_or(StatusCode::OK);
        (status, Json(record.response_body.clone())).into_response()
    }
}

/// Handle the idempotency claim for a mutating request.
///
/// Returns:
/// - `Ok(Some(response))` when the request is a replay (caller returns
///   the stored response untouched),
/// - `Err(...)` on a key conflict (409),
/// - `Ok(None)` for fresh requests (the handler runs and must call
///   [`IdempotencyService::complete`] with its response on success).
pub async fn claim_or_replay(
    service: &IdempotencyService,
    header: IdempotencyHeader,
    hash: &RequestHash,
) -> ApiResult<Option<Response>> {
    let Some(key) = header.0 else {
        return Ok(None);
    };
    match service.claim(&key, hash).await? {
        Claim::Replay(record) => Ok(Some(IdempotencyService::replay_response(&record))),
        Claim::Conflict(record) => Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::idempotency_conflict(record.key.as_str()),
        )),
        Claim::Fresh => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct FakeRepo {
        records: Mutex<HashMap<String, pan_africa_pay_domain::traits::IdempotencyRecord>>,
    }

    #[async_trait]
    impl IdempotencyRepository for FakeRepo {
        async fn store(
            &self,
            key: &str,
            request_hash: &str,
            response_body: serde_json::Value,
            status_code: u16,
            _ttl_secs: u64,
        ) -> pan_africa_pay_domain::error::AppResult<
            Option<pan_africa_pay_domain::traits::IdempotencyRecord>,
        > {
            let mut records = self.records.lock().expect("lock");
            if let Some(existing) = records.get(key) {
                if existing.request_hash != request_hash {
                    return Ok(Some(existing.clone()));
                }
                return Ok(None);
            }
            records.insert(
                key.to_string(),
                pan_africa_pay_domain::traits::IdempotencyRecord {
                    key: key.to_string(),
                    request_hash: request_hash.to_string(),
                    response_body,
                    status_code,
                },
            );
            Ok(None)
        }

        async fn get(
            &self,
            key: &str,
        ) -> pan_africa_pay_domain::error::AppResult<
            Option<pan_africa_pay_domain::traits::IdempotencyRecord>,
        > {
            Ok(self.records.lock().expect("lock").get(key).cloned())
        }
    }

    fn service() -> IdempotencyService {
        IdempotencyService::new(Arc::new(FakeRepo::default()))
    }

    #[tokio::test]
    async fn claim_is_fresh_on_first_use() {
        let service = service();
        let key = IdempotencyKey::parse("key-1").expect("key");
        let hash = RequestHash::compute_bytes(b"{}");
        assert_eq!(
            service.claim(&key, &hash).await.expect("claim"),
            Claim::Fresh
        );
    }

    #[tokio::test]
    async fn complete_then_claim_replays() {
        let service = service();
        let key = IdempotencyKey::parse("key-1").expect("key");
        let hash = RequestHash::compute_bytes(b"{\"amount\":10}");
        service
            .complete(&key, &hash, serde_json::json!({"data": {"ok": true}}), 200)
            .await
            .expect("complete");

        match service.claim(&key, &hash).await.expect("claim") {
            Claim::Replay(record) => {
                assert_eq!(record.status_code, 200);
                assert_eq!(record.response_body["data"]["ok"], true);
            }
            other => panic!("expected replay, got {other:?}"),
        }
    }

    #[tokio::test]
    async fn same_key_different_body_conflicts() {
        let service = service();
        let key = IdempotencyKey::parse("key-1").expect("key");
        let hash_a = RequestHash::compute_bytes(b"{\"amount\":10}");
        service
            .complete(&key, &hash_a, serde_json::json!({"data": {}}), 200)
            .await
            .expect("complete");

        let hash_b = RequestHash::compute_bytes(b"{\"amount\":20}");
        assert!(matches!(
            service.claim(&key, &hash_b).await.expect("claim"),
            Claim::Conflict(_)
        ));
    }

    #[tokio::test]
    async fn concurrent_conflicting_complete_errors() {
        let service = service();
        let key = IdempotencyKey::parse("key-1").expect("key");
        let hash_a = RequestHash::compute_bytes(b"{\"amount\":10}");
        service
            .complete(&key, &hash_a, serde_json::json!({}), 200)
            .await
            .expect("complete");

        let hash_b = RequestHash::compute_bytes(b"{\"amount\":20}");
        let err = service
            .complete(&key, &hash_b, serde_json::json!({}), 200)
            .await
            .expect_err("conflict expected");
        assert_eq!(
            err.0.code,
            pan_africa_pay_domain::error::ErrorCode::IdempotencyConflict
        );
    }
}
