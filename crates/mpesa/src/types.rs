//! Typed request/response payloads for the Daraja API.
//!
//! Field ordering follows the JSON required by Daraja; `serde_json`
//! preserves declaration order, and all fields are unflattened so the
//! wire format matches the specification.

use serde::{Deserialize, Serialize};

/// STK Push (Lipa Na M-Pesa Online) request.
///
/// Prompts the customer's phone with a payment request. The `password`
/// field is a base64 of `Shortcode + Passkey + Timestamp`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StkPushRequest {
    #[serde(rename = "BusinessShortCode")]
    pub business_short_code: String,
    #[serde(rename = "Password")]
    pub password: String,
    #[serde(rename = "Timestamp")]
    pub timestamp: String,
    /// `CustomerPayBillOnline` for standard paybill collection.
    #[serde(rename = "TransactionType")]
    pub transaction_type: String,
    /// Amount in major units as a decimal string (e.g. "150.00").
    #[serde(rename = "Amount")]
    pub amount: String,
    /// Customer phone number in E.164 format without the `+`.
    #[serde(rename = "PartyA")]
    pub party_a: String,
    /// Business short code (same as `BusinessShortCode`).
    #[serde(rename = "PartyB")]
    pub party_b: String,
    /// Customer phone number (same as `PartyA`).
    #[serde(rename = "PhoneNumber")]
    pub phone_number: String,
    /// Public callback URL for the payment result.
    #[serde(rename = "CallBackURL")]
    pub callback_url: String,
    /// Account reference visible to the customer on their phone.
    #[serde(rename = "AccountReference")]
    pub account_reference: String,
    /// Description visible to the customer.
    #[serde(rename = "TransactionDesc")]
    pub transaction_desc: String,
}

/// STK Push response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StkPushResponse {
    #[serde(rename = "MerchantRequestID")]
    pub merchant_request_id: Option<String>,
    #[serde(rename = "CheckoutRequestID")]
    pub checkout_request_id: Option<String>,
    #[serde(rename = "ResponseCode")]
    pub response_code: String,
    #[serde(rename = "ResponseDescription")]
    pub response_description: String,
    #[serde(rename = "CustomerMessage")]
    pub customer_message: Option<String>,
}

impl StkPushResponse {
    /// True if Daraja accepted the request for processing.
    pub fn is_accepted(&self) -> bool {
        self.response_code == "0"
    }
}

/// STK Push query request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StkQueryRequest {
    #[serde(rename = "BusinessShortCode")]
    pub business_short_code: String,
    #[serde(rename = "Password")]
    pub password: String,
    #[serde(rename = "Timestamp")]
    pub timestamp: String,
    #[serde(rename = "CheckoutRequestID")]
    pub checkout_request_id: String,
}

/// STK Push query response.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StkQueryResponse {
    #[serde(rename = "ResponseCode")]
    pub response_code: String,
    #[serde(rename = "ResponseDescription")]
    pub response_description: String,
    #[serde(rename = "MerchantRequestID")]
    pub merchant_request_id: Option<String>,
    #[serde(rename = "CheckoutRequestID")]
    pub checkout_request_id: Option<String>,
    #[serde(rename = "ResultCode")]
    pub result_code: Option<String>,
    #[serde(rename = "ResultDesc")]
    pub result_desc: Option<String>,
    #[serde(rename = "MpesaReceiptNumber")]
    pub mpesa_receipt_number: Option<String>,
    #[serde(rename = "TransactionDate")]
    pub transaction_date: Option<String>,
    #[serde(rename = "PhoneNumber")]
    pub phone_number: Option<String>,
    #[serde(rename = "Amount")]
    pub amount: Option<String>,
}

/// B2C payout request.
///
/// The `security_credential` is the base64 of the B2C short code's
/// public certificate, encrypted with RSA-OAEP. See [`crate::security`].
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct B2cRequest {
    #[serde(rename = "InitiatorName")]
    pub initiator_name: String,
    #[serde(rename = "SecurityCredential")]
    pub security_credential: String,
    #[serde(rename = "CommandID")]
    pub command_id: String,
    #[serde(rename = "Amount")]
    pub amount: String,
    #[serde(rename = "PartyA")]
    pub party_a: String,
    #[serde(rename = "PartyB")]
    pub party_b: String,
    #[serde(rename = "Remarks")]
    pub remarks: String,
    #[serde(rename = "QueueTimeOutURL")]
    pub queue_timeout_url: String,
    #[serde(rename = "ResultURL")]
    pub result_url: String,
    #[serde(rename = "Occasion")]
    pub occasion: String,
}

/// B2C payout response (synchronous acknowledgement only).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct B2cResponse {
    #[serde(rename = "ConversationID")]
    pub conversation_id: Option<String>,
    #[serde(rename = "OriginatorConversationID")]
    pub originator_conversation_id: Option<String>,
    #[serde(rename = "ResponseCode")]
    pub response_code: String,
    #[serde(rename = "ResponseDescription")]
    pub response_description: String,
}

impl B2cResponse {
    /// True if Daraja accepted the payout for processing.
    pub fn is_accepted(&self) -> bool {
        self.response_code == "0"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stk_push_response_acceptance_check() {
        let accepted = StkPushResponse {
            merchant_request_id: Some("m".to_string()),
            checkout_request_id: Some("c".to_string()),
            response_code: "0".to_string(),
            response_description: "Success".to_string(),
            customer_message: Some("Request accepted".to_string()),
        };
        assert!(accepted.is_accepted());

        let rejected = StkPushResponse {
            merchant_request_id: None,
            checkout_request_id: None,
            response_code: "1".to_string(),
            response_description: "Rejected".to_string(),
            customer_message: None,
        };
        assert!(!rejected.is_accepted());
    }

    #[test]
    fn daraja_fields_keep_camel_case_names() {
        let req = StkPushRequest {
            business_short_code: "174379".to_string(),
            password: "p".to_string(),
            timestamp: "t".to_string(),
            transaction_type: "CustomerPayBillOnline".to_string(),
            amount: "1.00".to_string(),
            party_a: "254712345678".to_string(),
            party_b: "174379".to_string(),
            phone_number: "254712345678".to_string(),
            callback_url: "https://x/cb".to_string(),
            account_reference: "ACCT-1".to_string(),
            transaction_desc: "payment".to_string(),
        };
        let json = serde_json::to_value(&req).expect("serialize");
        let obj = json.as_object().expect("object");
        assert!(obj.contains_key("BusinessShortCode"));
        assert!(!obj.contains_key("CheckoutRequestID"));
        assert!(obj.contains_key("CallBackURL"));
        assert!(obj.contains_key("TransactionDesc"));
    }
}
