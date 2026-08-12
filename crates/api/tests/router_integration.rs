//! Router integration tests.
//!
//! These exercise the full middleware stack (tracing, panic catching)
//! against a router whose state holds lazily-created pools. Endpoints
//! that require live databases (readiness) are covered by the storage
//! integration suite; here we verify routing, envelopes, and status
//! codes without external services.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use pan_africa_pay_api::config::{AppConfig, Environment, LoggingConfig, ServerConfig};
use pan_africa_pay_api::idempotency::IdempotencyService;
use pan_africa_pay_api::routes::build_router;
use pan_africa_pay_api::state::AppState;
use pan_africa_pay_storage::DatabasePool;

/// State with lazy pools: no connection is attempted until an
/// endpoint actually queries a store.
fn test_state() -> AppState {
    let config = AppConfig {
        env: Environment::Test,
        server: ServerConfig {
            host: "127.0.0.1".to_string(),
            port: 0,
        },
        database: pan_africa_pay_storage::DatabaseConfig::default(),
        logging: LoggingConfig::default(),
        ..AppConfig::default()
    };
    let pg = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy(&config.database.url)
        .expect("lazy pg pool");
    let redis = deadpool_redis::Manager::new(config.database.redis_url.clone())
        .map(|manager| {
            deadpool_redis::Pool::builder(manager)
                .max_size(1)
                .build()
                .expect("redis pool")
        })
        .expect("redis manager");
    AppState {
        config: std::sync::Arc::new(config),
        pool: DatabasePool {
            pg: pg.clone(),
            redis: redis.clone(),
        },
        mpesa: None,
        kotani: None,
        idempotency: IdempotencyService::new(std::sync::Arc::new(
            pan_africa_pay_storage::repositories::idempotency::IdempotencyRepo::new(
                pg.clone(),
                redis.clone(),
            ),
        )),
        payments: std::sync::Arc::new(
            pan_africa_pay_storage::repositories::payment::PaymentRepo::new(pg.clone()),
        ),
        users: std::sync::Arc::new(pan_africa_pay_storage::repositories::user::UserRepo::new(
            pg,
        )),
    }
}

#[tokio::test]
async fn liveness_endpoint_returns_200_and_envelope() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["data"]["status"], "ok");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .uri("/nope")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn stk_push_without_config_returns_configuration_error() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mpesa/stk/push")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"amount":"150","phone_number":"254712345678","account_reference":"ACCT-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::INTERNAL_SERVER_ERROR);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["error"]["code"], "CONFIGURATION_ERROR");
}

#[tokio::test]
async fn stk_push_validates_payload_before_provider() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mpesa/stk/push")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{"amount":"0","phone_number":"0712345678","account_reference":"ACCT-1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["error"]["code"], "VALIDATION_ERROR");
}

#[tokio::test]
async fn webhook_returns_200_and_matches_daraja_contract() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhooks/mpesa")
                .header("content-type", "application/json")
                .body(Body::from(
                    r#"{
                        "Body": {
                            "stkCallback": {
                                "MerchantRequestID": "29115-34620561-1",
                                "CheckoutRequestID": "ws_CO_191220191020363925",
                                "ResultCode": 0,
                                "ResultDesc": "The service request is processed successfully.",
                                "CallbackMetadata": {
                                    "Item": [
                                        {"Name": "Amount", "Value": 150.0},
                                        {"Name": "MpesaReceiptNumber", "Value": "NLJ7RT61SV"},
                                        {"Name": "TransactionDate", "Value": 20240101120000},
                                        {"Name": "PhoneNumber", "Value": 254712345678}
                                    ]
                                }
                            }
                        }
                    }"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["result_code"], 0);
    assert_eq!(json["result_desc"], "Success");
}

#[tokio::test]
async fn webhook_with_invalid_body_returns_400() {
    let app = build_router(test_state());

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/webhooks/mpesa")
                .header("content-type", "application/json")
                .body(Body::from("not json"))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// In-memory idempotency repository for router tests (no Redis needed).
#[derive(Default)]
struct MemoryIdempotencyRepo {
    records: std::sync::Mutex<
        std::collections::HashMap<String, pan_africa_pay_domain::traits::IdempotencyRecord>,
    >,
}

#[async_trait::async_trait]
impl pan_africa_pay_domain::traits::IdempotencyRepository for MemoryIdempotencyRepo {
    async fn store(
        &self,
        key: &str,
        request_hash: &str,
        response_body: serde_json::Value,
        status_code: u16,
        _ttl_secs: u64,
    ) -> pan_africa_pay_domain::error::AppResult<
        Option<pan_africa_pay_domain::traits::IdempotencyRecord>,
    > {
        let mut records = self.records.lock().expect("lock");
        if let Some(existing) = records.get(key) {
            if existing.request_hash != request_hash {
                return Ok(Some(existing.clone()));
            }
            return Ok(None);
        }
        records.insert(
            key.to_string(),
            pan_africa_pay_domain::traits::IdempotencyRecord {
                key: key.to_string(),
                request_hash: request_hash.to_string(),
                response_body,
                status_code,
            },
        );
        Ok(None)
    }

    async fn get(
        &self,
        key: &str,
    ) -> pan_africa_pay_domain::error::AppResult<
        Option<pan_africa_pay_domain::traits::IdempotencyRecord>,
    > {
        Ok(self.records.lock().expect("lock").get(key).cloned())
    }
}

/// State whose idempotency service uses an in-memory repository.
fn state_with_idempotency(repo: std::sync::Arc<MemoryIdempotencyRepo>) -> AppState {
    let mut state = test_state();
    state.idempotency = IdempotencyService::new(repo);
    state
}

#[tokio::test]
async fn deposit_with_idempotency_key_replays_stored_response() {
    use pan_africa_pay_domain::idempotency::RequestHash;

    let payload = serde_json::json!({
        "customer_key": "customer-key-123",
        "amount": 10.0,
        "wallet_id": "wallet-1",
        "reference_id": "ref-dep-1",
        "currency": "USD"
    });
    let hash = RequestHash::compute_bytes(payload.to_string().as_bytes());

    let repo = std::sync::Arc::new(MemoryIdempotencyRepo::default());
    repo.records.lock().expect("lock").insert(
        "k1".to_string(),
        pan_africa_pay_domain::traits::IdempotencyRecord {
            key: "k1".to_string(),
            request_hash: hash.as_str().to_string(),
            response_body: serde_json::json!({"data": {"reference_id": "stored-ref-dep-1"}}),
            status_code: 200,
        },
    );

    let app = build_router(state_with_idempotency(repo));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/kotani/deposit")
                .header("content-type", "application/json")
                .header("idempotency-key", "k1")
                .body(Body::from(payload.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["data"]["reference_id"], "stored-ref-dep-1");
}

#[tokio::test]
async fn deposit_with_conflicting_idempotency_key_returns_409() {
    use pan_africa_pay_domain::idempotency::RequestHash;

    let repo = std::sync::Arc::new(MemoryIdempotencyRepo::default());
    repo.records.lock().expect("lock").insert(
        "k2".to_string(),
        pan_africa_pay_domain::traits::IdempotencyRecord {
            key: "k2".to_string(),
            request_hash: RequestHash::compute_bytes(b"{\"different\":true}")
                .as_str()
                .to_string(),
            response_body: serde_json::json!({"data": {}}),
            status_code: 200,
        },
    );

    let app = build_router(state_with_idempotency(repo));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/kotani/deposit")
                .header("content-type", "application/json")
                .header("idempotency-key", "k2")
                .body(Body::from(
                    r#"{"customer_key":"c1","amount":10.0,"wallet_id":"w1","reference_id":"r1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::CONFLICT);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["error"]["code"], "IDEMPOTENCY_CONFLICT");
}

#[tokio::test]
async fn invalid_idempotency_key_returns_400() {
    let app = build_router(state_with_idempotency(std::sync::Arc::new(
        MemoryIdempotencyRepo::default(),
    )));
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/kotani/deposit")
                .header("content-type", "application/json")
                .header("idempotency-key", "bad key with spaces")
                .body(Body::from(
                    r#"{"customer_key":"c1","amount":10.0,"wallet_id":"w1","reference_id":"r1"}"#,
                ))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

/// In-memory payment repository for router tests (no DB needed).
#[derive(Default)]
struct MemoryPaymentRepo {
    payments: std::sync::Mutex<
        std::collections::HashMap<
            pan_africa_pay_domain::types::PaymentId,
            pan_africa_pay_domain::types::Payment,
        >,
    >,
}

#[async_trait::async_trait]
impl pan_africa_pay_domain::traits::PaymentRepository for MemoryPaymentRepo {
    async fn create_payment(
        &self,
        payment: &pan_africa_pay_domain::types::Payment,
    ) -> pan_africa_pay_domain::error::AppResult<()> {
        self.payments
            .lock()
            .expect("lock")
            .insert(payment.id, payment.clone());
        Ok(())
    }

    async fn get_payment(
        &self,
        id: pan_africa_pay_domain::types::PaymentId,
    ) -> pan_africa_pay_domain::error::AppResult<Option<pan_africa_pay_domain::types::Payment>>
    {
        Ok(self.payments.lock().expect("lock").get(&id).cloned())
    }

    async fn get_payment_by_idempotency_key(
        &self,
        key: &str,
    ) -> pan_africa_pay_domain::error::AppResult<Option<pan_africa_pay_domain::types::Payment>>
    {
        Ok(self
            .payments
            .lock()
            .expect("lock")
            .values()
            .find(|p| p.idempotency_key == key)
            .cloned())
    }

    async fn get_payment_by_mpesa_checkout_request_id(
        &self,
        checkout_request_id: &str,
    ) -> pan_africa_pay_domain::error::AppResult<Option<pan_africa_pay_domain::types::Payment>>
    {
        Ok(self
            .payments
            .lock()
            .expect("lock")
            .values()
            .find(|p| p.mpesa_checkout_request_id.as_deref() == Some(checkout_request_id))
            .cloned())
    }

    async fn update_payment_status(
        &self,
        id: pan_africa_pay_domain::types::PaymentId,
        status: pan_africa_pay_domain::types::PaymentStatus,
        mpesa_receipt_number: Option<String>,
        kotani_tx_id: Option<String>,
    ) -> pan_africa_pay_domain::error::AppResult<()> {
        if let Some(p) = self.payments.lock().expect("lock").get_mut(&id) {
            p.status = status;
            p.mpesa_receipt_number = mpesa_receipt_number;
            p.kotani_tx_id = kotani_tx_id;
        }
        Ok(())
    }

    async fn attach_callback_payload(
        &self,
        id: pan_africa_pay_domain::types::PaymentId,
        payload: serde_json::Value,
    ) -> pan_africa_pay_domain::error::AppResult<()> {
        if let Some(p) = self.payments.lock().expect("lock").get_mut(&id) {
            p.callback_payload = Some(payload);
        }
        Ok(())
    }

    async fn list_payments_by_user(
        &self,
        _user_id: pan_africa_pay_domain::types::UserId,
        _limit: i64,
        _offset: i64,
    ) -> pan_africa_pay_domain::error::AppResult<Vec<pan_africa_pay_domain::types::Payment>> {
        Ok(vec![])
    }
}

/// In-memory user repository for router tests.
#[derive(Default)]
struct MemoryUserRepo {
    users: std::sync::Mutex<
        std::collections::HashMap<
            pan_africa_pay_domain::types::UserId,
            pan_africa_pay_domain::types::User,
        >,
    >,
}

#[async_trait::async_trait]
impl pan_africa_pay_domain::traits::UserRepository for MemoryUserRepo {
    async fn create_user(
        &self,
        user: &pan_africa_pay_domain::types::User,
    ) -> pan_africa_pay_domain::error::AppResult<()> {
        self.users
            .lock()
            .expect("lock")
            .insert(user.id, user.clone());
        Ok(())
    }

    async fn get_user(
        &self,
        id: pan_africa_pay_domain::types::UserId,
    ) -> pan_africa_pay_domain::error::AppResult<Option<pan_africa_pay_domain::types::User>> {
        Ok(self.users.lock().expect("lock").get(&id).cloned())
    }
}

/// State wired with in-memory repos and optional provider clients.
fn state_with_fakes(
    mpesa: Option<pan_africa_pay_mpesa::MpesaClient>,
    kotani: Option<pan_africa_pay_kotani::KotaniClient>,
    users: std::sync::Arc<MemoryUserRepo>,
    payments: std::sync::Arc<MemoryPaymentRepo>,
) -> AppState {
    let mut state = test_state();
    state.mpesa = mpesa;
    state.kotani = kotani;
    state.idempotency =
        IdempotencyService::new(std::sync::Arc::new(MemoryIdempotencyRepo::default()));
    state.payments = payments;
    state.users = users;
    state
}

/// Build a state with in-memory repos and a seeded user.
fn state_with_seeded_user(
    mpesa: Option<pan_africa_pay_mpesa::MpesaClient>,
    kotani: Option<pan_africa_pay_kotani::KotaniClient>,
) -> (AppState, pan_africa_pay_domain::types::UserId) {
    let users = std::sync::Arc::new(MemoryUserRepo::default());
    let user_id = seeded_user();
    let user = pan_africa_pay_domain::types::User {
        id: user_id,
        email: "test@example.com".to_string(),
        phone: pan_africa_pay_domain::types::PhoneNumber::new("+254712345678").expect("phone"),
        created_at: chrono::Utc::now(),
        updated_at: chrono::Utc::now(),
    };
    users.users.lock().expect("lock").insert(user_id, user);
    (
        state_with_fakes(
            mpesa,
            kotani,
            users,
            std::sync::Arc::new(MemoryPaymentRepo::default()),
        ),
        user_id,
    )
}

fn mpesa_config(base_url: &str) -> pan_africa_pay_mpesa::config::MpesaConfig {
    pan_africa_pay_mpesa::config::MpesaConfig {
        consumer_key: "ck".to_string(),
        consumer_secret: "cs".to_string(),
        passkey: "pk".to_string(),
        short_code: "174379".to_string(),
        callback_url: "https://example.com/webhooks/mpesa".to_string(),
        environment: pan_africa_pay_mpesa::config::Environment::Sandbox,
        timeout_secs: 30,
        token_ttl_secs: 3600,
        base_url_override: base_url.to_string(),
    }
}

fn kotani_config(base_url: &str) -> pan_africa_pay_kotani::KotaniConfig {
    pan_africa_pay_kotani::KotaniConfig {
        api_key: "key".to_string(),
        api_secret: "secret".to_string(),
        base_url: base_url.to_string(),
        webhook_secret: "whsec".to_string(),
        callback_url: "https://example.com/webhooks/kotani".to_string(),
        wallet_id: "wallet-1".to_string(),
        timeout_secs: 30,
    }
}

fn seeded_user() -> pan_africa_pay_domain::types::UserId {
    pan_africa_pay_domain::types::UserId::new()
}

#[tokio::test]
async fn payin_kess_routes_to_mpesa_rail() {
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
        .and(path("/mpesa/stkpush/v1/processrequest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MerchantRequestID": "m1",
            "CheckoutRequestID": "ws_CO_1",
            "ResponseCode": "0",
            "ResponseDescription": "Success. Request accepted for processing"
        })))
        .mount(&server)
        .await;

    let (state, user_id) = state_with_seeded_user(
        Some(
            pan_africa_pay_mpesa::MpesaClient::from_config(mpesa_config(&server.uri()))
                .expect("mpesa client"),
        ),
        None,
    );
    let app = build_router(state);

    let body = serde_json::json!({
        "user_id": user_id.to_string(),
        "amount": 1500,
        "currency": "KES",
        "phone": "+254712345678"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/payments/payin")
                .header("content-type", "application/json")
                .header("idempotency-key", "payin-kes-1")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["data"]["rail"], "MPESA");
    assert_eq!(json["data"]["status"], "PENDING");
    assert_eq!(json["data"]["mpesa_checkout_request_id"], "ws_CO_1");
    assert!(json["data"]["payment_id"].as_str().is_some());
}

#[tokio::test]
async fn payin_usdc_routes_to_kotani_rail() {
    use wiremock::matchers::{bearer_token, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/api/v3/customer/mobile-money"))
        .and(bearer_token("key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "Customer created",
            "data": {
                "id": "cust-1",
                "phone_number": "+254712345678",
                "country_code": "KE",
                "customer_key": "customer-key-123"
            }
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/v3/deposit/mobile-money"))
        .and(bearer_token("key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "Deposit created",
            "data": {
                "id": "dep-1",
                "message": "pending",
                "reference_id": "some-ref",
                "reference_number": 42
            }
        })))
        .mount(&server)
        .await;

    let (state, user_id) = state_with_seeded_user(
        None,
        Some(
            pan_africa_pay_kotani::KotaniClient::from_config(kotani_config(&server.uri()))
                .expect("kotani client"),
        ),
    );
    let app = build_router(state);

    let body = serde_json::json!({
        "user_id": user_id.to_string(),
        "amount": 5_000_000,
        "currency": "USDC",
        "phone": "+254712345678"
    });
    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/payments/payin")
                .header("content-type", "application/json")
                .header("idempotency-key", "payin-usdc-1")
                .body(Body::from(body.to_string()))
                .expect("request"),
        )
        .await
        .expect("response");

    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let json: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(json["data"]["rail"], "KOTANI");
    assert_eq!(json["data"]["status"], "PROCESSING");
    assert_eq!(json["data"]["kotani_tx_id"], "dep-1");
    assert_eq!(json["data"]["kotani_customer_key"], "customer-key-123");
}
