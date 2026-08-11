//! Pan-Africa Pay Domain Crate
//!
//! This crate contains the shared kernel of the payment platform:
//! - Core domain types and value objects
//! - Error hierarchy and result types
//! - Domain events for event-driven architecture
//! - Repository traits (ports) for storage abstraction
//! - Idempotency primitives for safe retries

pub mod error;
pub mod events;
pub mod idempotency;
pub mod traits;
pub mod types;

pub use error::{AppError, AppResult, ErrorCode};
pub use events::DomainEvent;
pub use idempotency::{IdempotencyKey, IdempotencyRecord, RequestHash};
pub use traits::{EventPublisher, IdempotencyRepository, PaymentRepository, WalletRepository};
pub use types::*;