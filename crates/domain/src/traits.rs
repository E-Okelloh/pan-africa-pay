//! Repository traits (ports) for the storage layer.
//!
//! These traits define what the application needs from persistence,
//! without coupling the domain to any specific database. The `storage`
//! crate implements these traits against PostgreSQL/Redis; tests use
//! in-memory fakes.

use async_trait::async_trait;
use uuid::Uuid;

use crate::error::AppResult;
use crate::events::DomainEvent;
use crate::types::{Payment, PaymentId, PaymentStatus, User, UserId, Wallet, WalletId};

/// Persistence for payments.
#[async_trait]
pub trait PaymentRepository: Send + Sync {
    /// Persist a new payment.
    async fn create_payment(&self, payment: &Payment) -> AppResult<()>;

    /// Load a payment by id.
    async fn get_payment(&self, id: PaymentId) -> AppResult<Option<Payment>>;

    /// Load a payment by its idempotency key.
    async fn get_payment_by_idempotency_key(&self, key: &str) -> AppResult<Option<Payment>>;

    /// Load a payment by its M-Pesa checkout request id.
    async fn get_payment_by_mpesa_checkout_request_id(
        &self,
        checkout_request_id: &str,
    ) -> AppResult<Option<Payment>>;

    /// Update a payment's status, optionally attaching provider references.
    async fn update_payment_status(
        &self,
        id: PaymentId,
        status: PaymentStatus,
        mpesa_receipt_number: Option<String>,
        kotani_tx_id: Option<String>,
    ) -> AppResult<()>;

    /// Attach the provider callback payload to a payment (for audit).
    async fn attach_callback_payload(
        &self,
        id: PaymentId,
        payload: serde_json::Value,
    ) -> AppResult<()>;

    /// List payments for a user, most recent first.
    async fn list_payments_by_user(
        &self,
        user_id: UserId,
        limit: i64,
        offset: i64,
    ) -> AppResult<Vec<Payment>>;
}

/// Persistence for platform users.
#[async_trait]
pub trait UserRepository: Send + Sync {
    /// Create a new user.
    async fn create_user(&self, user: &User) -> AppResult<()>;

    /// Load a user by id.
    async fn get_user(&self, id: UserId) -> AppResult<Option<User>>;
}

/// Persistence for wallets.
#[async_trait]
pub trait WalletRepository: Send + Sync {
    /// Create a wallet for a user and currency.
    async fn create_wallet(
        &self,
        user_id: UserId,
        currency: crate::types::Currency,
    ) -> AppResult<Wallet>;

    /// Load a wallet by id.
    async fn get_wallet(&self, id: WalletId) -> AppResult<Option<Wallet>>;

    /// Load a user's wallet in a currency.
    async fn get_wallet_by_user_and_currency(
        &self,
        user_id: UserId,
        currency: crate::types::Currency,
    ) -> AppResult<Option<Wallet>>;

    /// Atomically apply a signed delta to a wallet balance.
    ///
    /// Returns the wallet with its updated balance. Fails if the
    /// resulting balance would go negative.
    async fn adjust_balance(&self, id: WalletId, delta: i64) -> AppResult<Wallet>;
}

/// Persistence for idempotency records.
#[async_trait]
pub trait IdempotencyRepository: Send + Sync {
    /// Store a completed response for an idempotency key.
    ///
    /// If the key already exists with a different request hash, the
    /// insert fails so callers can detect conflicts.
    async fn store(
        &self,
        key: &str,
        request_hash: &str,
        response_body: serde_json::Value,
        status_code: u16,
        ttl_secs: u64,
    ) -> AppResult<Option<IdempotencyRecord>>;

    /// Load the stored response for a key, if any.
    async fn get(&self, key: &str) -> AppResult<Option<IdempotencyRecord>>;
}

/// A stored idempotency response.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct IdempotencyRecord {
    pub key: String,
    pub request_hash: String,
    pub response_body: serde_json::Value,
    pub status_code: u16,
}

/// Publishes domain events to a message bus or log sink.
#[async_trait]
pub trait EventPublisher: Send + Sync {
    /// Publish a domain event.
    async fn publish(&self, event: &DomainEvent) -> AppResult<()>;

    /// Publish several domain events atomically (best-effort).
    async fn publish_many(&self, events: &[DomainEvent]) -> AppResult<()> {
        for event in events {
            self.publish(event).await?;
        }
        Ok(())
    }
}

/// Convenience trait implemented by UUID types used as primary keys.
pub trait Keyed {
    fn as_uuid(&self) -> Uuid;
}

impl Keyed for PaymentId {
    fn as_uuid(&self) -> Uuid {
        self.0
    }
}

impl Keyed for WalletId {
    fn as_uuid(&self) -> Uuid {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// In-memory fakes used by domain/service tests.
    mod fakes {
        use super::*;
        use crate::error::AppError;

        pub struct InMemoryPaymentRepo {
            pub payments: std::sync::Mutex<std::collections::HashMap<PaymentId, Payment>>,
        }

        #[async_trait]
        impl PaymentRepository for InMemoryPaymentRepo {
            async fn create_payment(&self, payment: &Payment) -> AppResult<()> {
                self.payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .insert(payment.id, payment.clone());
                Ok(())
            }

            async fn get_payment(&self, id: PaymentId) -> AppResult<Option<Payment>> {
                Ok(self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .get(&id)
                    .cloned())
            }

            async fn get_payment_by_idempotency_key(
                &self,
                key: &str,
            ) -> AppResult<Option<Payment>> {
                Ok(self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .values()
                    .find(|p| p.idempotency_key == key)
                    .cloned())
            }

            async fn get_payment_by_mpesa_checkout_request_id(
                &self,
                checkout_request_id: &str,
            ) -> AppResult<Option<Payment>> {
                Ok(self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .values()
                    .find(|p| p.mpesa_checkout_request_id.as_deref() == Some(checkout_request_id))
                    .cloned())
            }

            async fn update_payment_status(
                &self,
                id: PaymentId,
                status: PaymentStatus,
                mpesa_receipt_number: Option<String>,
                kotani_tx_id: Option<String>,
            ) -> AppResult<()> {
                if let Some(p) = self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .get_mut(&id)
                {
                    p.status = status;
                    p.mpesa_receipt_number = mpesa_receipt_number;
                    p.kotani_tx_id = kotani_tx_id;
                }
                Ok(())
            }

            async fn attach_callback_payload(
                &self,
                id: PaymentId,
                payload: serde_json::Value,
            ) -> AppResult<()> {
                if let Some(p) = self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .get_mut(&id)
                {
                    p.callback_payload = Some(payload);
                }
                Ok(())
            }

            async fn list_payments_by_user(
                &self,
                _user_id: UserId,
                limit: i64,
                _offset: i64,
            ) -> AppResult<Vec<Payment>> {
                Ok(self
                    .payments
                    .lock()
                    .map_err(|e| AppError::internal(format!("fake repo lock poisoned: {e}")))?
                    .values()
                    .take(limit as usize)
                    .cloned()
                    .collect())
            }
        }
    }

    use crate::types::{Currency, Money, Payment, PaymentType, Rail};
    use fakes::InMemoryPaymentRepo;

    #[test]
    fn in_memory_fake_round_trips_payments() {
        let rt = tokio::runtime::Runtime::new().expect("runtime");
        rt.block_on(async {
            let repo = InMemoryPaymentRepo {
                payments: std::sync::Mutex::new(std::collections::HashMap::new()),
            };
            let payment = Payment {
                id: PaymentId::new(),
                user_id: UserId::new(),
                payment_type: PaymentType::Collect,
                rail: Rail::Mpesa,
                status: PaymentStatus::Pending,
                amount: Money::new(10_000, Currency::KES),
                fee: Money::zero(Currency::KES),
                mpesa_checkout_request_id: None,
                mpesa_receipt_number: None,
                kotani_tx_id: None,
                callback_payload: None,
                idempotency_key: "key-1".to_string(),
                created_at: chrono::Utc::now(),
                updated_at: chrono::Utc::now(),
            };
            repo.create_payment(&payment).await.expect("create");
            let loaded = repo.get_payment(payment.id).await.expect("get");
            assert_eq!(loaded, Some(payment.clone()));
            let by_key = repo
                .get_payment_by_idempotency_key("key-1")
                .await
                .expect("by key");
            assert_eq!(by_key, Some(payment));
        });
    }
}
