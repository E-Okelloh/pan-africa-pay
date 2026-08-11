//! The M-Pesa Daraja HTTP client.
//!
//! Wraps authentication, request signing, and the three Daraja
//! endpoints the platform uses. All methods return typed results or
//! [`MpesaError`].

use std::time::Duration;

use chrono::Utc;
use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::{debug, warn};

use crate::auth::TokenCache;
use crate::config::MpesaConfig;
use crate::error::{MpesaError, MpesaResult};
use crate::security::{daraja_timestamp, stk_password};
use crate::types::{
    B2cRequest, B2cResponse, StkPushRequest, StkPushResponse, StkQueryRequest, StkQueryResponse,
};

/// Typed client for the Daraja API.
#[derive(Clone)]
pub struct MpesaClient {
    http: Client,
    tokens: TokenCache,
    config: MpesaConfig,
}

impl MpesaClient {
    /// Build a client from configuration.
    pub fn from_config(config: MpesaConfig) -> MpesaResult<Self> {
        config.validate()?;
        Self::new(config)
    }

    /// Build a client without validating credentials upfront.
    pub fn new(config: MpesaConfig) -> MpesaResult<Self> {
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| MpesaError::configuration(format!("failed to build HTTP client: {e}")))?;

        let tokens = TokenCache::new(
            http.clone(),
            config.environment,
            config.consumer_key.clone(),
            config.consumer_secret.clone(),
            config.token_ttl_secs,
            config.base_url_override.clone(),
        );

        Ok(Self {
            http,
            tokens,
            config,
        })
    }

    /// Send an STK Push prompt to the customer's phone.
    ///
    /// The password and timestamp are computed from the current time
    /// and the configured passkey; the caller supplies the commercial
    /// fields (amount, parties, callback).
    ///
    /// Returns the acknowledgement from Daraja; the actual result
    /// arrives later via the callback URL.
    pub async fn stk_push(&self, request: &StkPushRequest) -> MpesaResult<StkPushResponse> {
        let timestamp = daraja_timestamp(Utc::now());
        let password = stk_password(&self.config.short_code, &self.config.passkey, &timestamp);
        let mut request = request.clone();
        request.timestamp = timestamp;
        request.password = password;
        request.party_b = self.config.short_code.clone();

        let response = self
            .post_json::<StkPushRequest, StkPushResponse>(
                "/mpesa/stkpush/v1/processrequest",
                &request,
            )
            .await?;
        if !response.is_accepted() {
            return Err(MpesaError::Provider {
                code: response.response_code.clone(),
                message: response.response_description.clone(),
            });
        }
        Ok(response)
    }

    /// Poll the status of an STK Push transaction.
    pub async fn stk_query(&self, checkout_request_id: &str) -> MpesaResult<StkQueryResponse> {
        let timestamp = daraja_timestamp(Utc::now());
        let password = stk_password(&self.config.short_code, &self.config.passkey, &timestamp);
        let request = StkQueryRequest {
            business_short_code: self.config.short_code.clone(),
            password,
            timestamp,
            checkout_request_id: checkout_request_id.to_string(),
        };
        self.post_json::<StkQueryRequest, StkQueryResponse>(
            "/mpesa/stkpushquery/v1/query",
            &request,
        )
        .await
    }

    /// Initiate a B2C payout.
    ///
    /// - `security_credential`: base64 of the RSA-OAEP encrypted
    ///   consumer secret (see [`crate::security::security_credential`]).
    /// - `command_id`: `BusinessPayment`, `SalaryPayment`, or
    ///   `PromotionPayment`.
    pub async fn b2c(
        &self,
        request: &B2cRequest,
        security_credential: &str,
    ) -> MpesaResult<B2cResponse> {
        let mut request = request.clone();
        request.security_credential = security_credential.to_string();

        let response = self
            .post_json::<B2cRequest, B2cResponse>("/mpesa/b2c/v1/paymentrequest", &request)
            .await?;
        if !response.is_accepted() {
            return Err(MpesaError::Provider {
                code: response.response_code.clone(),
                message: response.response_description.clone(),
            });
        }
        Ok(response)
    }

    /// Authenticated POST of a serializable body to a Daraja endpoint.
    ///
    /// On `401` the cached token is invalidated and one retry is made
    /// with a fresh token.
    async fn post_json<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> MpesaResult<R> {
        let token = self.tokens.token().await?;
        let url = format!("{}{}", self.config.base_url(), path);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&token)
            .json(body)
            .send()
            .await
            .map_err(MpesaError::from)?;

        if response.status().as_u16() == 401 {
            debug!(
                provider = "mpesa",
                "token rejected (401), refreshing and retrying once"
            );
            self.tokens.invalidate();
            let token = self.tokens.token().await?;
            let response = self
                .http
                .post(&url)
                .bearer_auth(&token)
                .json(body)
                .send()
                .await
                .map_err(MpesaError::from)?;
            return self.decode(response).await;
        }
        self.decode(response).await
    }

    /// Decode a Daraja response, mapping non-2xx statuses to errors.
    async fn decode<R: DeserializeOwned>(&self, response: reqwest::Response) -> MpesaResult<R> {
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!(provider = "mpesa", %status, "Daraja returned non-success status");
            return Err(MpesaError::Http {
                status: status.as_u16(),
                body,
            });
        }
        response
            .json()
            .await
            .map_err(|e| MpesaError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_with_invalid_config_fails() {
        let err = MpesaClient::from_config(MpesaConfig::default());
        assert!(err.is_err());
    }

    #[test]
    fn client_with_complete_config_builds() {
        use crate::config::Environment;

        let config = MpesaConfig {
            consumer_key: "key".to_string(),
            consumer_secret: "secret".to_string(),
            passkey: "passkey".to_string(),
            short_code: "174379".to_string(),
            callback_url: "https://example.com/cb".to_string(),
            environment: Environment::Sandbox,
            ..MpesaConfig::default()
        };
        let client = MpesaClient::from_config(config).expect("client");
        assert_eq!(
            client.config.environment.base_url(),
            "https://sandbox.safaricom.co.ke"
        );
    }
}
