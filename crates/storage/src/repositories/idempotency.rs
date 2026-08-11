//! Idempotency record repository backed by Redis with a PostgreSQL fallback.
//!
//! Redis provides fast lookups with TTL expiry; PostgreSQL holds a
//! durable backup so responses survive a cache flush. Reads prefer
//! Redis and fall back to PostgreSQL.

use async_trait::async_trait;
use deadpool_redis::Pool as RedisPool;
use sqlx::PgPool;

use pan_africa_pay_domain::error::{AppError, AppResult};
use pan_africa_pay_domain::traits::{IdempotencyRecord, IdempotencyRepository};

/// Redis key prefix for idempotency records.
const REDIS_KEY_PREFIX: &str = "idempotency:";

/// Redis hash field names.
const FIELD_REQUEST_HASH: &str = "request_hash";
const FIELD_RESPONSE_BODY: &str = "response_body";
const FIELD_STATUS_CODE: &str = "status_code";

/// Adapter implementing [`IdempotencyRepository`].
pub struct IdempotencyRepo {
    pg: PgPool,
    redis: RedisPool,
}

impl IdempotencyRepo {
    /// Create a new adapter with a PostgreSQL and Redis pool.
    pub fn new(pg: PgPool, redis: RedisPool) -> Self {
        Self { pg, redis }
    }

    /// Compose the Redis key for an idempotency key.
    fn redis_key(key: &str) -> String {
        format!("{REDIS_KEY_PREFIX}{key}")
    }
}

#[async_trait]
impl IdempotencyRepository for IdempotencyRepo {
    async fn store(
        &self,
        key: &str,
        request_hash: &str,
        response_body: serde_json::Value,
        status_code: u16,
        ttl_secs: u64,
    ) -> AppResult<Option<IdempotencyRecord>> {
        // 1. Check for an existing record. If the hashes differ, this is
        //    a conflict; return the stored record so the caller can reject.
        if let Some(existing) = self.get(key).await? {
            if existing.request_hash != request_hash {
                return Ok(Some(existing));
            }
            return Ok(None);
        }

        // 2. Persist to PostgreSQL first (durable), then Redis (fast).
        sqlx::query(
            r#"
            INSERT INTO idempotency_keys (idempotency_key, request_hash, response_body, status_code, expires_at)
            VALUES ($1, $2, $3, $4, NOW() + make_interval(secs => $5))
            ON CONFLICT (idempotency_key) DO NOTHING
            "#,
        )
        .bind(key)
        .bind(request_hash)
        .bind(&response_body)
        .bind(i32::from(status_code))
        .bind(ttl_secs as i64)
        .execute(&self.pg)
        .await
        .map_err(|e| map_sql_error("store idempotency record", &e))?;

        // 3. Write to Redis with TTL for fast subsequent reads.
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| AppError::internal(format!("redis pool error: {e}")))?;

        let redis_key = Self::redis_key(key);
        let status = i64::from(status_code);
        let body = response_body.to_string();
        let status_str = status.to_string();
        redis::pipe()
            .hset_multiple(
                &redis_key,
                &[
                    (FIELD_REQUEST_HASH, request_hash),
                    (FIELD_RESPONSE_BODY, body.as_str()),
                    (FIELD_STATUS_CODE, status_str.as_str()),
                ],
            )
            .expire(&redis_key, ttl_secs as i64)
            .exec_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis write error: {e}")))?;

        Ok(None)
    }

    async fn get(&self, key: &str) -> AppResult<Option<IdempotencyRecord>> {
        // 1. Try Redis first.
        let mut conn = self
            .redis
            .get()
            .await
            .map_err(|e| AppError::internal(format!("redis pool error: {e}")))?;

        let redis_key = Self::redis_key(key);
        let exists: bool = redis::cmd("EXISTS")
            .arg(&redis_key)
            .query_async(&mut conn)
            .await
            .map_err(|e| AppError::internal(format!("redis read error: {e}")))?;

        if exists {
            let (hash, body, status): (String, String, i64) = redis::cmd("HMGET")
                .arg(&redis_key)
                .arg(FIELD_REQUEST_HASH)
                .arg(FIELD_RESPONSE_BODY)
                .arg(FIELD_STATUS_CODE)
                .query_async(&mut conn)
                .await
                .map_err(|e| AppError::internal(format!("redis read error: {e}")))?;

            let response_body = serde_json::from_str(&body)
                .map_err(|e| AppError::internal(format!("corrupt redis idempotency body: {e}")))?;

            return Ok(Some(IdempotencyRecord {
                key: key.to_string(),
                request_hash: hash,
                response_body,
                status_code: status as u16,
            }));
        }

        // 2. Fall back to PostgreSQL.
        let row: Option<(String, String, serde_json::Value, i32)> = sqlx::query_as(
            r#"
            SELECT idempotency_key, request_hash, response_body, status_code
            FROM idempotency_keys
            WHERE idempotency_key = $1 AND expires_at > NOW()
            "#,
        )
        .bind(key)
        .fetch_optional(&self.pg)
        .await
        .map_err(|e| map_sql_error("load idempotency record", &e))?;

        match row {
            Some((_, request_hash, response_body, status_code)) => {
                // Rehydrate Redis so subsequent reads stay fast.
                let mut conn = self
                    .redis
                    .get()
                    .await
                    .map_err(|e| AppError::internal(format!("redis pool error: {e}")))?;
                let status = i64::from(status_code);
                redis::pipe()
                    .hset_multiple(
                        &redis_key,
                        &[
                            (FIELD_REQUEST_HASH, request_hash.as_str()),
                            (FIELD_RESPONSE_BODY, response_body.to_string().as_str()),
                            (FIELD_STATUS_CODE, status.to_string().as_str()),
                        ],
                    )
                    .expire(&redis_key, 86_400)
                    .exec_async(&mut conn)
                    .await
                    .map_err(|e| AppError::internal(format!("redis write error: {e}")))?;

                Ok(Some(IdempotencyRecord {
                    key: key.to_string(),
                    request_hash,
                    response_body,
                    status_code: status_code as u16,
                }))
            }
            None => Ok(None),
        }
    }
}

/// Translate common sqlx errors into domain errors.
fn map_sql_error(action: &str, err: &sqlx::Error) -> AppError {
    AppError::internal(format!("{action}: {err}"))
}
