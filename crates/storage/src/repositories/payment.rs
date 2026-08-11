//! PostgreSQL-backed payment repository.

use async_trait::async_trait;
use sqlx::PgPool;

use pan_africa_pay_domain::error::{AppError, AppResult};
use pan_africa_pay_domain::traits::PaymentRepository;
use pan_africa_pay_domain::types::{Payment, PaymentId, PaymentStatus, UserId};

use crate::models::PaymentRow;

/// SQL adapter for [`PaymentRepository`].
pub struct PaymentRepo {
    pool: PgPool,
}

impl PaymentRepo {
    /// Create a new adapter bound to a connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl PaymentRepository for PaymentRepo {
    async fn create_payment(&self, payment: &Payment) -> AppResult<()> {
        let row = PaymentRow::from(payment);
        sqlx::query(
            r#"
            INSERT INTO payments (
                id, user_id, payment_type, rail, status, amount, currency, fee,
                mpesa_checkout_request_id, mpesa_receipt_number, kotani_tx_id,
                callback_payload, idempotency_key, created_at, updated_at
            )
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15)
            "#,
        )
        .bind(row.id)
        .bind(row.user_id)
        .bind(row.payment_type)
        .bind(row.rail)
        .bind(row.status)
        .bind(row.amount)
        .bind(row.currency)
        .bind(row.fee)
        .bind(row.mpesa_checkout_request_id)
        .bind(row.mpesa_receipt_number)
        .bind(row.kotani_tx_id)
        .bind(row.callback_payload)
        .bind(row.idempotency_key)
        .bind(row.created_at)
        .bind(row.updated_at)
        .execute(&self.pool)
        .await
        .map_err(|e| map_sql_error("insert payment", &e))?;
        Ok(())
    }

    async fn get_payment(&self, id: PaymentId) -> AppResult<Option<Payment>> {
        let row = sqlx::query_as::<_, PaymentRow>("SELECT * FROM payments WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_sql_error("select payment", &e))?;
        row.map(Payment::try_from)
            .transpose()
            .map_err(|e| AppError::internal(format!("corrupt payment row: {e}")))
    }

    async fn get_payment_by_idempotency_key(&self, key: &str) -> AppResult<Option<Payment>> {
        let row = sqlx::query_as::<_, PaymentRow>(
            "SELECT * FROM payments WHERE idempotency_key = $1",
        )
        .bind(key)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_sql_error("select payment by idempotency key", &e))?;
        row.map(Payment::try_from)
            .transpose()
            .map_err(|e| AppError::internal(format!("corrupt payment row: {e}")))
    }

    async fn update_payment_status(
        &self,
        id: PaymentId,
        status: PaymentStatus,
        mpesa_receipt_number: Option<String>,
        kotani_tx_id: Option<String>,
    ) -> AppResult<()> {
        let status_str = serde_enum(&status);
        sqlx::query(
            r#"
            UPDATE payments
            SET status = $2,
                mpesa_receipt_number = COALESCE($3, mpesa_receipt_number),
                kotani_tx_id = COALESCE($4, kotani_tx_id),
                updated_at = NOW()
            WHERE id = $1
            "#,
        )
        .bind(id.0)
        .bind(status_str)
        .bind(mpesa_receipt_number)
        .bind(kotani_tx_id)
        .execute(&self.pool)
        .await
        .map_err(|e| map_sql_error("update payment status", &e))?;
        Ok(())
    }

    async fn attach_callback_payload(&self, id: PaymentId, payload: serde_json::Value) -> AppResult<()> {
        sqlx::query("UPDATE payments SET callback_payload = $2, updated_at = NOW() WHERE id = $1")
            .bind(id.0)
            .bind(payload)
            .execute(&self.pool)
            .await
            .map_err(|e| map_sql_error("attach callback payload", &e))?;
        Ok(())
    }

    async fn list_payments_by_user(&self, user_id: UserId, limit: i64, offset: i64) -> AppResult<Vec<Payment>> {
        let rows = sqlx::query_as::<_, PaymentRow>(
            r#"
            SELECT * FROM payments
            WHERE user_id = $1
            ORDER BY created_at DESC
            LIMIT $2 OFFSET $3
            "#,
        )
        .bind(user_id.0)
        .bind(limit)
        .bind(offset)
        .fetch_all(&self.pool)
        .await
        .map_err(|e| map_sql_error("list payments by user", &e))?;

        rows.into_iter()
            .map(Payment::try_from)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::internal(format!("corrupt payment row: {e}")))
    }
}

/// Serialize a domain enum to its SCREAMING_SNAKE_CASE string form.
fn serde_enum<T>(value: &T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_string(value)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}

/// Translate common sqlx errors into domain errors.
fn map_sql_error(action: &str, err: &sqlx::Error) -> AppError {
    match err {
        sqlx::Error::Database(db) => {
            if let Some(code) = db.code() {
                // 23505: unique_violation
                if code == "23505" {
                    return AppError::conflict(format!("{action}: duplicate record"));
                }
            }
            AppError::internal(format!("{action}: database error: {db}"))
        }
        _ => AppError::internal(format!("{action}: {err}")),
    }
}
