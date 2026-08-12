//! HTTP error mapping.
//!
//! Converts domain [`AppError`] values into JSON error responses using
//! the API's stable error envelope:
//!
//! ```json
//! { "error": { "code": "VALIDATION_ERROR", "message": "..." } }
//! ```

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

use pan_africa_pay_domain::error::{AppError, ErrorCode};
use pan_africa_pay_kotani::KotaniError;
use pan_africa_pay_mpesa::MpesaError;

/// HTTP layer wrapper around a domain [`AppError`].
///
/// A newtype is required because `IntoResponse` (axum) cannot be
/// implemented for the foreign `AppError` type directly. Handlers
/// return `Result<T, ApiError>` and use `?`; the conversion is
/// automatic via [`From`].
#[derive(Debug)]
pub struct ApiError(pub AppError);

impl From<AppError> for ApiError {
    fn from(err: AppError) -> Self {
        Self(err)
    }
}

impl From<MpesaError> for ApiError {
    fn from(err: MpesaError) -> Self {
        Self(AppError::from(err))
    }
}

impl From<KotaniError> for ApiError {
    fn from(err: KotaniError) -> Self {
        Self(AppError::from(err))
    }
}

impl std::fmt::Display for ApiError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for ApiError {}

/// Convenient result alias for handlers.
pub type ApiResult<T> = Result<T, ApiError>;

/// Stable JSON envelope for API errors.
#[derive(Debug, Clone, Serialize)]
pub struct ApiErrorBody {
    pub code: String,
    pub message: String,
}

/// JSON envelope wrapper.
#[derive(Debug, Serialize)]
pub struct ApiErrorEnvelope {
    pub error: ApiErrorBody,
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let status = status_for(&self.0);
        let body = ApiErrorEnvelope {
            error: ApiErrorBody {
                code: format_code(&self.0),
                message: self.0.message,
            },
        };
        (status, Json(body)).into_response()
    }
}

/// HTTP status code for an application error.
pub fn status_for(err: &AppError) -> StatusCode {
    match err.code {
        ErrorCode::ValidationError => StatusCode::BAD_REQUEST,
        ErrorCode::NotFound => StatusCode::NOT_FOUND,
        ErrorCode::Conflict => StatusCode::CONFLICT,
        ErrorCode::InsufficientFunds => StatusCode::UNPROCESSABLE_ENTITY,
        ErrorCode::ExternalApiError => StatusCode::BAD_GATEWAY,
        ErrorCode::IdempotencyConflict => StatusCode::CONFLICT,
        ErrorCode::IdempotencyExpired => StatusCode::CONFLICT,
        ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
        ErrorCode::Forbidden => StatusCode::FORBIDDEN,
        ErrorCode::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        ErrorCode::InternalError => StatusCode::INTERNAL_SERVER_ERROR,
        ErrorCode::ServiceUnavailable => StatusCode::SERVICE_UNAVAILABLE,
        ErrorCode::ConfigurationError => StatusCode::INTERNAL_SERVER_ERROR,
    }
}

/// SCREAMING_SNAKE_CASE code as exposed to API clients.
fn format_code(err: &AppError) -> String {
    match err.code {
        ErrorCode::ValidationError => "VALIDATION_ERROR",
        ErrorCode::NotFound => "NOT_FOUND",
        ErrorCode::Conflict => "CONFLICT",
        ErrorCode::InsufficientFunds => "INSUFFICIENT_FUNDS",
        ErrorCode::ExternalApiError => "EXTERNAL_API_ERROR",
        ErrorCode::IdempotencyConflict => "IDEMPOTENCY_CONFLICT",
        ErrorCode::IdempotencyExpired => "IDEMPOTENCY_EXPIRED",
        ErrorCode::Unauthorized => "UNAUTHORIZED",
        ErrorCode::Forbidden => "FORBIDDEN",
        ErrorCode::RateLimited => "RATE_LIMITED",
        ErrorCode::InternalError => "INTERNAL_ERROR",
        ErrorCode::ServiceUnavailable => "SERVICE_UNAVAILABLE",
        ErrorCode::ConfigurationError => "CONFIGURATION_ERROR",
    }
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_map_to_status_codes() {
        use pan_africa_pay_domain::error::ErrorCode as Code;
        let cases = [
            (AppError::validation("x"), StatusCode::BAD_REQUEST),
            (AppError::new(Code::NotFound, "x"), StatusCode::NOT_FOUND),
            (
                AppError::new(Code::IdempotencyConflict, "x"),
                StatusCode::CONFLICT,
            ),
            (
                AppError::new(Code::RateLimited, "x"),
                StatusCode::TOO_MANY_REQUESTS,
            ),
            (
                AppError::new(Code::ExternalApiError, "x"),
                StatusCode::BAD_GATEWAY,
            ),
            (
                AppError::new(Code::InsufficientFunds, "x"),
                StatusCode::UNPROCESSABLE_ENTITY,
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(status_for(&err), expected);
        }
    }

    #[test]
    fn response_body_uses_stable_envelope() {
        use http_body_util::BodyExt;

        let rt = tokio::runtime::Runtime::new().expect("runtime");
        let err = ApiError::from(AppError::validation("phone must be E.164"));
        let response = err.into_response();
        let status = response.status();
        let bytes = rt
            .block_on(response.into_body().collect())
            .expect("collect body")
            .to_bytes();
        let json: serde_json::Value = serde_json::from_slice(&bytes).expect("valid json response");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(json["error"]["code"], "VALIDATION_ERROR");
        assert_eq!(json["error"]["message"], "phone must be E.164");
    }
}
