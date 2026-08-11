//! Domain events for the payment platform.
//!
//! Events describe state transitions that already happened. They are
//! published by the service layer after a state change is committed,
//! and consumed by handlers that react to those changes (webhooks,
//! notifications, reconciliation).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::types::{Money, PaymentId, PaymentStatus, PhoneNumber, UserId};

/// Unique identifier for a domain event instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EventId(pub uuid::Uuid);

impl EventId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v7())
    }
}

impl Default for EventId {
    fn default() -> Self {
        Self::new()
    }
}

/// A payment lifecycle event: created or transitioned.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaymentEvent {
    pub event_id: EventId,
    pub payment_id: PaymentId,
    pub user_id: UserId,
    /// The status this payment moved to.
    pub status: PaymentStatus,
    /// The status the payment was in before this event.
    pub previous_status: Option<PaymentStatus>,
    pub occurred_at: DateTime<Utc>,
}

impl PaymentEvent {
    pub fn new(
        payment_id: PaymentId,
        user_id: UserId,
        status: PaymentStatus,
        previous_status: Option<PaymentStatus>,
    ) -> Self {
        Self {
            event_id: EventId::new(),
            payment_id,
            user_id,
            status,
            previous_status,
            occurred_at: Utc::now(),
        }
    }
}

/// A wallet balance change: credit or debit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WalletEvent {
    pub event_id: EventId,
    pub user_id: UserId,
    pub wallet_id: crate::types::WalletId,
    /// Signed delta in minor units (positive = credit, negative = debit).
    pub delta: i64,
    /// Balance after applying the delta.
    pub new_balance: i64,
    /// Reference to the payment that caused the change.
    pub payment_id: Option<PaymentId>,
    pub occurred_at: DateTime<Utc>,
}

impl WalletEvent {
    pub fn credit(
        user_id: UserId,
        wallet_id: crate::types::WalletId,
        amount: i64,
        new_balance: i64,
        payment_id: Option<PaymentId>,
    ) -> Self {
        Self {
            event_id: EventId::new(),
            user_id,
            wallet_id,
            delta: amount,
            new_balance,
            payment_id,
            occurred_at: Utc::now(),
        }
    }

    pub fn debit(
        user_id: UserId,
        wallet_id: crate::types::WalletId,
        amount: i64,
        new_balance: i64,
        payment_id: Option<PaymentId>,
    ) -> Self {
        Self {
            event_id: EventId::new(),
            user_id,
            wallet_id,
            delta: -amount,
            new_balance,
            payment_id,
            occurred_at: Utc::now(),
        }
    }
}

/// M-Pesa specific event: a payment collected from a phone.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MpesaCollectedEvent {
    pub event_id: EventId,
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub phone: PhoneNumber,
    /// Gross amount collected from the customer.
    pub amount: Money,
    pub checkout_request_id: String,
    pub receipt_number: String,
    pub occurred_at: DateTime<Utc>,
}

/// Kotani specific event: a USDC deposit or withdrawal completed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KotaniTransactionEvent {
    pub event_id: EventId,
    pub payment_id: PaymentId,
    pub user_id: UserId,
    pub kotani_tx_id: String,
    pub amount: Money,
    pub occurred_at: DateTime<Utc>,
}

/// Top-level domain event enum.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DomainEvent {
    /// A payment was created or transitioned to a new status.
    PaymentTransition(PaymentEvent),
    /// A wallet balance was credited.
    WalletCredited(WalletEvent),
    /// A wallet balance was debited.
    WalletDebited(WalletEvent),
    /// M-Pesa STK Push funds were collected.
    MpesaCollected(MpesaCollectedEvent),
    /// A Kotani deposit/withdrawal completed.
    KotaniTransaction(KotaniTransactionEvent),
}

impl DomainEvent {
    /// The `event_id` shared across all variants.
    pub fn id(&self) -> EventId {
        match self {
            Self::PaymentTransition(e) => e.event_id,
            Self::WalletCredited(e) | Self::WalletDebited(e) => e.event_id,
            Self::MpesaCollected(e) => e.event_id,
            Self::KotaniTransaction(e) => e.event_id,
        }
    }

    /// The timestamp shared across all variants.
    pub fn occurred_at(&self) -> DateTime<Utc> {
        match self {
            Self::PaymentTransition(e) => e.occurred_at,
            Self::WalletCredited(e) | Self::WalletDebited(e) => e.occurred_at,
            Self::MpesaCollected(e) => e.occurred_at,
            Self::KotaniTransaction(e) => e.occurred_at,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{Currency, Money, PaymentStatus, PhoneNumber, UserId, WalletId};

    #[test]
    fn payment_event_serializes_with_tag() {
        let event = PaymentEvent::new(
            PaymentId::new(),
            UserId::new(),
            PaymentStatus::Pending,
            None,
        );
        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["status"], "PENDING");
        assert!(json["previous_status"].is_null());
    }

    #[test]
    fn wallet_credit_delta_is_positive() {
        let event = WalletEvent::credit(
            UserId::new(),
            WalletId::new(),
            500,
            1500,
            Some(PaymentId::new()),
        );
        assert_eq!(event.delta, 500);
    }

    #[test]
    fn wallet_debit_delta_is_negative() {
        let event = WalletEvent::debit(
            UserId::new(),
            WalletId::new(),
            300,
            1200,
            Some(PaymentId::new()),
        );
        assert_eq!(event.delta, -300);
    }

    #[test]
    fn domain_event_round_trip() {
        let event = DomainEvent::MpesaCollected(MpesaCollectedEvent {
            event_id: EventId::new(),
            payment_id: PaymentId::new(),
            user_id: UserId::new(),
            phone: PhoneNumber::new("+254712345678").unwrap(),
            amount: Money::new(10_000, Currency::KES),
            checkout_request_id: "ws_CO_123".to_string(),
            receipt_number: "PJX1AB2CD3".to_string(),
            occurred_at: Utc::now(),
        });

        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["type"], "MPESA_COLLECTED");

        let decoded: DomainEvent = serde_json::from_value(json).unwrap();
        assert_eq!(decoded, event);
    }

    #[test]
    fn event_ids_are_unique() {
        let a = EventId::new();
        let b = EventId::new();
        assert_ne!(a, b);
    }
}
