//! PostgreSQL-backed wallet repository.
//!
//! Balance updates are performed atomically with a guarded UPDATE that
//! refuses to move a balance below zero, avoiding read-modify-write
//! races between concurrent transactions.

use async_trait::async_trait;
use sqlx::PgPool;

use pan_africa_pay_domain::error::{AppError, AppResult};
use pan_africa_pay_domain::traits::WalletRepository;
use pan_africa_pay_domain::types::{Currency, UserId, Wallet, WalletId};

use crate::models::WalletRow;

/// SQL adapter for [`WalletRepository`].
pub struct WalletRepo {
    pool: PgPool,
}

impl WalletRepo {
    /// Create a new adapter bound to a connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

/// Enum for the DB-level balance guard failure.
const BALANCE_GUARD_ERROR: &str = "balance_guard";

#[async_trait]
impl WalletRepository for WalletRepo {
    async fn create_wallet(&self, user_id: UserId, currency: Currency) -> AppResult<Wallet> {
        let wallet_id = WalletId::new();
        let row = sqlx::query_as::<_, WalletRow>(
            r#"
            INSERT INTO wallets (id, user_id, currency, balance, created_at, updated_at)
            VALUES ($1, $2, $3, 0, NOW(), NOW())
            RETURNING id, user_id, currency, balance, created_at, updated_at
            "#,
        )
        .bind(wallet_id.0)
        .bind(user_id.0)
        .bind(currency.to_string())
        .fetch_one(&self.pool)
        .await
        .map_err(|e| map_sql_error("create wallet", &e))?;

        row.try_into()
            .map_err(|e| AppError::internal(format!("corrupt wallet row: {e}")))
    }

    async fn get_wallet(&self, id: WalletId) -> AppResult<Option<Wallet>> {
        let row = sqlx::query_as::<_, WalletRow>("SELECT * FROM wallets WHERE id = $1")
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_sql_error("select wallet", &e))?;
        row.map(Wallet::try_from)
            .transpose()
            .map_err(|e| AppError::internal(format!("corrupt wallet row: {e}")))
    }

    async fn get_wallet_by_user_and_currency(
        &self,
        user_id: UserId,
        currency: Currency,
    ) -> AppResult<Option<Wallet>> {
        let row = sqlx::query_as::<_, WalletRow>(
            "SELECT * FROM wallets WHERE user_id = $1 AND currency = $2",
        )
        .bind(user_id.0)
        .bind(currency.to_string())
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_sql_error("select wallet by user and currency", &e))?;
        row.map(Wallet::try_from)
            .transpose()
            .map_err(|e| AppError::internal(format!("corrupt wallet row: {e}")))
    }

    async fn adjust_balance(&self, id: WalletId, delta: i64) -> AppResult<Wallet> {
        // The `AND balance + $2 >= 0` guard makes the update atomic and
        // prevents overdrafts even under concurrency. When the guard
        // fails no row is returned; we translate that into a friendly
        // insufficient-funds error.
        let row = sqlx::query_as::<_, WalletRow>(
            r#"
            UPDATE wallets
            SET balance = balance + $2, updated_at = NOW()
            WHERE id = $1 AND balance + $2 >= 0
            RETURNING id, user_id, currency, balance, created_at, updated_at
            "#,
        )
        .bind(id.0)
        .bind(delta)
        .fetch_optional(&self.pool)
        .await
        .map_err(|e| map_sql_error("adjust wallet balance", &e))?;

        let Some(row) = row else {
            return Err(AppError::insufficient_funds(0, delta.abs()));
        };

        row.try_into()
            .map_err(|e| AppError::internal(format!("corrupt wallet row: {e}")))
    }
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

// Keep the const used for documentation purposes without dead-code
// warnings during Phase 1; it becomes relevant once DB constraints are
// enforced at the schema level.
#[allow(dead_code)]
fn _balance_guard_sentinel() -> &'static str {
    BALANCE_GUARD_ERROR
}
