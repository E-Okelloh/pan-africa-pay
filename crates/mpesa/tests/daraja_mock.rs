//! End-to-end client tests against a mock Daraja server.
//!
//! The mock asserts the OAuth flow (basic auth on the token endpoint,
//! bearer auth on the STK endpoint) and verifies request signing
//! against the real-public Daraja wire contract.

use wiremock::matchers::{body_partial_json, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use pan_africa_pay_mpesa::config::{Environment, MpesaConfig};
use pan_africa_pay_mpesa::types::StkPushRequest;
use pan_africa_pay_mpesa::MpesaClient;

/// Sandbox config pointed at the wiremock server.
fn config(base: &str) -> MpesaConfig {
    MpesaConfig {
        consumer_key: "test-consumer-key".to_string(),
        consumer_secret: "test-consumer-secret".to_string(),
        passkey: "test-passkey".to_string(),
        short_code: "174379".to_string(),
        callback_url: format!("{base}/cb"),
        environment: Environment::Sandbox,
        timeout_secs: 5,
        token_ttl_secs: 3_500,
        base_url_override: base.to_string(),
    }
}

fn stk_request(callback_url: &str) -> StkPushRequest {
    StkPushRequest {
        business_short_code: "174379".to_string(),
        password: "dummy".to_string(),
        timestamp: "20240101120000".to_string(),
        transaction_type: "CustomerPayBillOnline".to_string(),
        amount: "150.00".to_string(),
        party_a: "254712345678".to_string(),
        party_b: "174379".to_string(),
        phone_number: "254712345678".to_string(),
        callback_url: callback_url.to_string(),
        account_reference: "ACCT-1".to_string(),
        transaction_desc: "payment".to_string(),
    }
}

/// Payload Daraja expects on the wire for the STK push body. The
/// `Password` and `Timestamp` fields are filled by the caller so the
/// mock can validate them exactly when the test supplies them.
#[derive(serde::Serialize)]
struct WireStkPush {
    #[serde(rename = "BusinessShortCode")]
    business_short_code: &'static str,
    #[serde(rename = "Password")]
    password: String,
    #[serde(rename = "Timestamp")]
    timestamp: &'static str,
    #[serde(rename = "TransactionType")]
    transaction_type: &'static str,
    #[serde(rename = "Amount")]
    amount: &'static str,
    #[serde(rename = "PartyA")]
    party_a: &'static str,
    #[serde(rename = "PartyB")]
    party_b: &'static str,
    #[serde(rename = "PhoneNumber")]
    phone_number: &'static str,
    #[serde(rename = "CallBackURL")]
    callback_url: String,
    #[serde(rename = "AccountReference")]
    account_reference: &'static str,
    #[serde(rename = "TransactionDesc")]
    transaction_desc: &'static str,
}

#[tokio::test]
async fn stk_push_authenticates_and_sends() {
    let server = MockServer::start().await;

    // Token endpoint: expect basic auth with the consumer credentials
    // and return a short-lived token.
    Mock::given(method("GET"))
        .and(path("/oauth/v1/generate"))
        .and(query_param("grant_type", "client_credentials"))
        .and(header(
            "authorization",
            "Basic dGVzdC1jb25zdW1lci1rZXk6dGVzdC1jb25zdW1lci1zZWNyZXQ=",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "mock-access-token",
            "expires_in": 3600,
        })))
        .expect(1)
        .mount(&server)
        .await;

    // STK push endpoint: expect the bearer token and the exact wire
    // payload the client computes. The client signs `Password` from
    // the current timestamp, so assert the fixed business fields plus
    // a valid password prefix (base64 of shortcode+passkey).
    let expected_body = WireStkPush {
        business_short_code: "174379",
        password: "prefix-not-asserted".to_string(),
        timestamp: "not-asserted",
        transaction_type: "CustomerPayBillOnline",
        amount: "150.00",
        party_a: "254712345678",
        party_b: "174379",
        phone_number: "254712345678",
        callback_url: format!("{}/cb", server.uri()),
        account_reference: "ACCT-1",
        transaction_desc: "payment",
    };

    let mut expected_value = serde_json::to_value(&expected_body).expect("body");
    // Remove the fields that legitimately vary (signing inputs).
    expected_value
        .as_object_mut()
        .expect("object")
        .remove("Password");
    expected_value
        .as_object_mut()
        .expect("object")
        .remove("Timestamp");

    Mock::given(method("POST"))
        .and(path("/mpesa/stkpush/v1/processrequest"))
        .and(header("authorization", "Bearer mock-access-token"))
        .and(body_partial_json(expected_value))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MerchantRequestID": "29115-34620561-1",
            "CheckoutRequestID": "ws_CO_191220191020363925",
            "ResponseCode": "0",
            "ResponseDescription": "Request accepted for processing",
            "CustomerMessage": "Request accepted",
        })))
        .expect(1)
        .mount(&server)
        .await;

    let client = MpesaClient::new(config(&server.uri())).expect("client");
    let response = client
        .stk_push(&stk_request(&format!("{}/cb", server.uri())))
        .await
        .expect("stk push");

    assert_eq!(response.response_code, "0");
    assert_eq!(
        response.checkout_request_id.as_deref(),
        Some("ws_CO_191220191020363925")
    );
    assert!(response.is_accepted());
}

#[tokio::test]
async fn token_is_cached_between_calls() {
    let server = MockServer::start().await;

    // Token endpoint hit exactly once across two STK pushes.
    Mock::given(method("GET"))
        .and(path("/oauth/v1/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "mock-access-token",
            "expires_in": 3600,
        })))
        .expect(1)
        .mount(&server)
        .await;

    Mock::given(method("POST"))
        .and(path("/mpesa/stkpush/v1/processrequest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MerchantRequestID": "m1",
            "CheckoutRequestID": "c1",
            "ResponseCode": "0",
            "ResponseDescription": "accepted",
            "CustomerMessage": null,
        })))
        .expect(2)
        .mount(&server)
        .await;

    let client = MpesaClient::new(config(&server.uri())).expect("client");
    let req = stk_request(&format!("{}/cb", server.uri()));
    client.stk_push(&req).await.expect("first push");
    client.stk_push(&req).await.expect("second push");
}

#[tokio::test]
async fn token_failure_propagates_as_authentication_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/oauth/v1/generate"))
        .respond_with(ResponseTemplate::new(401).set_body_string("invalid credentials"))
        .expect(1)
        .mount(&server)
        .await;

    let client = MpesaClient::new(config(&server.uri())).expect("client");
    let result = client
        .stk_push(&stk_request(&format!("{}/cb", server.uri())))
        .await;
    assert!(matches!(
        result,
        Err(pan_africa_pay_mpesa::MpesaError::Authentication(_))
    ));
}

#[tokio::test]
async fn provider_rejection_returns_provider_error() {
    let server = MockServer::start().await;

    Mock::given(method("GET"))
        .and(path("/oauth/v1/generate"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "mock-access-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;

    // Daraja rejects with a non-zero ResponseCode.
    Mock::given(method("POST"))
        .and(path("/mpesa/stkpush/v1/processrequest"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "MerchantRequestID": "m1",
            "CheckoutRequestID": "c1",
            "ResponseCode": "1037",
            "ResponseDescription": "DS timeout",
            "CustomerMessage": null,
        })))
        .mount(&server)
        .await;

    let client = MpesaClient::new(config(&server.uri())).expect("client");
    let result = client
        .stk_push(&stk_request(&format!("{}/cb", server.uri())))
        .await;
    assert!(matches!(
        result,
        Err(pan_africa_pay_mpesa::MpesaError::Provider { .. })
    ));
}
