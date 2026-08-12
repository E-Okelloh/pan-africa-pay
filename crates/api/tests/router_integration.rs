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
        pool: DatabasePool { pg, redis },
        mpesa: None,
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
