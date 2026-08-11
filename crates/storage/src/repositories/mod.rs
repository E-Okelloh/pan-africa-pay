//! Repository implementations (adapters) backed by PostgreSQL and Redis.
//!
//! Each module implements a trait from `pan-africa-pay-domain::traits`.

pub mod idempotency;
pub mod payment;
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
    /// Idempotency record adapter (Redis + PG).
    pub idempotency: Arc<idempotency::IdempotencyRepo>,
}

impl Repositories {
    /// Construct all adapters from a PostgreSQL pool and Redis pool.
    pub fn new(pg: PgPool, redis_pool: deadpool_redis::Pool) -> Self {
        Self {
            payments: Arc::new(payment::PaymentRepo::new(pg.clone())),
            wallets: Arc::new(wallet::WalletRepo::new(pg.clone())),
            idempotency: Arc::new(idempotency::IdempotencyRepo::new(pg, redis_pool)),
        }
    }
}
