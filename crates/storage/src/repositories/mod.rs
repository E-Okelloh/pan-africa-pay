//! Repository implementations (adapters) backed by PostgreSQL and Redis.
//!
//! Each module implements a trait from `pan-africa-pay-domain::traits`.

pub mod audit;
pub mod idempotency;
pub mod payment;
pub mod user;
pub mod wallet;

use std::sync::Arc;

use sqlx::PgPool;

/// Bundle of all repository implementations for easy dependency injection.
#[derive(Clone)]
pub struct Repositories {
    /// Payment persistence adapter.
    pub payments: Arc<payment::PaymentRepo>,
    /// Wallet persistence adapter.
    pub wallets: Arc<wallet::WalletRepo>,
    /// User persistence adapter.
    pub users: Arc<user::UserRepo>,
    /// Idempotency record adapter (Redis + PG).
    pub idempotency: Arc<idempotency::IdempotencyRepo>,
    /// Audit log adapter.
    pub audit: Arc<audit::AuditRepo>,
}

impl Repositories {
    /// Construct all adapters from a PostgreSQL pool and Redis pool.
    pub fn new(pg: PgPool, redis_pool: deadpool_redis::Pool) -> Self {
        Self {
            payments: Arc::new(payment::PaymentRepo::new(pg.clone())),
            wallets: Arc::new(wallet::WalletRepo::new(pg.clone())),
            users: Arc::new(user::UserRepo::new(pg.clone())),
            idempotency: Arc::new(idempotency::IdempotencyRepo::new(pg.clone(), redis_pool)),
            audit: Arc::new(audit::AuditRepo::new(pg)),
        }
    }
}
