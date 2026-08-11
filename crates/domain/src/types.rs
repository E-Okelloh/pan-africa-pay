//! Core domain types and value objects.
//!
//! This module defines the fundamental types used across the platform:
//! - Money and currency handling (KES, USDC)
//! - Phone number validation (E.164 format)
//! - Payment lifecycle types (type, status, rail)
//! - Wallet identifiers

use std::fmt;
use std::str::FromStr;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::error::{AppError, AppResult};

/// Supported currencies.
///
/// The MVP supports the Kenyan Shilling (fiat rail via M-Pesa) and
/// USD Coin (USDC, digital rail via Kotani Pay).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    /// Kenyan Shilling - local fiat currency.
    KES,
    /// USD Coin - USDC stablecoin on Stellar.
    USDC,
}

impl fmt::Display for Currency {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", match self {
            Self::KES => "KES",
            Self::USDC => "USDC",
        })
    }
}

impl FromStr for Currency {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_uppercase().as_str() {
            "KES" => Ok(Self::KES),
            "USDC" => Ok(Self::USDC),
            _ => Err(AppError::validation(format!("Unsupported currency: {s}"))),
        }
    }
}

/// Monetary value expressed as minor units (cents/avos).
///
/// Storing amounts as `i64` minor units avoids floating-point errors
/// that would corrupt financial records. All arithmetic happens on
/// integers; display formatting is a presentation concern.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct Money {
    /// Amount in minor units (e.g. cents for KES, avos for USDC).
    pub amount: i64,
    /// Currency of the amount.
    pub currency: Currency,
}

impl Money {
    /// Number of minor units per major unit for each currency.
    pub const KES_MINOR_UNITS: i64 = 100;
    pub const USDC_MINOR_UNITS: i64 = 1_000_000;

    /// Create a new `Money` value.
    pub fn new(amount: i64, currency: Currency) -> Self {
        Self { amount, currency }
    }

    /// Zero amount for the given currency.
    pub fn zero(currency: Currency) -> Self {
        Self { amount: 0, currency }
    }

    /// Amount in major units (e.g. KES 150.00).
    pub fn to_decimal(&self) -> Decimal {
        let divisor = self.minor_units();
        Decimal::from_i64_with_scale(self.amount, divisor.ilog10() as u32)
    }

    /// Number of minor units per major unit.
    pub fn minor_units(&self) -> i64 {
        match self.currency {
            Currency::KES => Self::KES_MINOR_UNITS,
            Currency::USDC => Self::USDC_MINOR_UNITS,
        }
    }

    /// Add two amounts, panicking on currency mismatch.
    ///
    /// Returns a checked-add result; overflow is impossible within
    /// realistic payment volumes but is still guarded.
    pub fn checked_add(&self, other: Self) -> Option<Self> {
        if self.currency != other.currency {
            return None;
        }
        Some(Self {
            amount: self.amount.checked_add(other.amount)?,
            currency: self.currency,
        })
    }

    /// Subtract two amounts, panicking on currency mismatch.
    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        if self.currency != other.currency {
            return None;
        }
        Some(Self {
            amount: self.amount.checked_sub(other.amount)?,
            currency: self.currency,
        })
    }

    /// True if the amount is non-negative.
    pub fn is_non_negative(&self) -> bool {
        self.amount >= 0
    }
}

impl fmt::Display for Money {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.to_decimal(), self.currency)
    }
}

/// E.164 international phone number.
///
/// Validates the international format used by all African mobile money
/// providers (e.g. +254712345678 for Kenya). Digits are stored without
/// the leading `+` for consistent comparison.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PhoneNumber(String);

impl PhoneNumber {
    /// Country code prefix for Kenya.
    pub const KENYA_CC: &'static str = "254";

    /// Create a new phone number, validating the E.164 format.
    pub fn new(raw: &str) -> AppResult<Self> {
        let normalized = raw.trim().trim_start_matches('+');
        if normalized.is_empty() || !normalized.chars().all(|c| c.is_ascii_digit()) {
            return Err(AppError::validation("Phone number must contain only digits"));
        }
        if !(8..=15).contains(&normalized.len()) {
            return Err(AppError::validation("Phone number must be 8-15 digits (E.164)"));
        }
        Ok(Self(normalized.to_string()))
    }

    /// Create a phone number from its country code and subscriber digits.
    pub fn from_parts(country_code: &str, subscriber: &str) -> AppResult<Self> {
        Self::new(&format!("{country_code}{subscriber}"))
    }

    /// The full E.164 number without the leading `+`.
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// The number with the leading `+` prefix (canonical E.164).
    pub fn with_plus(&self) -> String {
        format!("+{}", self.0)
    }

    /// Extract the country code (first 1-3 digits).
    pub fn country_code(&self) -> &str {
        &self.0[..3.min(self.0.len())]
    }

    /// True if this number belongs to Kenya.
    pub fn is_kenyan(&self) -> bool {
        self.0.starts_with(Self::KENYA_CC)
    }
}

impl fmt::Display for PhoneNumber {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "+{}", self.0)
    }
}

impl FromStr for PhoneNumber {
    type Err = AppError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

/// Direction of a payment relative to the platform.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentType {
    /// Money coming in: M-Pesa STK Push or Kotani USDC deposit.
    Collect,
    /// Money going out: B2C payout or Kotani USDC withdrawal.
    Payout,
    /// USDC deposit into a user's wallet via Kotani (KES -> USDC).
    Deposit,
    /// USDC withdrawal from a user's wallet via Kotani (USDC -> KES).
    Withdraw,
}

impl PaymentType {
    /// Parse from the SCREAMING_SNAKE_CASE string form used in storage.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "COLLECT" => Ok(Self::Collect),
            "PAYOUT" => Ok(Self::Payout),
            "DEPOSIT" => Ok(Self::Deposit),
            "WITHDRAW" => Ok(Self::Withdraw),
            _ => Err(AppError::validation(format!("Unsupported payment type: {value}"))),
        }
    }
}

/// Lifecycle status of a payment.
///
/// Transitions are enforced in the service layer:
/// Pending -> Processing -> Completed | Failed
/// Pending -> Expired (after timeout)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum PaymentStatus {
    /// Payment created, awaiting confirmation from provider.
    Pending,
    /// Provider has acknowledged the payment; processing.
    Processing,
    /// Payment succeeded and funds were moved.
    Completed,
    /// Payment failed at the provider.
    Failed,
    /// Payment expired after the timeout window.
    Expired,
}

impl PaymentStatus {
    /// Terminal states from which a payment cannot transition.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Expired)
    }

    /// Parse from the SCREAMING_SNAKE_CASE string form used in storage.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "PENDING" => Ok(Self::Pending),
            "PROCESSING" => Ok(Self::Processing),
            "COMPLETED" => Ok(Self::Completed),
            "FAILED" => Ok(Self::Failed),
            "EXPIRED" => Ok(Self::Expired),
            _ => Err(AppError::validation(format!("Unsupported payment status: {value}"))),
        }
    }
}

impl Rail {
    /// Parse from the SCREAMING_SNAKE_CASE string form used in storage.
    pub fn parse(value: &str) -> AppResult<Self> {
        match value {
            "MPESA" => Ok(Self::Mpesa),
            "KOTANI" => Ok(Self::Kotani),
            _ => Err(AppError::validation(format!("Unsupported rail: {value}"))),
        }
    }
}

/// The settlement rail a payment travels on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Rail {
    /// Safaricom M-Pesa Daraja API (local fiat).
    Mpesa,
    /// Kotani Pay API (USDC on Stellar, cross-border).
    Kotani,
}

/// Unique identifier for a user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserId(pub Uuid);

impl UserId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for UserId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for UserId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a payment.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PaymentId(pub Uuid);

impl PaymentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PaymentId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for PaymentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a wallet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct WalletId(pub Uuid);

impl WalletId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for WalletId {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for WalletId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A user's wallet balance in one currency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Wallet {
    pub id: WalletId,
    pub user_id: UserId,
    pub currency: Currency,
    pub balance: i64,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

/// A single payment record across any rail.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payment {
    pub id: PaymentId,
    pub user_id: UserId,
    pub payment_type: PaymentType,
    pub rail: Rail,
    pub status: PaymentStatus,
    pub amount: Money,
    pub fee: Money,
    /// M-Pesa checkout request id (M-Pesa rail only).
    pub mpesa_checkout_request_id: Option<String>,
    /// M-Pesa receipt number (M-Pesa rail only).
    pub mpesa_receipt_number: Option<String>,
    /// Kotani transaction id (Kotani rail only).
    pub kotani_tx_id: Option<String>,
    /// Provider callback payload (for audit/reconciliation).
    pub callback_payload: Option<serde_json::Value>,
    /// The idempotency key that produced this payment.
    pub idempotency_key: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn phone_number_accepts_kenyan_number() {
        let phone = PhoneNumber::new("+254712345678").unwrap();
        assert_eq!(phone.as_str(), "254712345678");
        assert_eq!(phone.country_code(), "254");
        assert!(phone.is_kenyan());
    }

    #[test]
    fn phone_number_rejects_invalid_input() {
        assert!(PhoneNumber::new("").is_err());
        assert!(PhoneNumber::new("+abc").is_err());
        assert!(PhoneNumber::new("123").is_err());
        assert!(PhoneNumber::new("+25471234567890123456789").is_err());
    }

    #[test]
    fn phone_number_round_trip() {
        let phone = PhoneNumber::new("254712345678").unwrap();
        assert_eq!(phone.to_string(), "+254712345678");
        assert_eq!(PhoneNumber::from_str("+254712345678").unwrap(), phone);
    }

    #[test]
    fn money_arithmetic_requires_same_currency() {
        let kes_100 = Money::new(100_00, Currency::KES);
        let usdc_1 = Money::new(1_000_000, Currency::USDC);
        assert!(kes_100.checked_add(usdc_1).is_none());
        assert!(kes_100.checked_sub(usdc_1).is_none());
    }

    #[test]
    fn money_arithmetic_matches_currency() {
        let a = Money::new(100_00, Currency::KES);
        let b = Money::new(50_00, Currency::KES);
        assert_eq!(a.checked_sub(b).unwrap(), Money::new(50_00, Currency::KES));
    }

    #[test]
    fn money_display_shows_currency() {
        assert_eq!(Money::new(100_00, Currency::KES).to_string(), "100.00 KES");
    }

    #[test]
    fn currency_parses_case_insensitively() {
        assert_eq!(Currency::from_str("kes").unwrap(), Currency::KES);
        assert_eq!(Currency::from_str("USDC").unwrap(), Currency::USDC);
        assert!(Currency::from_str("EUR").is_err());
    }

    #[test]
    fn payment_status_terminal_states() {
        assert!(PaymentStatus::Completed.is_terminal());
        assert!(PaymentStatus::Failed.is_terminal());
        assert!(PaymentStatus::Expired.is_terminal());
        assert!(!PaymentStatus::Pending.is_terminal());
        assert!(!PaymentStatus::Processing.is_terminal());
    }
}
