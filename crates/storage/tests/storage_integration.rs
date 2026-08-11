//! Integration tests for the storage crate against real PostgreSQL and Redis.
//!
//! These tests require the services from `docker-compose.yml` to be
//! running locally. They are skipped when the databases are unreachable
//! so the CI test job degrades gracefully.

use std::time::Duration;

use pan_africa_pay_domain::error::AppResult;
use pan_africa_pay_domain::traits::{
    IdempotencyRepository, PaymentRepository, WalletRepository,
};
use pan_africa_pay_domain::types::{
    Currency, Money, Payment, PaymentId, PaymentStatus, PaymentType, Rail, UserId,
};
use pan_africa_pay_storage::pool::{DatabaseConfig, DatabasePool};
use pan_africa_pay_storage::repositories::Repositories;

/// Build a fresh database pool, applying migrations if needed.
async fn test_pool() -> Option<DatabasePool> {
    let config = DatabaseConfig {
        url: std::env::var("DATABASE_URL")
            .unwrap_or_else(|_| "postgres://postgres:postgres@localhost:5432/pan_africa_pay".to_string()),
        ..DatabaseConfig::default()
    };

    match DatabasePool::connect(&config).await {
        Ok(pool) => {
            let _ = pool.run_migrations().await;
            Some(pool)
        }
        Err(e) => {
            eprintln!("skipping storage integration tests: {e}");
            None
        }
    }
}

fn sample_payment(user_id: UserId) -> Payment {
    Payment {
        id: PaymentId::new(),
        user_id,
        payment_type: PaymentType::Collect,
        rail: Rail::Mpesa,
        status: PaymentStatus::Pending,
        amount: Money::new(10_000, Currency::KES),
        fee: Money::new(100, Currency::KES),
        mpesa_checkout_request_id: Some("ws_CO_123456789".to_string()),
        mpesa_receipt_number: None,
        kotani_tx_id: None,
        callback_payload: None,
        idempotency_key: uuid::Uuid::new_v4().to_string(),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    }
}

#[tokio::test]
async fn payment_repo_crud_round_trip() -> AppResult<()> {
    let Some(pool) = test_pool().await else {
        return Ok(());
    };
    let repos = Repositories::new(pool.pg.clone(), pool.redis.clone());
    let user_id = UserId::new();

    // Fresh user row is required due to the FK constraint.
    sqlx::query(
        "INSERT INTO users (id, email, phone, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id.0)
    .bind(format!("{}@test.local", user_id.0))
    .bind("+254712345678")
    .bind("not-a-real-hash")
    .execute(&pool.pg)
    .await
    .map_err(|e| pan_africa_pay_domain::error::AppError::internal(e.to_string()))?;

    let payment = sample_payment(user_id);
    repos.payments.create_payment(&payment).await?;

    let loaded = repos
        .payments
        .get_payment(payment.id)
        .await?
        .expect("payment should exist after insert");
    assert_eq!(loaded.id, payment.id);
    assert_eq!(loaded.status, PaymentStatus::Pending);
    assert_eq!(loaded.amount, payment.amount);

    let by_key = repos
        .payments
        .get_payment_by_idempotency_key(&payment.idempotency_key)
        .await?
        .expect("payment should be findable by idempotency key");
    assert_eq!(by_key.id, payment.id);

    repos
        .payments
        .update_payment_status(
            payment.id,
            PaymentStatus::Completed,
            Some("PJX1AB2CD3".to_string()),
            None,
        )
        .await?;

    let completed = repos
        .payments
        .get_payment(payment.id)
        .await?
        .expect("payment should still exist");
    assert_eq!(completed.status, PaymentStatus::Completed);
    assert_eq!(
        completed.mpesa_receipt_number.as_deref(),
        Some("PJX1AB2CD3")
    );

    let listed = repos.payments.list_payments_by_user(user_id, 10, 0).await?;
    assert_eq!(listed.len(), 1);

    Ok(())
}

#[tokio::test]
async fn wallet_repo_balance_adjustments_are_atomic() -> AppResult<()> {
    let Some(pool) = test_pool().await else {
        return Ok(());
    };
    let repos = Repositories::new(pool.pg.clone(), pool.redis.clone());
    let user_id = UserId::new();

    sqlx::query(
        "INSERT INTO users (id, email, phone, password_hash) VALUES ($1, $2, $3, $4)",
    )
    .bind(user_id.0)
    .bind(format!("{}@test.local", user_id.0))
    .bind("+254712345678")
    .bind("not-a-real-hash")
    .execute(&pool.pg)
    .await
    .map_err(|e| pan_africa_pay_domain::error::AppError::internal(e.to_string()))?;

    let wallet = repos
        .wallets
        .create_wallet(user_id, Currency::KES)
        .await?;
    assert_eq!(wallet.balance, 0);

    let credited = repos.wallets.adjust_balance(wallet.id, 5_000).await?;
    assert_eq!(credited.balance, 5_000);

    let debited = repos.wallets.adjust_balance(wallet.id, -2_000).await?;
    assert_eq!(debited.balance, 3_000);

    // Overdraft must be rejected.
    let overdraft = repos.wallets.adjust_balance(wallet.id, -10_000).await;
    assert!(overdraft.is_err());

    let reloaded = repos
        .wallets
        .get_wallet(wallet.id)
        .await?
        .expect("wallet should exist");
    assert_eq!(reloaded.balance, 3_000);

    Ok(())
}

#[tokio::test]
async fn idempotency_repo_stores_and_reads_records() -> AppResult<()> {
    let Some(pool) = test_pool().await else {
        return Ok(());
    };
    let repos = Repositories::new(pool.pg.clone(), pool.redis.clone());

    let key = uuid::Uuid::new_v4().to_string();
    let body = serde_json::json!({ "ok": true, "payment_id": "abc" });

    let conflict = repos
        .idempotency
        .store(&key, "hash-v1", body.clone(), 200, 3600)
        .await?;
    assert!(conflict.is_none(), "first store should not conflict");

    let loaded = repos
        .idempotency
        .get(&key)
        .await?
        .expect("record should be readable after store");
    assert_eq!(loaded.request_hash, "hash-v1");
    assert_eq!(loaded.status_code, 200);
    assert_eq!(loaded.response_body, body);

    // Storing the same key with a different hash surfaces a conflict.
    let conflict = repos
        .idempotency
        .store(&key, "hash-v2", body, 200, 3600)
        .await?
        .expect("reusing a key with a different hash must conflict");
    assert_eq!(conflict.request_hash, "hash-v1");

    Ok(())
}

#[tokio::test]
async fn database_pool_health_check_passes() -> AppResult<()> {
    let Some(pool) = test_pool().await else {
        return Ok(());
    };
    tokio::time::timeout(Duration::from_secs(10), pool.health_check())
        .await
        .map_err(|_| pan_africa_pay_domain::error::AppError::service_unavailable("health check timed out"))?
}
