//! Error types and result aliases for the application.

use std::fmt;
use thiserror::Error;

/// Application error codes for programmatic handling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    /// Input validation failed.
    ValidationError,
    /// Resource not found.
    NotFound,
    /// Conflict with existing resource (e.g., duplicate key).
    Conflict,
    /// Insufficient funds for operation.
    InsufficientFunds,
    /// External API error (M-Pesa, Kotani, etc.).
    ExternalApiError,
    /// Idempotency key collision with different request.
    IdempotencyConflict,
    /// Idempotency key expired or not found.
    IdempotencyExpired,
    /// Authentication failed.
    Unauthorized,
    /// Authorization failed.
    Forbidden,
    /// Rate limit exceeded.
    RateLimited,
    /// Internal server error.
    InternalError,
    /// Service temporarily unavailable.
    ServiceUnavailable,
    /// Configuration error.
    ConfigurationError,
}

/// Application error with structured context.
#[derive(Debug, Error)]
#[error("{code}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    pub message: String,
    #[source]
    pub source: Option<Box<dyn std::error::Error + Send + Sync>>,
    pub context: serde_json::Value,
}

impl AppError {
    /// Create a new application error.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            source: None,
            context: serde_json::Value::Null,
        }
    }

    /// Add context to the error.
    pub fn with_context(mut self, key: &str, value: impl serde::Serialize) -> Self {
        if self.context.is_null() {
            self.context = serde_json::json!({});
        }
        if let serde_json::Value::Object(ref mut map) = self.context {
            map.insert(
                key.to_string(),
                serde_json::to_value(value).unwrap_or(serde_json::Value::Null),
            );
        }
        self
    }

    /// Add source error.
    pub fn with_source(mut self, source: impl std::error::Error + Send + Sync + 'static) -> Self {
        self.source = Some(Box::new(source));
        self
    }

    /// Validation error helper.
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ValidationError, message)
    }

    /// Not found error helper.
    pub fn not_found(resource: impl Into<String>) -> Self {
        Self::new(
            ErrorCode::NotFound,
            format!("{} not found", resource.into()),
        )
    }

    /// Conflict error helper.
    pub fn conflict(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Conflict, message)
    }

    /// Insufficient funds error helper.
    pub fn insufficient_funds(available: i64, required: i64) -> Self {
        Self::new(ErrorCode::InsufficientFunds, "Insufficient funds")
            .with_context("available", available)
            .with_context("required", required)
    }

    /// External API error helper.
    pub fn external_api(provider: impl Into<String>, message: impl Into<String>) -> Self {
        let provider = provider.into();
        Self::new(
            ErrorCode::ExternalApiError,
            format!("{provider} error: {}", message.into()),
        )
        .with_context("provider", provider)
    }

    /// Idempotency conflict helper.
    pub fn idempotency_conflict(key: &str) -> Self {
        Self::new(
            ErrorCode::IdempotencyConflict,
            "Idempotency key used with different request",
        )
        .with_context("idempotency_key", key)
    }

    /// Idempotency expired helper.
    pub fn idempotency_expired(key: &str) -> Self {
        Self::new(ErrorCode::IdempotencyExpired, "Idempotency key expired")
            .with_context("idempotency_key", key)
    }

    /// Unauthorized error helper.
    pub fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Unauthorized, message)
    }

    /// Forbidden error helper.
    pub fn forbidden(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Forbidden, message)
    }

    /// Rate limited error helper.
    pub fn rate_limited(retry_after_secs: u64) -> Self {
        Self::new(ErrorCode::RateLimited, "Rate limit exceeded")
            .with_context("retry_after_secs", retry_after_secs)
    }

    /// Internal error helper.
    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::InternalError, message)
    }

    /// Service unavailable helper.
    pub fn service_unavailable(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ServiceUnavailable, message)
    }

    /// Configuration error helper.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::ConfigurationError, message)
    }
}

impl fmt::Display for ErrorCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:?}", self)
    }
}

/// Type alias for application results.
pub type AppResult<T> = Result<T, AppError>;

/// Extension trait for converting external errors to AppError.
pub trait IntoAppResult<T> {
    fn into_app_result(self, code: ErrorCode, message: impl Into<String>) -> AppResult<T>;
}

impl<T, E: std::error::Error + Send + Sync + 'static> IntoAppResult<T> for Result<T, E> {
    fn into_app_result(self, code: ErrorCode, message: impl Into<String>) -> AppResult<T> {
        self.map_err(|e| AppError::new(code, message).with_source(e))
    }
}
