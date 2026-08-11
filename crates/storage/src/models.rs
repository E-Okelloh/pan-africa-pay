//! Database row types.
//!
//! Row structs mirror the SQL schema exactly and provide conversions
//! to the domain entities defined in `pan-africa-pay-domain`.

use chrono::{DateTime, Utc};
use serde_json::Value as JsonValue;
use uuid::Uuid;

use pan_africa_pay_domain::types::{
    Currency, Money, Payment, PaymentId, PaymentStatus, PaymentType, Rail, UserId, Wallet, WalletId,
};

/// Row representation of the `payments` table.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct PaymentRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub payment_type: String,
    pub rail: String,
    pub status: String,
    pub amount: i64,
    pub currency: String,
    pub fee: i64,
    pub mpesa_checkout_request_id: Option<String>,
    pub mpesa_receipt_number: Option<String>,
    pub kotani_tx_id: Option<String>,
    pub callback_payload: Option<JsonValue>,
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<PaymentRow> for Payment {
    type Error = String;

    fn try_from(row: PaymentRow) -> Result<Self, Self::Error> {
        let currency: Currency = row
            .currency
            .parse()
            .map_err(|_| format!("invalid currency: {}", row.currency))?;
        Ok(Self {
            id: PaymentId(row.id),
            user_id: UserId(row.user_id),
            payment_type: PaymentType::parse(&row.payment_type).map_err(|e| e.message)?,
            rail: Rail::parse(&row.rail).map_err(|e| e.message)?,
            status: PaymentStatus::parse(&row.status).map_err(|e| e.message)?,
            amount: Money::new(row.amount, currency),
            fee: Money::new(row.fee, currency),
            mpesa_checkout_request_id: row.mpesa_checkout_request_id,
            mpesa_receipt_number: row.mpesa_receipt_number,
            kotani_tx_id: row.kotani_tx_id,
            callback_payload: row.callback_payload,
            idempotency_key: row.idempotency_key,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

impl From<&Payment> for PaymentRow {
    fn from(p: &Payment) -> Self {
        Self {
            id: p.id.0,
            user_id: p.user_id.0,
            payment_type: serde_enum(&p.payment_type),
            rail: serde_enum(&p.rail),
            status: serde_enum(&p.status),
            amount: p.amount.amount,
            currency: p.amount.currency.to_string(),
            fee: p.fee.amount,
            mpesa_checkout_request_id: p.mpesa_checkout_request_id.clone(),
            mpesa_receipt_number: p.mpesa_receipt_number.clone(),
            kotani_tx_id: p.kotani_tx_id.clone(),
            callback_payload: p.callback_payload.clone(),
            idempotency_key: p.idempotency_key.clone(),
            created_at: p.created_at,
            updated_at: p.updated_at,
        }
    }
}

impl WalletRow {
    /// Convert a domain wallet into its row representation.
    pub fn from_wallet(w: &Wallet) -> Self {
        Self {
            id: w.id.0,
            user_id: w.user_id.0,
            currency: w.currency.to_string(),
            balance: w.balance,
            created_at: w.created_at,
            updated_at: w.updated_at,
        }
    }
}

/// Row representation of the `wallets` table.
#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct WalletRow {
    pub id: Uuid,
    pub user_id: Uuid,
    pub currency: String,
    pub balance: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl TryFrom<WalletRow> for Wallet {
    type Error = String;

    fn try_from(row: WalletRow) -> Result<Self, Self::Error> {
        let currency: Currency = row
            .currency
            .parse()
            .map_err(|_| format!("invalid currency: {}", row.currency))?;
        Ok(Self {
            id: WalletId(row.id),
            user_id: UserId(row.user_id),
            currency,
            balance: row.balance,
            created_at: row.created_at,
            updated_at: row.updated_at,
        })
    }
}

/// Serialize a domain enum to its SCREAMING_SNAKE_CASE string form.
fn serde_enum<T>(value: &T) -> String
where
    T: serde::Serialize,
{
    serde_json::to_string(value)
        .unwrap_or_default()
        .trim_matches('"')
        .to_string()
}
