//! Pan-Africa Pay Storage Crate
//!
//! This crate implements the repository traits defined in the `domain`
//! crate against PostgreSQL (via `sqlx`) and Redis.
//!
//! ## Architecture
//!
//! - `pool.rs` - connection pool builders for PostgreSQL and Redis
//! - `models.rs` - database row types mapping to domain entities
//! - `repositories/` - trait implementations:
//!   - `payment.rs` - payment persistence
//!   - `wallet.rs` - wallet persistence with atomic balance updates
//!   - `idempotency.rs` - idempotency records (Redis primary, PG backup)
//!
//! ## Migrations
//!
//! SQL migrations live in `crates/storage/migrations` and are applied
//! with `sqlx migrate run` (see `sqlx-cli`).

pub mod models;
pub mod pool;
pub mod repositories;

pub use models::{PaymentRow, WalletRow};
pub use pool::{DatabaseConfig, DatabasePool};
pub use repositories::Repositories;
