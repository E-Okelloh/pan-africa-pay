//! Pan-Africa Pay API crate.
//!
//! The HTTP layer of the platform: configuration bootstrap, axum
//! router construction, shared state, and error envelope mapping.
//!
//! - `config` - layered configuration (defaults + environment)
//! - `state` - shared application state (pools, config)
//! - `error` - domain errors to HTTP JSON envelope mapping
//! - `routes` - router assembly and route handlers

pub mod config;
pub mod error;
pub mod idempotency;
pub mod reconciliation;
pub mod routes;
pub mod state;
