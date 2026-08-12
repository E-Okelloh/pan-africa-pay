//! Best-effort domain event publishing.
//!
//! Audit events must never break the primary payment flow: publishing
//! failures are logged and swallowed.

use pan_africa_pay_domain::events::DomainEvent;
use pan_africa_pay_domain::traits::EventPublisher;
use tracing::warn;

/// Publish events, logging (never failing) on error.
pub async fn publish_best_effort(publisher: &dyn EventPublisher, events: &[DomainEvent]) {
    if let Err(error) = publisher.publish_many(events).await {
        warn!(%error, count = events.len(), "failed to publish audit events");
    }
}
