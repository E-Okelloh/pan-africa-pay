//! Kotani Pay v3 request/response types (wire contract).

use serde::{Deserialize, Serialize};

/// Generic Kotani envelope: `{ success, message, data }`.
#[derive(Debug, Deserialize)]
pub struct KotaniEnvelope<T> {
    pub success: bool,
    pub message: String,
    pub data: Option<T>,
}

/// Create a mobile money customer.
#[derive(Debug, Clone, Serialize)]
pub struct CustomerRequest {
    #[serde(rename = "phone_number")]
    pub phone_number: String,
    #[serde(rename = "country_code")]
    pub country_code: String,
    pub network: Option<String>,
    #[serde(rename = "account_name")]
    pub account_name: Option<String>,
    #[serde(rename = "first_name")]
    pub first_name: Option<String>,
    #[serde(rename = "last_name")]
    pub last_name: Option<String>,
    pub email: Option<String>,
}

/// A registered mobile money customer.
#[derive(Debug, Clone, Deserialize)]
pub struct Customer {
    pub id: Option<String>,
    #[serde(rename = "phone_number")]
    pub phone_number: String,
    #[serde(rename = "country_code")]
    pub country_code: String,
    pub network: Option<String>,
    #[serde(rename = "customer_key")]
    pub customer_key: Option<String>,
    #[serde(rename = "account_name")]
    pub account_name: Option<String>,
    #[serde(rename = "first_name")]
    pub first_name: Option<String>,
    #[serde(rename = "last_name")]
    pub last_name: Option<String>,
}

/// Initiate a deposit (fiat -> stablecoin).
#[derive(Debug, Clone, Serialize)]
pub struct DepositRequest {
    #[serde(rename = "customer_key")]
    pub customer_key: String,
    pub amount: f64,
    #[serde(rename = "wallet_id")]
    pub wallet_id: String,
    #[serde(rename = "callback_url")]
    pub callback_url: Option<String>,
    #[serde(rename = "reference_id")]
    pub reference_id: String,
    pub currency: Option<String>,
}

/// Deposit acknowledgement.
#[derive(Debug, Clone, Deserialize)]
pub struct DepositResponse {
    pub id: Option<String>,
    pub message: Option<String>,
    #[serde(rename = "reference_id")]
    pub reference_id: String,
    #[serde(rename = "reference_number")]
    pub reference_number: Option<u64>,
    #[serde(rename = "redirect_url")]
    pub redirect_url: Option<String>,
}

/// Initiate a withdrawal (stablecoin -> fiat).
#[derive(Debug, Clone, Serialize)]
pub struct WithdrawRequest {
    #[serde(rename = "customer_key")]
    pub customer_key: String,
    pub amount: f64,
    #[serde(rename = "walletId")]
    pub wallet_id: String,
    #[serde(rename = "callbackUrl")]
    pub callback_url: Option<String>,
    #[serde(rename = "referenceId")]
    pub reference_id: String,
    pub currency: Option<String>,
    pub network: Option<String>,
}

/// Withdrawal acknowledgement.
#[derive(Debug, Clone, Deserialize)]
pub struct WithdrawResponse {
    pub id: Option<String>,
    pub message: Option<String>,
    #[serde(rename = "referenceId")]
    pub reference_id: String,
    #[serde(rename = "referenceNumber")]
    pub reference_number: Option<u64>,
}

/// Transaction status.
#[derive(Debug, Clone, Deserialize)]
pub struct StatusResponse {
    pub status: Option<String>,
    pub message: Option<String>,
    pub reference_id: Option<String>,
    #[serde(rename = "referenceId")]
    pub reference_id_camel: Option<String>,
    pub amount: Option<f64>,
    pub currency: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

/// Crypto wallet returned by `GET /api/v3/wallet/crypto`.
#[derive(Debug, Clone, Deserialize)]
pub struct CryptoWallet {
    pub id: Option<String>,
    pub currency: Option<String>,
    pub network: Option<String>,
    pub address: Option<String>,
    pub balance: Option<f64>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_deposit_envelope() {
        let json = serde_json::json!({
            "success": true,
            "message": "Deposit has been successfully created.",
            "data": {
                "id": "dep_1",
                "message": "pending",
                "reference_id": "ref-123",
                "reference_number": 42,
                "redirect_url": "https://kotanipay.com/redirect"
            }
        });
        let envelope: KotaniEnvelope<DepositResponse> =
            serde_json::from_value(json).expect("parse");
        let data = envelope.data.expect("data");
        assert_eq!(data.reference_id, "ref-123");
        assert_eq!(data.reference_number, Some(42));
    }

    #[test]
    fn withdraw_request_uses_camel_case_id_fields() {
        let req = WithdrawRequest {
            customer_key: "ck".to_string(),
            amount: 10.0,
            wallet_id: "w".to_string(),
            callback_url: None,
            reference_id: "r1".to_string(),
            currency: None,
            network: Some("MPESA".to_string()),
        };
        let value = serde_json::to_value(&req).expect("serialize");
        assert!(value.get("walletId").is_some());
        assert!(value.get("callbackUrl").is_some());
        assert!(value.get("referenceId").is_some());
        assert!(value.get("wallet_id").is_none());
    }

    #[test]
    fn deposit_request_uses_snake_case_id_fields() {
        let req = DepositRequest {
            customer_key: "ck".to_string(),
            amount: 10.0,
            wallet_id: "w".to_string(),
            callback_url: None,
            reference_id: "r1".to_string(),
            currency: None,
        };
        let value = serde_json::to_value(&req).expect("serialize");
        assert!(value.get("wallet_id").is_some());
        assert!(value.get("reference_id").is_some());
    }
}
