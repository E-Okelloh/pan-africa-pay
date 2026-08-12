//! PostgreSQL-backed user repository.

use async_trait::async_trait;
use sqlx::PgPool;

use pan_africa_pay_domain::error::{AppError, AppResult};
use pan_africa_pay_domain::traits::UserRepository;
use pan_africa_pay_domain::types::{User, UserId};

/// SQL adapter for [`UserRepository`].
pub struct UserRepo {
    pool: PgPool,
}

impl UserRepo {
    /// Create a new adapter bound to a connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl UserRepository for UserRepo {
    async fn create_user(&self, user: &User) -> AppResult<()> {
        sqlx::query(
            r#"
            INSERT INTO users (id, email, phone, password_hash, kyc_tier)
            VALUES ($1, $2, $3, '', 'NONE')
            ON CONFLICT (id) DO NOTHING
            "#,
        )
        .bind(user.id.0)
        .bind(&user.email)
        .bind(user.phone.as_str())
        .execute(&self.pool)
        .await
        .map_err(|e| map_sql_error("insert user", &e))?;
        Ok(())
    }

    async fn get_user(&self, id: UserId) -> AppResult<Option<User>> {
        let row: Option<(uuid::Uuid, String, String, chrono::DateTime<chrono::Utc>)> =
            sqlx::query_as(
                r#"
                SELECT id, email, phone, created_at
                FROM users
                WHERE id = $1
                "#,
            )
            .bind(id.0)
            .fetch_optional(&self.pool)
            .await
            .map_err(|e| map_sql_error("select user", &e))?;

        row.map(|(id, email, phone, created_at)| {
            let phone = pan_africa_pay_domain::types::PhoneNumber::new(&phone)
                .map_err(|e| AppError::internal(format!("corrupt user phone: {e}")))?;
            Ok(User {
                id: UserId(id),
                email,
                phone,
                created_at,
                updated_at: created_at,
            })
        })
        .transpose()
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
