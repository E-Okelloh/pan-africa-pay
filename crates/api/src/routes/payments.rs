//! Dual-rail payment flow: a single payin endpoint that routes to the
//! right provider rail, plus user and payment read endpoints.
//!
//! Routing rules:
//! - `KES` -> M-Pesa STK push (local fiat rail)
//! - `USDC` -> Kotani Pay deposit (cross-border stablecoin rail)
//!
//! Every payin requires an `Idempotency-Key` header: the key becomes
//! the payment's `idempotency_key` (unique in the database), so retries
//! can never create a second payment.

use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use chrono::Utc;
use serde::{Deserialize, Serialize};

use pan_africa_pay_domain::idempotency::RequestHash;
use pan_africa_pay_domain::types::{
    Currency, Money, Payment, PaymentId, PaymentStatus, PaymentType, PhoneNumber, Rail, User,
    UserId,
};
use pan_africa_pay_mpesa::types::StkPushRequest;

use crate::error::{ApiError, ApiResult};
use crate::idempotency::{claim_or_replay, IdempotencyHeader};
use crate::routes::{ok, OkEnvelope};
use crate::state::AppState;

/// Body accepted by `POST /payments/payin`.
#[derive(Debug, Clone, Deserialize)]
pub struct PayinPayload {
    /// User initiating the payment (must exist).
    pub user_id: UserId,
    /// Amount in minor units (cents for KES, avos for USDC).
    pub amount: i64,
    /// `KES` routes to M-Pesa; `USDC` routes to Kotani.
    pub currency: Currency,
    /// Customer phone in E.164 format.
    pub phone: String,
    /// Kotani customer key (stablecoin rail only). Auto-registered when
    /// absent and returned in the response.
    pub customer_key: Option<String>,
}

/// User creation body.
#[derive(Debug, Clone, Deserialize)]
pub struct CreateUserPayload {
    pub email: String,
    pub phone: String,
}

/// Payin acknowledgement.
#[derive(Debug, Clone, Serialize)]
pub struct PayinAck {
    pub payment_id: String,
    pub rail: Rail,
    pub status: PaymentStatus,
    pub amount: i64,
    pub currency: Currency,
    pub mpesa_checkout_request_id: Option<String>,
    pub kotani_tx_id: Option<String>,
    /// Present when the stablecoin rail auto-registered a Kotani customer.
    pub kotani_customer_key: Option<String>,
}

/// Initiate a payment on the correct rail.
pub async fn payin(
    State(state): State<AppState>,
    header: IdempotencyHeader,
    body: Bytes,
) -> ApiResult<Response> {
    let key = header.0.ok_or_else(|| {
        ApiError::from(pan_africa_pay_domain::error::AppError::validation(
            "Idempotency-Key header is required for payin",
        ))
    })?;
    let payload: PayinPayload = parse_json(&body)?;
    validate_payin(&payload)?;

    let hash = RequestHash::compute_bytes(&body);
    if let Some(response) = claim_or_replay(
        &state.idempotency,
        IdempotencyHeader(Some(key.clone())),
        &hash,
    )
    .await?
    {
        return Ok(response);
    }

    // Payment-level dedup: the idempotency key uniquely identifies a
    // payment; a prior successful initiation is returned as-is.
    if let Some(existing) = state
        .payments
        .get_payment_by_idempotency_key(key.as_str())
        .await
        .map_err(ApiError::from)?
    {
        return Ok(ok_response(&existing));
    }

    // The user must exist (payments are FK-bound to users).
    state
        .users
        .get_user(payload.user_id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::from(pan_africa_pay_domain::error::AppError::not_found(
                "user not found",
            ))
        })?;

    let payment_id = PaymentId::new();
    let phone = PhoneNumber::new(&payload.phone)?;
    let amount = Money {
        amount: payload.amount,
        currency: payload.currency,
    };

    let (rail, status, checkout_id, kotani_tx_id, kotani_customer_key) = match payload.currency {
        Currency::KES => {
            let client = state.mpesa.as_ref().ok_or_else(|| {
                ApiError::from(pan_africa_pay_domain::error::AppError::configuration(
                    "M-Pesa is not configured",
                ))
            })?;
            let request = StkPushRequest {
                business_short_code: String::new(),
                password: String::new(),
                timestamp: String::new(),
                transaction_type: "CustomerPayBillOnline".to_string(),
                amount: payload.amount.to_string(),
                party_a: phone.as_str().to_string(),
                party_b: String::new(),
                phone_number: phone.as_str().to_string(),
                callback_url: state.config.mpesa.callback_url.clone(),
                account_reference: short_ref(&payment_id),
                transaction_desc: "Payin".to_string(),
            };
            let ack = client.stk_push(&request).await?;
            (
                Rail::Mpesa,
                PaymentStatus::Pending,
                ack.checkout_request_id,
                None,
                None,
            )
        }
        Currency::USDC => {
            let client = state.kotani.as_ref().ok_or_else(|| {
                ApiError::from(pan_africa_pay_domain::error::AppError::configuration(
                    "Kotani is not configured",
                ))
            })?;
            let customer_key = match payload.customer_key {
                Some(existing) => existing,
                None => client
                    .create_customer(&pan_africa_pay_kotani::types::CustomerRequest {
                        phone_number: format!("+{}", phone.as_str()),
                        country_code: "KE".to_string(),
                        network: Some("MPESA".to_string()),
                        account_name: None,
                        first_name: None,
                        last_name: None,
                        email: None,
                    })
                    .await?
                    .customer_key
                    .ok_or_else(|| {
                        ApiError::from(pan_africa_pay_domain::error::AppError::external_api(
                            "kotani",
                            "customer registration did not return a customer key",
                        ))
                    })?,
            };
            let deposit = client
                .deposit(&pan_africa_pay_kotani::types::DepositRequest {
                    customer_key: customer_key.clone(),
                    amount: amount_in_major(&amount),
                    wallet_id: state.config.kotani.wallet_id.clone(),
                    callback_url: Some(state.config.kotani.callback_url.clone()),
                    reference_id: payment_id.to_string(),
                    currency: Some("USDC".to_string()),
                })
                .await?;
            (
                Rail::Kotani,
                PaymentStatus::Processing,
                None,
                deposit.id,
                Some(customer_key),
            )
        }
    };

    let payment = Payment {
        id: payment_id,
        user_id: payload.user_id,
        payment_type: PaymentType::Collect,
        rail,
        status,
        amount,
        fee: Money {
            amount: 0,
            currency: payload.currency,
        },
        mpesa_checkout_request_id: checkout_id.clone(),
        mpesa_receipt_number: None,
        kotani_tx_id: kotani_tx_id.clone(),
        callback_payload: None,
        idempotency_key: key.as_str().to_string(),
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    state
        .payments
        .create_payment(&payment)
        .await
        .map_err(ApiError::from)?;

    let ack = PayinAck {
        payment_id: payment.id.to_string(),
        rail,
        status,
        amount: payment.amount.amount,
        currency: payment.amount.currency,
        mpesa_checkout_request_id: checkout_id,
        kotani_tx_id,
        kotani_customer_key,
    };
    let response_body = serde_json::to_value(OkEnvelope { data: ack }).expect("serializable ack");
    state
        .idempotency
        .complete(&key, &hash, response_body.clone(), StatusCode::OK.as_u16())
        .await?;
    Ok((StatusCode::OK, Json(response_body)).into_response())
}

/// Fetch a payment by id.
pub async fn get_payment(
    State(state): State<AppState>,
    Path(id): Path<PaymentId>,
) -> ApiResult<Json<OkEnvelope<Payment>>> {
    let payment = state
        .payments
        .get_payment(id)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| {
            ApiError::from(pan_africa_pay_domain::error::AppError::not_found("payment"))
        })?;
    Ok(ok(payment))
}

/// List payments for a user.
#[derive(Debug, Deserialize)]
pub struct ListPaymentsQuery {
    pub user_id: UserId,
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
}

fn default_limit() -> i64 {
    50
}

pub async fn list_payments(
    State(state): State<AppState>,
    Query(query): Query<ListPaymentsQuery>,
) -> ApiResult<Json<OkEnvelope<Vec<Payment>>>> {
    let payments = state
        .payments
        .list_payments_by_user(query.user_id, query.limit, query.offset)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(payments))
}

/// Create a platform user.
pub async fn create_user(
    State(state): State<AppState>,
    Json(payload): Json<CreateUserPayload>,
) -> ApiResult<Json<OkEnvelope<serde_json::Value>>> {
    if !payload.email.contains('@') {
        return Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation("email must be valid"),
        ));
    }
    let phone = PhoneNumber::new(&payload.phone)?;
    let user = User {
        id: UserId::new(),
        email: payload.email,
        phone,
        created_at: Utc::now(),
        updated_at: Utc::now(),
    };
    state
        .users
        .create_user(&user)
        .await
        .map_err(ApiError::from)?;
    Ok(ok(serde_json::json!({
        "id": user.id.to_string(),
        "email": user.email,
        "phone": user.phone.as_str(),
    })))
}

/// Validate a payin payload.
fn validate_payin(payload: &PayinPayload) -> ApiResult<()> {
    if payload.amount <= 0 {
        return Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation("amount must be positive"),
        ));
    }
    if !payload
        .phone
        .chars()
        .all(|c| c.is_ascii_digit() || c == '+')
    {
        return Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "phone must be E.164 digits with optional leading +",
            ),
        ));
    }
    Ok(())
}

/// Convert minor units to major units (Kotani expects decimal amounts).
fn amount_in_major(money: &Money) -> f64 {
    let divisor = match money.currency {
        Currency::KES => Money::KES_MINOR_UNITS,
        Currency::USDC => Money::USDC_MINOR_UNITS,
    };
    money.amount as f64 / divisor as f64
}

/// Short alphanumeric reference for the STK prompt (max 12 chars).
fn short_ref(id: &PaymentId) -> String {
    let hex = id.0.simple().to_string();
    hex[..12.min(hex.len())].to_string().to_uppercase()
}

/// Build the standard success response for a payment.
fn ok_response(payment: &Payment) -> Response {
    let body = serde_json::to_value(OkEnvelope {
        data: PayinAck {
            payment_id: payment.id.to_string(),
            rail: payment.rail,
            status: payment.status,
            amount: payment.amount.amount,
            currency: payment.amount.currency,
            mpesa_checkout_request_id: payment.mpesa_checkout_request_id.clone(),
            kotani_tx_id: payment.kotani_tx_id.clone(),
            kotani_customer_key: None,
        },
    })
    .expect("serializable ack");
    (StatusCode::OK, Json(body)).into_response()
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

    #[test]
    fn routes_kess_to_mpesa_minor_units() {
        let money = Money {
            amount: 1_500,
            currency: Currency::KES,
        };
        assert_eq!(amount_in_major(&money), 15.0);
    }

    #[test]
    fn routes_usdc_to_major_units() {
        let money = Money {
            amount: 5_000_000,
            currency: Currency::USDC,
        };
        assert_eq!(amount_in_major(&money), 5.0);
    }

    #[test]
    fn short_ref_is_max_twelve_chars() {
        let id = PaymentId::new();
        assert!(short_ref(&id).len() <= 12);
        assert!(short_ref(&id).chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn payin_validation_accepts_valid() {
        let payload = PayinPayload {
            user_id: UserId::new(),
            amount: 100,
            currency: Currency::KES,
            phone: "+254712345678".to_string(),
            customer_key: None,
        };
        assert!(validate_payin(&payload).is_ok());
    }

    #[test]
    fn payin_validation_rejects_bad_values() {
        let base = PayinPayload {
            user_id: UserId::new(),
            amount: 100,
            currency: Currency::KES,
            phone: "+254712345678".to_string(),
            customer_key: None,
        };
        assert!(validate_payin(&PayinPayload {
            amount: 0,
            ..base.clone()
        })
        .is_err());
        assert!(validate_payin(&PayinPayload {
            phone: "not-a-phone".to_string(),
            ..base
        })
        .is_err());
    }
}
