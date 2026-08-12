//! Shared application state passed to route handlers.

use std::sync::Arc;

use pan_africa_pay_domain::traits::{EventPublisher, PaymentRepository, UserRepository};
use pan_africa_pay_kotani::KotaniClient;
use pan_africa_pay_mpesa::MpesaClient;
use pan_africa_pay_storage::repositories::audit::AuditRepo;
use pan_africa_pay_storage::repositories::idempotency::IdempotencyRepo;
use pan_africa_pay_storage::repositories::payment::PaymentRepo;
use pan_africa_pay_storage::repositories::user::UserRepo;
use pan_africa_pay_storage::DatabasePool;

use crate::config::AppConfig;
use crate::idempotency::IdempotencyService;

/// Shared application state.
#[derive(Clone)]
pub struct AppState {
    /// Immutable application configuration.
    pub config: Arc<AppConfig>,
    /// Database and cache pools.
    pub pool: DatabasePool,
    /// M-Pesa Daraja client, present when `MPESA_*` is configured.
    pub mpesa: Option<MpesaClient>,
    /// Kotani Pay client, present when `KOTANI_*` is configured.
    pub kotani: Option<KotaniClient>,
    /// Idempotency service backed by Redis + PostgreSQL.
    pub idempotency: IdempotencyService,
    /// Payment persistence.
    pub payments: Arc<dyn PaymentRepository>,
    /// User persistence.
    pub users: Arc<dyn UserRepository>,
    /// Audit event sink (append-only audit log).
    pub events: Arc<dyn EventPublisher>,
    /// Audit log reader.
    pub audit: Arc<pan_africa_pay_storage::repositories::audit::AuditRepo>,
}

impl AppState {
    /// Build state from configuration, establishing pools eagerly.
    pub async fn new(config: AppConfig) -> anyhow::Result<Self> {
        let pool = DatabasePool::connect(&config.database)
            .await
            .map_err(|e| anyhow::anyhow!("failed to connect to stores: {e}"))?;
        let mpesa = if config.mpesa.is_configured() {
            Some(
                MpesaClient::from_config(config.mpesa.clone())
                    .map_err(|e| anyhow::anyhow!("invalid M-Pesa config: {e}"))?,
            )
        } else {
            None
        };
        let kotani = if config.kotani.is_configured() {
            Some(
                KotaniClient::from_config(config.kotani.clone())
                    .map_err(|e| anyhow::anyhow!("invalid Kotani config: {e}"))?,
            )
        } else {
            None
        };
        let idempotency = IdempotencyService::new(Arc::new(IdempotencyRepo::new(
            pool.pg.clone(),
            pool.redis.clone(),
        )));
        let payments = Arc::new(PaymentRepo::new(pool.pg.clone()));
        let users = Arc::new(UserRepo::new(pool.pg.clone()));
        let audit = Arc::new(AuditRepo::new(pool.pg.clone()));
        let events: Arc<dyn EventPublisher> = audit.clone();
        Ok(Self {
            config: Arc::new(config),
            pool,
            mpesa,
            kotani,
            idempotency,
            payments,
            users,
            events,
            audit,
        })
    }

    /// Apply pending database migrations.
    pub async fn run_migrations(&self) -> anyhow::Result<()> {
        self.pool
            .run_migrations()
            .await
            .map_err(|e| anyhow::anyhow!("migration failed: {e}"))
    }
}
