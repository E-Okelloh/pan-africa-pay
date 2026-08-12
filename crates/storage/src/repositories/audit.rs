//! Append-only audit log repository.
//!
//! Implements [`EventPublisher`] by persisting every domain event to
//! the `audit_log` table. Append-only semantics are enforced by the
//! application: rows are never updated or deleted.

use async_trait::async_trait;
use sqlx::PgPool;

use pan_africa_pay_domain::error::{AppError, AppResult};
use pan_africa_pay_domain::events::DomainEvent;
use pan_africa_pay_domain::traits::EventPublisher;

/// SQL adapter implementing [`EventPublisher`].
pub struct AuditRepo {
    pool: PgPool,
}

/// A single audit log row as returned by [`AuditRepo::list_for_entity`].
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AuditEntry {
    pub event_type: String,
    pub actor_id: Option<uuid::Uuid>,
    pub payload: serde_json::Value,
    pub created_at: chrono::DateTime<chrono::Utc>,
}

impl AuditRepo {
    /// Create a new adapter bound to a connection pool.
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// List audit events for an entity (e.g. all lifecycle events of a
    /// payment), newest first.
    pub async fn list_for_entity(
        &self,
        entity_type: &str,
        entity_id: uuid::Uuid,
        limit: i64,
    ) -> AppResult<Vec<AuditEntry>> {
        sqlx::query_as::<
            _,
            (
                String,
                Option<uuid::Uuid>,
                serde_json::Value,
                chrono::DateTime<chrono::Utc>,
            ),
        >(
            r#"
            SELECT event_type, actor_id, payload, created_at
            FROM audit_log
            WHERE entity_type = $1 AND entity_id = $2
            ORDER BY created_at DESC
            LIMIT $3
            "#,
        )
        .bind(entity_type)
        .bind(entity_id)
        .bind(limit)
        .fetch_all(&self.pool)
        .await
        .map(|rows| {
            rows.into_iter()
                .map(|(event_type, actor_id, payload, created_at)| AuditEntry {
                    event_type,
                    actor_id,
                    payload,
                    created_at,
                })
                .collect()
        })
        .map_err(|e| AppError::internal(format!("audit query failed: {e}")))
    }
}

/// Columns derived from a domain event for the audit_log table.
struct AuditRow<'a> {
    event_type: &'static str,
    entity_type: &'static str,
    entity_id: uuid::Uuid,
    actor_id: Option<uuid::Uuid>,
    payload: &'a serde_json::Value,
}

#[async_trait]
impl EventPublisher for AuditRepo {
    async fn publish(&self, event: &DomainEvent) -> AppResult<()> {
        let payload = serde_json::to_value(event)
            .map_err(|e| AppError::internal(format!("event serialization failed: {e}")))?;
        let row = audit_row(event, &payload);

        sqlx::query(
            r#"
            INSERT INTO audit_log (event_type, entity_type, entity_id, actor_id, payload)
            VALUES ($1, $2, $3, $4, $5)
            "#,
        )
        .bind(row.event_type)
        .bind(row.entity_type)
        .bind(row.entity_id)
        .bind(row.actor_id)
        .bind(row.payload)
        .execute(&self.pool)
        .await
        .map_err(|e| AppError::internal(format!("audit insert failed: {e}")))?;
        Ok(())
    }
}

/// Map a domain event to audit_log columns.
fn audit_row<'a>(event: &DomainEvent, payload: &'a serde_json::Value) -> AuditRow<'a> {
    match event {
        DomainEvent::PaymentTransition(e) => AuditRow {
            event_type: "PAYMENT_TRANSITION",
            entity_type: "PAYMENT",
            entity_id: e.payment_id.0,
            actor_id: Some(e.user_id.0),
            payload,
        },
        DomainEvent::WalletCredited(e) | DomainEvent::WalletDebited(e) => AuditRow {
            event_type: if matches!(event, DomainEvent::WalletCredited(_)) {
                "WALLET_CREDITED"
            } else {
                "WALLET_DEBITED"
            },
            entity_type: "WALLET",
            entity_id: e.wallet_id.0,
            actor_id: Some(e.user_id.0),
            payload,
        },
        DomainEvent::MpesaCollected(e) => AuditRow {
            event_type: "MPESA_COLLECTED",
            entity_type: "PAYMENT",
            entity_id: e.payment_id.0,
            actor_id: Some(e.user_id.0),
            payload,
        },
        DomainEvent::KotaniTransaction(e) => AuditRow {
            event_type: "KOTANI_TRANSACTION",
            entity_type: "PAYMENT",
            entity_id: e.payment_id.0,
            actor_id: Some(e.user_id.0),
            payload,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pan_africa_pay_domain::events::PaymentEvent;
    use pan_africa_pay_domain::types::{PaymentId, PaymentStatus, UserId};

    #[test]
    fn maps_payment_transition_row() {
        let event = DomainEvent::PaymentTransition(PaymentEvent::new(
            PaymentId::new(),
            UserId::new(),
            PaymentStatus::Completed,
            Some(PaymentStatus::Pending),
        ));
        let payload = serde_json::to_value(&event).expect("serialize");
        let row = audit_row(&event, &payload);
        assert_eq!(row.event_type, "PAYMENT_TRANSITION");
        assert_eq!(row.entity_type, "PAYMENT");
        assert!(row.actor_id.is_some());
        assert_eq!(payload["status"], "COMPLETED");
    }

    #[test]
    fn maps_wallet_events_to_credit_and_debit() {
        let user_id = UserId::new();
        let wallet_id = pan_africa_pay_domain::types::WalletId::new();
        let credit = DomainEvent::WalletCredited(
            pan_africa_pay_domain::events::WalletEvent::credit(user_id, wallet_id, 100, 500, None),
        );
        let debit = DomainEvent::WalletDebited(pan_africa_pay_domain::events::WalletEvent::debit(
            user_id, wallet_id, 50, 450, None,
        ));
        let payload = serde_json::Value::Null;
        assert_eq!(audit_row(&credit, &payload).event_type, "WALLET_CREDITED");
        assert_eq!(audit_row(&debit, &payload).event_type, "WALLET_DEBITED");
        assert_eq!(audit_row(&credit, &payload).entity_type, "WALLET");
    }
}
