//! M-Pesa client error types.

use pan_africa_pay_domain::error::AppError;

/// Errors raised by the M-Pesa Daraja client.
#[derive(Debug, thiserror::Error)]
pub enum MpesaError {
    /// Configuration is missing required values.
    #[error("M-Pesa configuration error: {0}")]
    Configuration(String),
    /// Request timed out.
    #[error("M-Pesa request timed out: {0}")]
    Timeout(String),
    /// HTTP transport failure (network, TLS, proxy).
    #[error("M-Pesa transport error: {0}")]
    Transport(String),
    /// Daraja returned a non-success HTTP status.
    #[error("M-Pesa returned HTTP {status}: {body}")]
    Http { status: u16, body: String },
    /// Daraja returned a JSON body that could not be parsed.
    #[error("M-Pesa response could not be decoded: {0}")]
    Decode(String),
    /// Daraja explicitly rejected the request with an error code.
    #[error("M-Pesa error {code}: {message}")]
    Provider { code: String, message: String },
    /// The token endpoint denied the consumer credentials.
    #[error("M-Pesa authentication failed: {0}")]
    Authentication(String),
}

impl MpesaError {
    /// Configuration error helper.
    pub fn configuration(message: impl Into<String>) -> Self {
        Self::Configuration(message.into())
    }
}

/// Result alias for M-Pesa operations.
pub type MpesaResult<T> = Result<T, MpesaError>;

impl From<MpesaError> for AppError {
    fn from(err: MpesaError) -> Self {
        AppError::external_api("mpesa", err.to_string())
    }
}

impl From<reqwest::Error> for MpesaError {
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
    fn mpesa_error_maps_to_app_error() {
        let err = MpesaError::Provider {
            code: "401".to_string(),
            message: "unable to raise an exception".to_string(),
        };
        let app: AppError = err.into();
        assert_eq!(
            app.code,
            pan_africa_pay_domain::error::ErrorCode::ExternalApiError
        );
    }
}
