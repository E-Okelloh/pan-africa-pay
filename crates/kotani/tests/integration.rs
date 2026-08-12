//! Wiremock-backed integration tests for the Kotani client.
//!
//! These verify the wire contract against a mock server: auth header,
//! request body shapes (snake_case vs camelCase per endpoint), envelope
//! decoding, and error mapping.

use pan_africa_pay_kotani::types::{
    CustomerRequest, DepositRequest, StatusResponse, WithdrawRequest,
};
use pan_africa_pay_kotani::{KotaniClient, KotaniConfig, KotaniError};
use wiremock::matchers::{bearer_token, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn config(base_url: &str) -> KotaniConfig {
    KotaniConfig {
        api_key: "test-api-key".to_string(),
        api_secret: "test-api-secret".to_string(),
        base_url: base_url.to_string(),
        callback_url: "https://example.com/webhooks/kotani".to_string(),
        ..KotaniConfig::default()
    }
}

fn deposit_request() -> DepositRequest {
    DepositRequest {
        customer_key: "customer-key-123".to_string(),
        amount: 10.0,
        wallet_id: "wallet-1".to_string(),
        callback_url: Some("https://example.com/webhooks/kotani".to_string()),
        reference_id: "ref-dep-1".to_string(),
        currency: Some("USD".to_string()),
    }
}

fn withdraw_request() -> WithdrawRequest {
    WithdrawRequest {
        customer_key: "customer-key-123".to_string(),
        amount: 10.0,
        wallet_id: "wallet-1".to_string(),
        callback_url: None,
        reference_id: "ref-wd-1".to_string(),
        currency: None,
        network: Some("MPESA".to_string()),
    }
}

fn customer_request() -> CustomerRequest {
    CustomerRequest {
        phone_number: "+254712345678".to_string(),
        country_code: "KE".to_string(),
        network: Some("MPESA".to_string()),
        account_name: None,
        first_name: Some("John".to_string()),
        last_name: Some("Doe".to_string()),
        email: None,
    }
}

#[tokio::test]
async fn creates_mobile_money_customer() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    Mock::given(method("POST"))
        .and(path("/api/v3/customer/mobile-money"))
        .and(bearer_token("test-api-key"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "Customer created",
            "data": {
                "id": "cust-1",
                "phone_number": "+254712345678",
                "country_code": "KE",
                "network": "MPESA",
                "customer_key": "customer-key-123"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let customer = client
        .create_customer(&customer_request())
        .await
        .expect("ok");
    assert_eq!(customer.customer_key.as_deref(), Some("customer-key-123"));
    assert_eq!(customer.phone_number, "+254712345678");
}

#[tokio::test]
async fn initiates_deposit() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    Mock::given(method("POST"))
        .and(path("/api/v3/deposit/mobile-money"))
        .and(bearer_token("test-api-key"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json",
        ))
        .and(wiremock::matchers::body_partial_json(
            serde_json::to_value(deposit_request()).unwrap(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "Deposit created",
            "data": {
                "id": "dep-1",
                "message": "pending",
                "reference_id": "ref-dep-1",
                "reference_number": 42,
                "redirect_url": "https://sandbox.kotanipay.io/redirect"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let response = client.deposit(&deposit_request()).await.expect("ok");
    assert_eq!(response.reference_id, "ref-dep-1");
    assert_eq!(response.reference_number, Some(42));
    assert!(response.redirect_url.is_some());
}

#[tokio::test]
async fn initiates_withdrawal_with_camel_case_fields() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    let expected_body = serde_json::json!({
        "customer_key": "customer-key-123",
        "amount": 10.0,
        "walletId": "wallet-1",
        "referenceId": "ref-wd-1",
        "network": "MPESA"
    });

    Mock::given(method("POST"))
        .and(path("/api/v3/withdraw/mobile-money"))
        .and(bearer_token("test-api-key"))
        .and(wiremock::matchers::header(
            "content-type",
            "application/json",
        ))
        .and(wiremock::matchers::body_partial_json(expected_body))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "Withdrawal created",
            "data": {
                "id": "wd-1",
                "message": "pending",
                "referenceId": "ref-wd-1",
                "referenceNumber": 43
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let response = client.withdraw(&withdraw_request()).await.expect("ok");
    assert_eq!(response.reference_id, "ref-wd-1");
    assert_eq!(response.reference_number, Some(43));
}

#[tokio::test]
async fn polls_deposit_status() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    Mock::given(method("GET"))
        .and(path("/api/v3/deposit/mobile-money/status/ref-dep-1"))
        .and(bearer_token("test-api-key"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": true,
            "message": "status",
            "data": {
                "status": "completed",
                "reference_id": "ref-dep-1",
                "amount": 10.0,
                "currency": "USD"
            }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let status: StatusResponse = client.deposit_status("ref-dep-1").await.expect("ok");
    assert_eq!(status.status.as_deref(), Some("completed"));
    assert_eq!(status.amount, Some(10.0));
}

#[tokio::test]
async fn authentication_failure_maps_to_auth_error() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    Mock::given(method("POST"))
        .and(path("/api/v3/customer/mobile-money"))
        .respond_with(ResponseTemplate::new(401).set_body_string("Unauthorized"))
        .expect(1)
        .mount(&server)
        .await;

    let err = client
        .create_customer(&customer_request())
        .await
        .unwrap_err();
    assert!(matches!(err, KotaniError::Authentication(_)));
}

#[tokio::test]
async fn provider_error_envelope_surfaces_message() {
    let server = MockServer::start().await;
    let client = KotaniClient::from_config(config(&server.uri())).expect("client");

    Mock::given(method("POST"))
        .and(path("/api/v3/customer/mobile-money"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "success": false,
            "message": "Invalid phone number",
            "data": null
        })))
        .expect(1)
        .mount(&server)
        .await;

    let err = client
        .create_customer(&customer_request())
        .await
        .unwrap_err();
    match err {
        KotaniError::Provider { code, message } => {
            assert_eq!(message, "Invalid phone number");
            assert_eq!(code, "API_ERROR");
        }
        other => panic!("expected provider error, got {other:?}"),
    }
}
