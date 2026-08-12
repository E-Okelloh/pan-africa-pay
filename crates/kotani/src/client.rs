//! The Kotani Pay HTTP client (API v3).
//!
//! Covers the platform flows: registering mobile money customers,
//! initiating deposits and withdrawals, checking transaction status,
//! and listing crypto wallets.

use std::time::Duration;

use reqwest::Client;
use serde::de::DeserializeOwned;
use serde::Serialize;
use tracing::warn;

use crate::config::KotaniConfig;
use crate::error::{KotaniError, KotaniResult};
use crate::types::{
    Customer, CustomerRequest, DepositRequest, DepositResponse, KotaniEnvelope, StatusResponse,
    WithdrawRequest, WithdrawResponse,
};

/// Typed client for the Kotani Pay API.
#[derive(Clone)]
pub struct KotaniClient {
    http: Client,
    config: KotaniConfig,
}

impl KotaniClient {
    /// Build a client from configuration.
    pub fn from_config(config: KotaniConfig) -> KotaniResult<Self> {
        config.validate()?;
        let http = Client::builder()
            .timeout(Duration::from_secs(config.timeout_secs))
            .build()
            .map_err(|e| KotaniError::configuration(format!("failed to build HTTP client: {e}")))?;
        Ok(Self { http, config })
    }

    /// Register (or fetch) a mobile money customer.
    pub async fn create_customer(&self, request: &CustomerRequest) -> KotaniResult<Customer> {
        self.post::<CustomerRequest, Customer>("/api/v3/customer/mobile-money", request)
            .await
    }

    /// Initiate a deposit (fiat -> stablecoin).
    pub async fn deposit(&self, request: &DepositRequest) -> KotaniResult<DepositResponse> {
        self.post::<DepositRequest, DepositResponse>("/api/v3/deposit/mobile-money", request)
            .await
    }

    /// Initiate a withdrawal (stablecoin -> fiat).
    pub async fn withdraw(&self, request: &WithdrawRequest) -> KotaniResult<WithdrawResponse> {
        self.post::<WithdrawRequest, WithdrawResponse>("/api/v3/withdraw/mobile-money", request)
            .await
    }

    /// Poll the status of a deposit by its reference id.
    pub async fn deposit_status(&self, reference_id: &str) -> KotaniResult<StatusResponse> {
        self.get(&format!(
            "/api/v3/deposit/mobile-money/status/{reference_id}"
        ))
        .await
    }

    /// Poll the status of a withdrawal by its reference id.
    pub async fn withdraw_status(&self, reference_id: &str) -> KotaniResult<StatusResponse> {
        self.get(&format!("/api/v3/withdraw/status/{reference_id}"))
            .await
    }

    /// List crypto wallets (used to pick a `wallet_id`).
    pub async fn crypto_wallets(&self) -> KotaniResult<Vec<crate::types::CryptoWallet>> {
        let envelope: KotaniEnvelope<Vec<crate::types::CryptoWallet>> =
            self.request_raw("/api/v3/wallet/crypto").await?;
        envelope
            .data
            .ok_or_else(|| KotaniError::Decode("wallets response missing data".to_string()))
    }

    /// Authenticated POST of a serializable body.
    async fn post<T: Serialize, R: DeserializeOwned>(
        &self,
        path: &str,
        body: &T,
    ) -> KotaniResult<R> {
        let url = format!("{}{}", self.config.base_url, path);
        let response = self
            .http
            .post(&url)
            .bearer_auth(&self.config.api_key)
            .json(body)
            .send()
            .await
            .map_err(KotaniError::from)?;
        let envelope: KotaniEnvelope<R> = self.decode(response).await?;
        if !envelope.success {
            return Err(KotaniError::Provider {
                code: "API_ERROR".to_string(),
                message: envelope.message,
            });
        }
        envelope
            .data
            .ok_or_else(|| KotaniError::Decode(format!("response to {path} is missing data")))
    }

    /// Authenticated GET.
    async fn get<R: DeserializeOwned>(&self, path: &str) -> KotaniResult<R> {
        let url = format!("{}{}", self.config.base_url, path);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(KotaniError::from)?;
        let envelope: KotaniEnvelope<R> = self.decode(response).await?;
        envelope
            .data
            .ok_or_else(|| KotaniError::Decode(format!("response to {path} is missing data")))
    }

    /// Raw envelope request (for list-shaped responses).
    async fn request_raw(
        &self,
        path: &str,
    ) -> KotaniResult<KotaniEnvelope<Vec<crate::types::CryptoWallet>>> {
        let url = format!("{}{}", self.config.base_url, path);
        let response = self
            .http
            .get(&url)
            .bearer_auth(&self.config.api_key)
            .send()
            .await
            .map_err(KotaniError::from)?;
        self.decode(response).await
    }

    /// Decode a Kotani response, mapping non-2xx statuses to errors.
    async fn decode<T: DeserializeOwned>(&self, response: reqwest::Response) -> KotaniResult<T> {
        let status = response.status();
        if status.as_u16() == 401 {
            let body = response.text().await.unwrap_or_default();
            return Err(KotaniError::Authentication(body));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            warn!(provider = "kotani", %status, "Kotani returned non-success status");
            return Err(KotaniError::Http {
                status: status.as_u16(),
                body,
            });
        }
        response
            .json()
            .await
            .map_err(|e| KotaniError::Decode(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_with_invalid_config_fails() {
        assert!(KotaniClient::from_config(KotaniConfig::default()).is_err());
    }

    #[test]
    fn client_with_complete_config_builds() {
        let config = KotaniConfig {
            api_key: "key".to_string(),
            base_url: crate::config::SANDBOX_BASE_URL.to_string(),
            callback_url: "https://example.com/webhooks/kotani".to_string(),
            ..KotaniConfig::default()
        };
        assert!(KotaniClient::from_config(config).is_ok());
    }
}
