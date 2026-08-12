//! Reconciliation sweeper.
//!
//! Webhooks reconcile payments when providers deliver callbacks, but a
//! missed callback (network blip, provider outage) leaves a payment
//! stuck in `PENDING`/`PROCESSING` forever. The sweeper polls provider
//! status endpoints for stale payments and settles them:
//!
//! - M-Pesa rail: `stk_query(checkout_request_id)`
//! - Kotani rail: `deposit_status(payment_id)`
//!
//! The sweeper is deliberately conservative: it only flips a payment
//! to a terminal state when the provider explicitly reports an outcome,
//! and it logs everything for audit.

use std::sync::Arc;
use std::time::Duration;

use tokio::time::{interval, MissedTickBehavior};
use tracing::{debug, error, info, warn};

use pan_africa_pay_domain::traits::PaymentRepository;
use pan_africa_pay_domain::types::{Payment, PaymentStatus, Rail};
use pan_africa_pay_kotani::KotaniClient;
use pan_africa_pay_mpesa::MpesaClient;

/// Statuses the sweeper looks for.
const STALE_STATUSES: &[PaymentStatus] = &[PaymentStatus::Pending, PaymentStatus::Processing];

/// Default sweep cadence.
pub const DEFAULT_SWEEP_INTERVAL_SECS: u64 = 60;

/// Default age threshold before a payment is considered stale.
pub const DEFAULT_STALE_MINUTES: i64 = 10;

/// Batch size per sweep.
const SWEEP_BATCH: i64 = 100;

/// Periodic reconciliation of stale payments.
#[derive(Clone)]
pub struct ReconciliationSweeper {
    payments: Arc<dyn PaymentRepository>,
    mpesa: Option<MpesaClient>,
    kotani: Option<KotaniClient>,
    interval_secs: u64,
    stale_minutes: i64,
}

impl ReconciliationSweeper {
    /// Build a sweeper from repositories and provider clients.
    pub fn new(
        payments: Arc<dyn PaymentRepository>,
        mpesa: Option<MpesaClient>,
        kotani: Option<KotaniClient>,
        interval_secs: u64,
        stale_minutes: i64,
    ) -> Self {
        Self {
            payments,
            mpesa,
            kotani,
            interval_secs,
            stale_minutes,
        }
    }

    /// Run the sweep loop forever (used by the server bootstrap).
    pub async fn run(&self) {
        let mut ticker = interval(Duration::from_secs(self.interval_secs.max(1)));
        ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);
        loop {
            ticker.tick().await;
            if let Err(err) = self.sweep_once().await {
                error!(component = "reconciliation", "sweep failed: {err}");
            }
        }
    }

    /// Run one reconciliation pass over stale payments.
    pub async fn sweep_once(&self) -> Result<usize, Box<dyn std::error::Error + Send + Sync>> {
        let stale = self
            .payments
            .list_payments_for_reconciliation(STALE_STATUSES, self.stale_minutes, SWEEP_BATCH)
            .await
            .map_err(|e| format!("failed to list stale payments: {e}"))?;

        if stale.is_empty() {
            return Ok(0);
        }
        info!(
            component = "reconciliation",
            count = stale.len(),
            "reconciling stale payments"
        );

        let mut settled = 0;
        for payment in stale {
            match self.reconcile(&payment).await {
                Ok(true) => settled += 1,
                Ok(false) => {}
                Err(err) => {
                    warn!(
                        component = "reconciliation",
                        payment_id = %payment.id,
                        "reconcile skipped: {err}"
                    );
                }
            }
        }
        Ok(settled)
    }

    /// Reconcile a single payment. Returns `true` when it was settled.
    async fn reconcile(&self, payment: &Payment) -> Result<bool, String> {
        match payment.rail {
            Rail::Mpesa => self.reconcile_mpesa(payment).await,
            Rail::Kotani => self.reconcile_kotani(payment).await,
        }
    }

    async fn reconcile_mpesa(&self, payment: &Payment) -> Result<bool, String> {
        let Some(client) = &self.mpesa else {
            debug!(payment_id = %payment.id, "mpesa client not configured; skipping");
            return Ok(false);
        };
        let Some(checkout_request_id) = &payment.mpesa_checkout_request_id else {
            return Ok(false);
        };

        let result = client
            .stk_query(checkout_request_id)
            .await
            .map_err(|e| format!("stk_query failed: {e}"))?;

        let status = match result.result_code.as_deref() {
            Some("0") => {
                info!(
                    component = "reconciliation",
                    payment_id = %payment.id,
                    "M-Pesa confirmed payment"
                );
                PaymentStatus::Completed
            }
            Some(code) => {
                warn!(
                    component = "reconciliation",
                    payment_id = %payment.id,
                    result_code = code,
                    "M-Pesa reported failure"
                );
                PaymentStatus::Failed
            }
            None => return Ok(false),
        };

        self.payments
            .update_payment_status(payment.id, status, None, None)
            .await
            .map_err(|e| format!("status update failed: {e}"))?;
        Ok(true)
    }

    async fn reconcile_kotani(&self, payment: &Payment) -> Result<bool, String> {
        let Some(client) = &self.kotani else {
            debug!(payment_id = %payment.id, "kotani client not configured; skipping");
            return Ok(false);
        };

        let reference = payment.id.to_string();
        let result = client
            .deposit_status(&reference)
            .await
            .map_err(|e| format!("deposit_status failed: {e}"))?;

        let outcome = result
            .status
            .as_deref()
            .or(result.message.as_deref())
            .unwrap_or_default()
            .to_lowercase();

        let status = if outcome.contains("success") || outcome.contains("complete") {
            info!(
                component = "reconciliation",
                payment_id = %payment.id,
                "Kotani confirmed payment"
            );
            PaymentStatus::Completed
        } else if outcome.contains("fail")
            || outcome.contains("cancel")
            || outcome.contains("error")
        {
            warn!(
                component = "reconciliation",
                payment_id = %payment.id,
                "Kotani reported failure: {outcome}"
            );
            PaymentStatus::Failed
        } else {
            // Still in flight at the provider; leave for the next pass.
            return Ok(false);
        };

        self.payments
            .update_payment_status(payment.id, status, None, None)
            .await
            .map_err(|e| format!("status update failed: {e}"))?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use chrono::Utc;
    use std::collections::HashMap;
    use std::sync::Mutex;

    use pan_africa_pay_domain::error::AppResult;
    use pan_africa_pay_domain::types::{Currency, Money, PaymentId, PaymentType, UserId};

    #[derive(Default)]
    struct FakePayments {
        records: Mutex<HashMap<PaymentId, Payment>>,
        statuses: Mutex<Vec<PaymentStatus>>,
    }

    #[async_trait]
    impl PaymentRepository for FakePayments {
        async fn create_payment(&self, payment: &Payment) -> AppResult<()> {
            self.records
                .lock()
                .unwrap()
                .insert(payment.id, payment.clone());
            Ok(())
        }
        async fn get_payment(&self, id: PaymentId) -> AppResult<Option<Payment>> {
            Ok(self.records.lock().unwrap().get(&id).cloned())
        }
        async fn get_payment_by_idempotency_key(&self, _key: &str) -> AppResult<Option<Payment>> {
            Ok(None)
        }
        async fn get_payment_by_mpesa_checkout_request_id(
            &self,
            _checkout_request_id: &str,
        ) -> AppResult<Option<Payment>> {
            Ok(None)
        }
        async fn update_payment_status(
            &self,
            _id: PaymentId,
            status: PaymentStatus,
            _mpesa_receipt_number: Option<String>,
            _kotani_tx_id: Option<String>,
        ) -> AppResult<()> {
            self.statuses.lock().unwrap().push(status);
            Ok(())
        }
        async fn attach_callback_payload(
            &self,
            _id: PaymentId,
            _payload: serde_json::Value,
        ) -> AppResult<()> {
            Ok(())
        }
        async fn list_payments_by_user(
            &self,
            _user_id: UserId,
            _limit: i64,
            _offset: i64,
        ) -> AppResult<Vec<Payment>> {
            Ok(vec![])
        }
        async fn list_payments_for_reconciliation(
            &self,
            statuses: &[PaymentStatus],
            _stale_minutes: i64,
            _limit: i64,
        ) -> AppResult<Vec<Payment>> {
            Ok(self
                .records
                .lock()
                .unwrap()
                .values()
                .filter(|p| statuses.contains(&p.status))
                .cloned()
                .collect())
        }
    }

    fn payment(rail: Rail, status: PaymentStatus) -> Payment {
        Payment {
            id: PaymentId::new(),
            user_id: UserId::new(),
            payment_type: PaymentType::Collect,
            rail,
            status,
            amount: Money {
                amount: 1000,
                currency: Currency::KES,
            },
            fee: Money {
                amount: 0,
                currency: Currency::KES,
            },
            mpesa_checkout_request_id: Some("ws_CO_RECON".to_string()),
            mpesa_receipt_number: None,
            kotani_tx_id: None,
            callback_payload: None,
            idempotency_key: "key".to_string(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[tokio::test]
    async fn sweeper_without_providers_skips_everything() {
        let payments = Arc::new(FakePayments::default());
        payments.records.lock().unwrap().insert(
            PaymentId::new(),
            payment(Rail::Mpesa, PaymentStatus::Pending),
        );

        let sweeper = ReconciliationSweeper::new(payments.clone(), None, None, 60, 10);
        let settled = sweeper.sweep_once().await.expect("sweep");
        assert_eq!(settled, 0);
        assert!(payments.statuses.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn mpesa_sweep_settles_completed_payment() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/oauth/v1/generate"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "access_token": "token-1",
                "expires_in": 3599
            })))
            .mount(&server)
            .await;
        Mock::given(method("POST"))
            .and(path("/mpesa/stkpushquery/v1/query"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "ResponseCode": "0",
                "ResponseDescription": "The service request is processed successfully.",
                "MerchantRequestID": "m1",
                "CheckoutRequestID": "ws_CO_RECON",
                "ResultCode": "0",
                "ResultDesc": "The service request is processed successfully."
            })))
            .mount(&server)
            .await;

        let client = pan_africa_pay_mpesa::MpesaClient::from_config(
            pan_africa_pay_mpesa::config::MpesaConfig {
                consumer_key: "ck".to_string(),
                consumer_secret: "cs".to_string(),
                passkey: "pk".to_string(),
                short_code: "174379".to_string(),
                callback_url: "https://example.com/webhooks/mpesa".to_string(),
                environment: pan_africa_pay_mpesa::config::Environment::Sandbox,
                timeout_secs: 30,
                token_ttl_secs: 3600,
                base_url_override: server.uri(),
            },
        )
        .expect("client");

        let payments = Arc::new(FakePayments::default());
        let pending = payment(Rail::Mpesa, PaymentStatus::Pending);
        payments.records.lock().unwrap().insert(pending.id, pending);

        let sweeper = ReconciliationSweeper::new(payments.clone(), Some(client), None, 60, 10);
        let settled = sweeper.sweep_once().await.expect("sweep");
        assert_eq!(settled, 1);
        let statuses = payments.statuses.lock().unwrap();
        assert_eq!(statuses.as_slice(), &[PaymentStatus::Completed]);
    }

    #[tokio::test]
    async fn kotani_sweep_settles_failed_payment() {
        use wiremock::matchers::{bearer_token, method};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path_contains("/api/v3/deposit/mobile-money/status/"))
            .and(bearer_token("key"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "success": true,
                "message": "status",
                "data": {
                    "status": "failed",
                    "message": "Insufficient funds",
                    "reference_id": "some-ref"
                }
            })))
            .mount(&server)
            .await;

        let client =
            pan_africa_pay_kotani::KotaniClient::from_config(pan_africa_pay_kotani::KotaniConfig {
                api_key: "key".to_string(),
                api_secret: "secret".to_string(),
                base_url: server.uri(),
                webhook_secret: "whsec".to_string(),
                callback_url: "https://example.com/webhooks/kotani".to_string(),
                wallet_id: "wallet-1".to_string(),
                timeout_secs: 30,
            })
            .expect("client");

        let payments = Arc::new(FakePayments::default());
        let pending = payment(Rail::Kotani, PaymentStatus::Processing);
        payments.records.lock().unwrap().insert(pending.id, pending);

        let sweeper = ReconciliationSweeper::new(payments.clone(), None, Some(client), 60, 10);
        let settled = sweeper.sweep_once().await.expect("sweep");
        assert_eq!(settled, 1);
        let statuses = payments.statuses.lock().unwrap();
        assert_eq!(statuses.as_slice(), &[PaymentStatus::Failed]);
    }

    /// Match a path by prefix (status endpoints embed a dynamic id).
    fn path_contains(substr: &'static str) -> impl wiremock::Match {
        struct Contains(&'static str);
        impl wiremock::Match for Contains {
            fn matches(&self, request: &wiremock::Request) -> bool {
                request.url.path().contains(self.0)
            }
        }
        Contains(substr)
    }
}
