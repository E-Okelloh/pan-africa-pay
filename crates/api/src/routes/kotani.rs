//! Kotani endpoints: mobile money customers, deposits, withdrawals,
//! and the signed webhook for transaction callbacks.

use axum::body::Bytes;
use axum::extract::State;
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use pan_africa_pay_kotani::types::{CustomerRequest, DepositRequest, WithdrawRequest};

use crate::error::{ApiError, ApiResult};
use crate::routes::{ok, OkEnvelope};
use crate::state::AppState;

/// Signature header Kotani sends on webhook callbacks.
const SIGNATURE_HEADER: &str = "x-kotani-signature";

/// Body accepted by `POST /kotani/customers`.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateCustomerPayload {
    /// E.164 phone number, e.g. `+254712345678`.
    pub phone_number: String,
    /// `GH`, `KE`, `NG` (or ISO-3: `GHA`, `KEN`, `NGA`).
    pub country_code: String,
    /// Mobile money network, e.g. `MPESA`, `MTN`, `AIRTEL`, `VODAFONE`.
    pub network: Option<String>,
    pub account_name: Option<String>,
    pub first_name: Option<String>,
    pub last_name: Option<String>,
    pub email: Option<String>,
}

/// Body accepted by `POST /kotani/deposit`.
#[derive(Debug, Clone, Deserialize)]
pub struct DepositPayload {
    /// Customer key returned at registration.
    pub customer_key: String,
    /// Amount in stablecoin units (e.g. USDT).
    pub amount: f64,
    /// Crypto wallet id from `GET /kotani/wallets`.
    pub wallet_id: String,
    /// Idempotency / correlation reference.
    pub reference_id: String,
    pub currency: Option<String>,
}

/// Body accepted by `POST /kotani/withdraw`.
#[derive(Debug, Clone, Deserialize)]
pub struct WithdrawPayload {
    pub customer_key: String,
    pub amount: f64,
    pub wallet_id: String,
    pub reference_id: String,
    pub currency: Option<String>,
    /// Mobile money network for the payout, e.g. `MPESA`.
    pub network: Option<String>,
}

/// Customer registration response.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerAck {
    pub id: Option<String>,
    pub phone_number: String,
    pub country_code: String,
    pub network: Option<String>,
    pub customer_key: Option<String>,
}

/// Deposit acknowledgement.
#[derive(Debug, Clone, Serialize)]
pub struct DepositAck {
    pub id: Option<String>,
    pub reference_id: String,
    pub reference_number: Option<u64>,
    pub redirect_url: Option<String>,
}

/// Withdrawal acknowledgement.
#[derive(Debug, Clone, Serialize)]
pub struct WithdrawAck {
    pub id: Option<String>,
    pub reference_id: String,
    pub reference_number: Option<u64>,
}

/// Register a mobile money customer with Kotani.
pub async fn create_customer(
    State(state): State<AppState>,
    Json(payload): Json<CreateCustomerPayload>,
) -> ApiResult<Json<OkEnvelope<CustomerAck>>> {
    validate_phone(&payload.phone_number)?;
    validate_country(&payload.country_code)?;

    let client = kotani_client(&state)?;
    let customer = client
        .create_customer(&CustomerRequest {
            phone_number: payload.phone_number,
            country_code: payload.country_code,
            network: payload.network,
            account_name: payload.account_name,
            first_name: payload.first_name,
            last_name: payload.last_name,
            email: payload.email,
        })
        .await?;

    Ok(ok(CustomerAck {
        id: customer.id,
        phone_number: customer.phone_number,
        country_code: customer.country_code,
        network: customer.network,
        customer_key: customer.customer_key,
    }))
}

/// Initiate a deposit (fiat -> stablecoin).
pub async fn deposit(
    State(state): State<AppState>,
    Json(payload): Json<DepositPayload>,
) -> ApiResult<Json<OkEnvelope<DepositAck>>> {
    validate_amount(payload.amount)?;
    validate_reference(&payload.reference_id)?;

    let client = kotani_client(&state)?;
    let response = client
        .deposit(&DepositRequest {
            customer_key: payload.customer_key,
            amount: payload.amount,
            wallet_id: payload.wallet_id,
            callback_url: Some(state.config.kotani.callback_url.clone()),
            reference_id: payload.reference_id,
            currency: payload.currency,
        })
        .await?;

    Ok(ok(DepositAck {
        id: response.id,
        reference_id: response.reference_id,
        reference_number: response.reference_number,
        redirect_url: response.redirect_url,
    }))
}

/// Initiate a withdrawal (stablecoin -> fiat).
pub async fn withdraw(
    State(state): State<AppState>,
    Json(payload): Json<WithdrawPayload>,
) -> ApiResult<Json<OkEnvelope<WithdrawAck>>> {
    validate_amount(payload.amount)?;
    validate_reference(&payload.reference_id)?;

    let client = kotani_client(&state)?;
    let response = client
        .withdraw(&WithdrawRequest {
            customer_key: payload.customer_key,
            amount: payload.amount,
            wallet_id: payload.wallet_id,
            callback_url: Some(state.config.kotani.callback_url.clone()),
            reference_id: payload.reference_id,
            currency: payload.currency,
            network: payload.network,
        })
        .await?;

    Ok(ok(WithdrawAck {
        id: response.id,
        reference_id: response.reference_id,
        reference_number: response.reference_number,
    }))
}

/// Kotani transaction callback.
///
/// Fields marked with `#[allow(dead_code)]` are part of the wire
/// contract even if the current handler does not consume them yet.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
pub struct KotaniCallback {
    /// Event type, e.g. `deposit.success`, `deposit.failed`.
    pub event: Option<String>,
    #[serde(rename = "reference_id")]
    pub reference_id: Option<String>,
    #[serde(rename = "referenceId")]
    pub reference_id_camel: Option<String>,
    pub status: Option<String>,
    pub message: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Handle a signed Kotani webhook callback.
///
/// Kotani retries non-2xx responses, so this handler always responds
/// 200 after verifying the `X-Kotani-Signature` and recording the
/// outcome. Unverifiable callbacks are rejected with 401.
pub async fn webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<Json<serde_json::Value>, (axum::http::StatusCode, String)> {
    let signature = headers
        .get(SIGNATURE_HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();

    let secret = &state.config.kotani.webhook_secret;
    if !pan_africa_pay_kotani::verify_webhook_signature(secret, &body, signature) {
        warn!(
            provider = "kotani",
            "rejected webhook with invalid signature"
        );
        return Err((
            axum::http::StatusCode::UNAUTHORIZED,
            "invalid signature".to_string(),
        ));
    }

    let callback: KotaniCallback = serde_json::from_slice(&body).map_err(|err| {
        let message = format!("invalid webhook body: {err}");
        warn!(provider = "kotani", "{message}");
        (axum::http::StatusCode::BAD_REQUEST, message)
    })?;

    info!(
        provider = "kotani",
        event = ?callback.event,
        status = ?callback.status,
        reference_id = ?callback.reference_id,
        "received Kotani callback"
    );

    Ok(Json(serde_json::json!({ "received": true })))
}

/// Access the Kotani client, failing fast when unconfigured.
fn kotani_client(state: &AppState) -> ApiResult<&pan_africa_pay_kotani::KotaniClient> {
    state.kotani.as_ref().ok_or_else(|| {
        ApiError::from(pan_africa_pay_domain::error::AppError::configuration(
            "Kotani is not configured",
        ))
    })
}

/// E.164 phone validation: optional `+`, country code + 9 digits.
fn validate_phone(phone: &str) -> ApiResult<()> {
    let digits: String = phone.chars().filter(|c| c.is_ascii_digit()).collect();
    if (11..=15).contains(&digits.len()) && digits.starts_with("254") {
        Ok(())
    } else {
        Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "phone_number must be E.164 (e.g. +254712345678)",
            ),
        ))
    }
}

/// Country code must be 2 or 3 uppercase letters.
fn validate_country(country: &str) -> ApiResult<()> {
    if (2..=3).contains(&country.len()) && country.chars().all(|c| c.is_ascii_uppercase()) {
        Ok(())
    } else {
        Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "country_code must be 2-3 uppercase letters (KE, GHA, ...)",
            ),
        ))
    }
}

/// Amount must be positive and bounded.
fn validate_amount(amount: f64) -> ApiResult<()> {
    if amount > 0.0 && amount <= 1_000_000.0 {
        Ok(())
    } else {
        Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "amount must be between 0 and 1,000,000",
            ),
        ))
    }
}

/// Reference id must be non-empty.
fn validate_reference(reference: &str) -> ApiResult<()> {
    if reference.trim().is_empty() {
        Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation("reference_id is required"),
        ))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pan_africa_pay_kotani::webhook;

    #[test]
    fn phone_validation_accepts_e164_variants() {
        assert!(validate_phone("+254712345678").is_ok());
        assert!(validate_phone("254712345678").is_ok());
    }

    #[test]
    fn phone_validation_rejects_bad_format() {
        assert!(validate_phone("0712345678").is_err());
        assert!(validate_phone("123").is_err());
        assert!(validate_phone("+254712345678901234").is_err());
    }

    #[test]
    fn country_validation() {
        assert!(validate_country("KE").is_ok());
        assert!(validate_country("GHA").is_ok());
        assert!(validate_country("ke").is_err());
        assert!(validate_country("KENYA").is_err());
        assert!(validate_country("").is_err());
    }

    #[test]
    fn amount_validation() {
        assert!(validate_amount(1.0).is_ok());
        assert!(validate_amount(0.0).is_err());
        assert!(validate_amount(-5.0).is_err());
        assert!(validate_amount(2_000_000.0).is_err());
    }

    #[test]
    fn reference_validation() {
        assert!(validate_reference("ref-1").is_ok());
        assert!(validate_reference("").is_err());
        assert!(validate_reference("  ").is_err());
    }

    #[test]
    fn parses_kotani_callback() {
        let json = serde_json::json!({
            "event": "deposit.success",
            "reference_id": "ref-dep-1",
            "status": "completed",
            "message": "Funds credited",
            "amount": 10.0
        });
        let callback: KotaniCallback = serde_json::from_value(json).expect("parse callback");
        assert_eq!(callback.event.as_deref(), Some("deposit.success"));
        assert_eq!(callback.reference_id.as_deref(), Some("ref-dep-1"));
        assert!(callback.extra.contains_key("amount"));
    }

    #[test]
    fn signed_callback_verifies_round_trip() {
        let secret = "webhook-secret";
        let body = br#"{"event":"deposit.success","reference_id":"ref-dep-1"}"#;
        let sig = webhook::sign(secret, body);
        assert!(webhook::verify_signature(secret, body, &sig));
        assert!(!webhook::verify_signature(secret, b"tampered", &sig));
    }
}
