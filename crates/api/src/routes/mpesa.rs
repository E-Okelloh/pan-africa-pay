//! M-Pesa endpoints: STK push initiation, status query, and the Daraja
//! callback webhook.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};

use pan_africa_pay_domain::idempotency::RequestHash;
use pan_africa_pay_mpesa::types::StkPushRequest;
use tracing::info;

use crate::error::{ApiError, ApiResult};
use crate::idempotency::{claim_or_replay, IdempotencyHeader};
use crate::routes::{ok, OkEnvelope};
use crate::state::AppState;

/// Body accepted by `POST /mpesa/stk/push`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StkPushPayload {
    /// Amount in KES (whole number string, e.g. `"150"`).
    pub amount: String,
    /// Customer phone in E.164 format, e.g. `254712345678`.
    pub phone_number: String,
    /// Reference shown on the customer's STK prompt (max 12 chars).
    pub account_reference: String,
    /// Short description shown to the customer (max 13 chars).
    #[serde(default = "default_transaction_desc")]
    pub transaction_desc: String,
}

fn default_transaction_desc() -> String {
    "Payment".to_string()
}

/// Initiate an STK push prompt on the customer's phone.
///
/// Accepts an optional `Idempotency-Key` header: retries with the same
/// key and body replay the original response.
pub async fn stk_push(
    State(state): State<AppState>,
    header: IdempotencyHeader,
    body: axum::body::Bytes,
) -> ApiResult<Response> {
    let payload: StkPushPayload = parse_json(&body)?;
    validate_phone(&payload.phone_number)?;
    validate_amount(&payload.amount)?;

    let hash = RequestHash::compute_bytes(&body);
    if let Some(response) = claim_or_replay(&state.idempotency, header.clone(), &hash).await? {
        return Ok(response);
    }

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
        amount: payload.amount,
        party_a: payload.phone_number.clone(),
        party_b: String::new(),
        phone_number: payload.phone_number,
        callback_url: state.config.mpesa.callback_url.clone(),
        account_reference: payload.account_reference,
        transaction_desc: payload.transaction_desc,
    };

    let ack = client.stk_push(&request).await?;
    let body = serde_json::to_value(OkEnvelope {
        data: StkPushAck {
            merchant_request_id: ack.merchant_request_id,
            checkout_request_id: ack.checkout_request_id,
            response_code: ack.response_code,
            response_description: ack.response_description,
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

/// Query the status of an STK push by its checkout request id.
pub async fn stk_query(
    State(state): State<AppState>,
    Path(checkout_request_id): Path<String>,
) -> ApiResult<Json<OkEnvelope<StkQueryAck>>> {
    let client = state.mpesa.as_ref().ok_or_else(|| {
        ApiError::from(pan_africa_pay_domain::error::AppError::configuration(
            "M-Pesa is not configured",
        ))
    })?;
    let result = client.stk_query(&checkout_request_id).await?;
    Ok(ok(StkQueryAck {
        checkout_request_id: result.checkout_request_id,
        result_code: result.result_code,
        result_desc: result.result_desc,
    }))
}

/// Acknowledgement for an STK query.
#[derive(Debug, Clone, Serialize)]
pub struct StkQueryAck {
    pub checkout_request_id: Option<String>,
    pub result_code: Option<String>,
    pub result_desc: Option<String>,
}

/// Acknowledgement returned to the API caller.
#[derive(Debug, Clone, Serialize)]
pub struct StkPushAck {
    pub merchant_request_id: Option<String>,
    pub checkout_request_id: Option<String>,
    pub response_code: String,
    pub response_description: String,
}

/// E.164 phone validation: `254` + 9 digits.
fn validate_phone(phone: &str) -> ApiResult<()> {
    if phone.len() == 12
        && phone.starts_with("254")
        && phone[3..].chars().all(|c| c.is_ascii_digit())
    {
        Ok(())
    } else {
        Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "phone_number must be E.164 (2547XXXXXXXX)",
            ),
        ))
    }
}

/// KES amount must be a whole number between 1 and 150,000.
fn validate_amount(amount: &str) -> ApiResult<()> {
    match amount.parse::<u64>() {
        Ok(value) if (1..=150_000).contains(&value) => Ok(()),
        _ => Err(ApiError::from(
            pan_africa_pay_domain::error::AppError::validation(
                "amount must be a whole number between 1 and 150,000 KES",
            ),
        )),
    }
}

/// Top-level Daraja callback envelope.
#[derive(Debug, Deserialize)]
pub struct StkCallbackEnvelope {
    #[serde(rename = "Body")]
    pub body: CallbackBody,
}

#[derive(Debug, Deserialize)]
pub struct CallbackBody {
    #[serde(rename = "stkCallback")]
    pub stk_callback: StkCallback,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct StkCallback {
    #[serde(rename = "MerchantRequestID")]
    pub merchant_request_id: String,
    #[serde(rename = "CheckoutRequestID")]
    pub checkout_request_id: String,
    #[serde(rename = "ResultCode")]
    pub result_code: i64,
    #[serde(rename = "ResultDesc")]
    pub result_desc: String,
    #[serde(rename = "CallbackMetadata", default)]
    pub callback_metadata: Option<CallbackMetadata>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct CallbackMetadata {
    #[serde(rename = "Item", default)]
    pub item: Vec<MetadataItem>,
}

#[derive(Debug, Deserialize, Serialize)]
pub struct MetadataItem {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "Value")]
    pub value: Option<serde_json::Value>,
}

/// Ack payload Daraja expects back on a successful webhook.
#[derive(Debug, Serialize)]
pub struct WebhookAck {
    pub result_code: i64,
    pub result_desc: String,
}

/// Handle the Daraja STK callback.
///
/// Daraja retries non-200 responses, so this handler always responds
/// 200 after logging and recording the result.
pub async fn webhook(
    State(state): State<AppState>,
    payload: Result<Json<StkCallbackEnvelope>, axum::extract::rejection::JsonRejection>,
) -> Result<Json<WebhookAck>, (axum::http::StatusCode, String)> {
    let envelope = payload.map_err(|err| {
        let message = format!("invalid webhook body: {err}");
        tracing::warn!(provider = "mpesa", "{message}");
        (axum::http::StatusCode::BAD_REQUEST, message)
    })?;

    let callback = &envelope.body.stk_callback;
    let metadata: Vec<(String, Option<serde_json::Value>)> = callback
        .callback_metadata
        .as_ref()
        .map(|m| {
            m.item
                .iter()
                .map(|i| (i.name.clone(), i.value.clone()))
                .collect()
        })
        .unwrap_or_default();

    let receipt = metadata
        .iter()
        .find(|(name, _)| name == "MpesaReceiptNumber")
        .and_then(|(_, value)| value.as_ref().and_then(|v| v.as_str()))
        .map(str::to_string);

    info!(
        provider = "mpesa",
        merchant_request_id = %callback.merchant_request_id,
        checkout_request_id = %callback.checkout_request_id,
        result_code = callback.result_code,
        result_desc = %callback.result_desc,
        "received STK callback"
    );

    // Reconcile: link the callback to a payment and update its status.
    // Reconciliation is best-effort at callback time: storage failures
    // are logged and still acknowledged, so Daraja never retries and
    // the webhook contract (always 200) is preserved.
    match state
        .payments
        .get_payment_by_mpesa_checkout_request_id(&callback.checkout_request_id)
        .await
    {
        Ok(Some(payment)) => {
            let status = if callback.result_code == 0 {
                pan_africa_pay_domain::types::PaymentStatus::Completed
            } else {
                pan_africa_pay_domain::types::PaymentStatus::Failed
            };
            if let Err(err) = state
                .payments
                .update_payment_status(payment.id, status, receipt, None)
                .await
            {
                tracing::error!(provider = "mpesa", "payment update failed: {err}");
            }
            if let Err(err) = state
                .payments
                .attach_callback_payload(
                    payment.id,
                    serde_json::to_value(callback).unwrap_or_default(),
                )
                .await
            {
                tracing::error!(provider = "mpesa", "callback attach failed: {err}");
            }
        }
        Ok(None) => {
            tracing::warn!(
                provider = "mpesa",
                checkout_request_id = %callback.checkout_request_id,
                "callback received for unknown payment"
            );
        }
        Err(err) => {
            tracing::error!(
                provider = "mpesa",
                checkout_request_id = %callback.checkout_request_id,
                "payment lookup failed: {err}"
            );
        }
    }

    Ok(Json(WebhookAck {
        result_code: 0,
        result_desc: "Success".to_string(),
    }))
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
    fn phone_validation_accepts_e164() {
        assert!(validate_phone("254712345678").is_ok());
    }

    #[test]
    fn phone_validation_rejects_bad_format() {
        assert!(validate_phone("0712345678").is_err());
        assert!(validate_phone("25471234567").is_err());
        assert!(validate_phone("2547123456789").is_err());
        assert!(validate_phone("25471234567a").is_err());
    }

    #[test]
    fn amount_validation_accepts_whole_kess() {
        assert!(validate_amount("1").is_ok());
        assert!(validate_amount("150").is_ok());
        assert!(validate_amount("150000").is_ok());
    }

    #[test]
    fn amount_validation_rejects_bad_values() {
        assert!(validate_amount("0").is_err());
        assert!(validate_amount("150001").is_err());
        assert!(validate_amount("1.5").is_err());
        assert!(validate_amount("abc").is_err());
    }

    #[test]
    fn parses_daraja_callback_envelope() {
        let json = serde_json::json!({
            "Body": {
                "stkCallback": {
                    "MerchantRequestID": "m1",
                    "CheckoutRequestID": "ws_CO_1",
                    "ResultCode": 0,
                    "ResultDesc": "Success",
                    "CallbackMetadata": {
                        "Item": [
                            {"Name": "Amount", "Value": 150.0},
                            {"Name": "MpesaReceiptNumber", "Value": "NLJ7RT61SV"},
                            {"Name": "PhoneNumber", "Value": "254712345678"}
                        ]
                    }
                }
            }
        });
        let envelope: StkCallbackEnvelope = serde_json::from_value(json).expect("parse callback");
        assert_eq!(envelope.body.stk_callback.result_code, 0);
        let items = &envelope
            .body
            .stk_callback
            .callback_metadata
            .expect("metadata")
            .item;
        assert_eq!(items.len(), 3);
        assert_eq!(items[1].name, "MpesaReceiptNumber");
        assert_eq!(
            items[1].value.as_ref().and_then(|v| v.as_str()),
            Some("NLJ7RT61SV")
        );
    }

    #[test]
    fn parses_failed_callback_without_metadata() {
        let json = serde_json::json!({
            "Body": {
                "stkCallback": {
                    "MerchantRequestID": "m1",
                    "CheckoutRequestID": "ws_CO_2",
                    "ResultCode": 1032,
                    "ResultDesc": "Request cancelled by user"
                }
            }
        });
        let envelope: StkCallbackEnvelope = serde_json::from_value(json).expect("parse callback");
        assert_eq!(envelope.body.stk_callback.result_code, 1032);
        assert!(envelope.body.stk_callback.callback_metadata.is_none());
    }
}
