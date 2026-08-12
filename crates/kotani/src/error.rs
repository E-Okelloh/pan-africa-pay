//! Kotani Pay client error types.

use pan_africa_pay_domain::error::AppError;

/// Errors raised by the Kotani Pay client.
#[derive(Debug, thiserror::Error)]
pub enum KotaniError {
    /// Configuration is missing required values.
    #[error("Kotani configuration error: {0}")]
    Configuration(String),
    /// Request timed out.
    #[error("Kotani request timed out: {0}")]
    Timeout(String),
    /// HTTP transport failure (network, TLS, proxy).
    #[error("Kotani transport error: {0}")]
    Transport(String),
    /// Kotani returned a non-success HTTP status.
    #[error("Kotani returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// Kotani returned a JSON body that could not be parsed.
    #[error("Kotani response could not be decoded: {0}")]
    Decode(String),
    /// Kotani explicitly rejected the request.
    #[error("Kotani error {code}: {message}")]
    Provider { code: String, message: String },
    /// The API key was rejected.
    #[error("Kotani authentication failed: {0}")]
    Authentication(String),
}

impl KotaniError {
    /// Configuration error helper.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }
}

/// Result alias for Kotani operations.
pub type KotaniResult<T> = Result<T, KotaniError>;

impl From<KotaniError> for AppError {
    fn from(err: KotaniError) -> Self {
        AppError::external_api("kotani", err.to_string())
    }
}

impl From<reqwest::Error> for KotaniError {
    fn from(err: reqwest::Error) -> Self {
        if err.is_timeout() {
            Self::Timeout(err.to_string())
        } else {
            Self::Transport(err.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kotani_error_maps_to_app_error() {
        let err = KotaniError::Authentication("invalid api key".to_string());
        let app: AppError = err.into();
        assert_eq!(
            app.code,
            pan_africa_pay_domain::error::ErrorCode::ExternalApiError
        );
    }
}
