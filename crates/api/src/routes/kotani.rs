//! Kotani endpoints: mobile money customers, deposits, withdrawals,
//! and the signed webhook for transaction callbacks.

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::response::Response;
use axum::Json;
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use pan_africa_pay_domain::idempotency::RequestHash;
use pan_africa_pay_kotani::types::{CustomerRequest, DepositRequest, WithdrawRequest};

use crate::error::{ApiError, ApiResult};
use crate::idempotency::{claim_or_replay, IdempotencyHeader};
use crate::routes::{ok, OkEnvelope};
use crate::state::AppState;

/// Signature header Kotani sends on webhook callbacks.
const SIGNATURE_HEADER: &str = "x-kotani-signature";

/// Body accepted by `POST /kotani/customers`.
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    header: IdempotencyHeader,
    body: Bytes,
) -> ApiResult<Response> {
    let payload: CreateCustomerPayload = parse_json(&body)?;
    validate_phone(&payload.phone_number)?;
    validate_country(&payload.country_code)?;

    let hash = RequestHash::compute_bytes(&body);
    if let Some(response) = claim_or_replay(&state.idempotency, header.clone(), &hash).await? {
        return Ok(response);
    }

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

    let body = serde_json::to_value(OkEnvelope {
        data: CustomerAck {
            id: customer.id,
            phone_number: customer.phone_number,
            country_code: customer.country_code,
            network: customer.network,
            customer_key: customer.customer_key,
        },
    })
    .expect("serializable ack");
    if let Some(key) = header.0 {
        state
            .idempotency
            .complete(&key, &hash, body.clone(), StatusCode::OK.as_u16())
            .await?;
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Initiate a deposit (fiat -> stablecoin).
pub async fn deposit(
    State(state): State<AppState>,
    header: IdempotencyHeader,
    body: Bytes,
) -> ApiResult<Response> {
    let payload: DepositPayload = parse_json(&body)?;
    validate_amount(payload.amount)?;
    validate_reference(&payload.reference_id)?;

    let hash = RequestHash::compute_bytes(&body);
    if let Some(response) = claim_or_replay(&state.idempotency, header.clone(), &hash).await? {
        return Ok(response);
    }

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

    let body = serde_json::to_value(OkEnvelope {
        data: DepositAck {
            id: response.id,
            reference_id: response.reference_id,
            reference_number: response.reference_number,
            redirect_url: response.redirect_url,
        },
    })
    .expect("serializable ack");
    if let Some(key) = header.0 {
        state
            .idempotency
            .complete(&key, &hash, body.clone(), StatusCode::OK.as_u16())
            .await?;
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Initiate a withdrawal (stablecoin -> fiat).
pub async fn withdraw(
    State(state): State<AppState>,
    header: IdempotencyHeader,
    body: Bytes,
) -> ApiResult<Response> {
    let payload: WithdrawPayload = parse_json(&body)?;
    validate_amount(payload.amount)?;
    validate_reference(&payload.reference_id)?;

    let hash = RequestHash::compute_bytes(&body);
    if let Some(response) = claim_or_replay(&state.idempotency, header.clone(), &hash).await? {
        return Ok(response);
    }

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

    let body = serde_json::to_value(OkEnvelope {
        data: WithdrawAck {
            id: response.id,
            reference_id: response.reference_id,
            reference_number: response.reference_number,
        },
    })
    .expect("serializable ack");
    if let Some(key) = header.0 {
        state
            .idempotency
            .complete(&key, &hash, body.clone(), StatusCode::OK.as_u16())
            .await?;
    }
    Ok((StatusCode::OK, Json(body)).into_response())
}

/// Poll the status of a Kotani deposit by reference id.
pub async fn deposit_status(
    State(state): State<AppState>,
    Path(reference_id): Path<String>,
) -> ApiResult<Json<OkEnvelope<StatusAck>>> {
    let client = kotani_client(&state)?;
    let status = client.deposit_status(&reference_id).await?;
    Ok(ok(StatusAck {
        reference_id: status
            .reference_id
            .or(status.reference_id_camel)
            .unwrap_or(reference_id),
        status: status.status,
        message: status.message,
        amount: status.amount,
        currency: status.currency,
    }))
}

/// Poll the status of a Kotani withdrawal by reference id.
pub async fn withdraw_status(
    State(state): State<AppState>,
    Path(reference_id): Path<String>,
) -> ApiResult<Json<OkEnvelope<StatusAck>>> {
    let client = kotani_client(&state)?;
    let status = client.withdraw_status(&reference_id).await?;
    Ok(ok(StatusAck {
        reference_id: status
            .reference_id
            .or(status.reference_id_camel)
            .unwrap_or(reference_id),
        status: status.status,
        message: status.message,
        amount: status.amount,
        currency: status.currency,
    }))
}

/// Transaction status acknowledgement.
#[derive(Debug, Clone, Serialize)]
pub struct StatusAck {
    pub reference_id: String,
    pub status: Option<String>,
    pub message: Option<String>,
    pub amount: Option<f64>,
    pub currency: Option<String>,
}

/// Kotani transaction callback.
///
/// Fields marked with `#[allow(dead_code)]` are part of the wire
/// contract even if the current handler does not consume them yet.
#[derive(Debug, Deserialize, Serialize)]
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

    // Reconcile: the deposit reference_id is our payment id. Resolve the
    // payment and update its status from the callback. Reconciliation is
    // best-effort at callback time: storage failures are logged and the
    // callback still acknowledged so Kotani never retries.
    let reference = callback
        .reference_id
        .as_deref()
        .or(callback.reference_id_camel.as_deref());
    if let Some(reference) = reference {
        if let Some(id) = parse_payment_id(reference) {
            match state.payments.get_payment(id).await {
                Ok(Some(payment)) => {
                    let outcome = callback
                        .event
                        .as_deref()
                        .or(callback.status.as_deref())
                        .unwrap_or_default()
                        .to_lowercase();
                    let status = if outcome.contains("success") || outcome.contains("complete") {
                        pan_africa_pay_domain::types::PaymentStatus::Completed
                    } else if outcome.contains("fail") || outcome.contains("cancel") {
                        pan_africa_pay_domain::types::PaymentStatus::Failed
                    } else {
                        pan_africa_pay_domain::types::PaymentStatus::Processing
                    };
                    if let Err(err) = state
                        .payments
                        .update_payment_status(payment.id, status, None, None)
                        .await
                    {
                        tracing::error!(provider = "kotani", "payment update failed: {err}");
                    }
                    if let Err(err) = state
                        .payments
                        .attach_callback_payload(
                            payment.id,
                            serde_json::to_value(&callback).unwrap_or_default(),
                        )
                        .await
                    {
                        tracing::error!(provider = "kotani", "callback attach failed: {err}");
                    }
                    crate::events::publish_best_effort(
                        state.events.as_ref(),
                        &kotani_reconciliation_events(&payment, status),
                    )
                    .await;
                }
                Ok(None) => {
                    tracing::warn!(
                        provider = "kotani",
                        reference_id = reference,
                        "callback received for unknown payment"
                    );
                }
                Err(err) => {
                    tracing::error!(provider = "kotani", "payment lookup failed: {err}");
                }
            }
        } else {
            tracing::warn!(
                provider = "kotani",
                reference_id = reference,
                "callback reference is not a payment id"
            );
        }
    }

    Ok(Json(serde_json::json!({ "received": true })))
}

/// Audit events for a Kotani callback reconciliation: always a
/// `PaymentTransition`; plus `KotaniTransaction` when the deposit
/// completed and we hold its transaction id.
fn kotani_reconciliation_events(
    payment: &pan_africa_pay_domain::types::Payment,
    status: pan_africa_pay_domain::types::PaymentStatus,
) -> Vec<pan_africa_pay_domain::events::DomainEvent> {
    use pan_africa_pay_domain::events::{DomainEvent, EventId, KotaniTransactionEvent};
    let mut events = vec![DomainEvent::PaymentTransition(
        pan_africa_pay_domain::events::PaymentEvent::new(
            payment.id,
            payment.user_id,
            status,
            Some(payment.status),
        ),
    )];
    if status == pan_africa_pay_domain::types::PaymentStatus::Completed {
        if let Some(kotani_tx_id) = payment.kotani_tx_id.clone() {
            events.push(DomainEvent::KotaniTransaction(KotaniTransactionEvent {
                event_id: EventId::new(),
                payment_id: payment.id,
                user_id: payment.user_id,
                kotani_tx_id,
                amount: payment.amount,
                occurred_at: chrono::Utc::now(),
            }));
        }
    }
    events
}

/// Access the Kotani client, failing fast when unconfigured.
fn kotani_client(state: &AppState) -> ApiResult<&pan_africa_pay_kotani::KotaniClient> {
    state.kotani.as_ref().ok_or_else(|| {
        ApiError::from(pan_africa_pay_domain::error::AppError::configuration(
            "Kotani is not configured",
        ))
    })
}

/// Parse a callback reference into a payment id (UUID).
fn parse_payment_id(reference: &str) -> Option<pan_africa_pay_domain::types::PaymentId> {
    uuid::Uuid::parse_str(reference)
        .ok()
        .map(pan_africa_pay_domain::types::PaymentId)
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

/// Deserialize a request body, mapping JSON failures to 400 validation
/// errors (replaces the axum `Json` rejection path).
fn parse_json<T: serde::de::DeserializeOwned>(body: &[u8]) -> ApiResult<T> {
    serde_json::from_slice(body).map_err(|err| {
        ApiError::from(pan_africa_pay_domain::error::AppError::validation(format!(
            "invalid JSON body: {err}"
        )))
    })
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
