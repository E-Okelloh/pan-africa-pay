//! Kotani Pay client (API v3) for pan-africa-pay.
//!
//! Kotani Pay provides mobile money rails across Africa: register mobile
//! money customers, initiate deposits (fiat -> stablecoin) and
//! withdrawals (stablecoin -> fiat), and receive signed webhook
//! callbacks for transaction outcomes.

pub mod client;
pub mod config;
pub mod error;
pub mod types;
pub mod webhook;

pub use client::KotaniClient;
pub use config::{KotaniConfig, PRODUCTION_BASE_URL, SANDBOX_BASE_URL};
pub use error::{KotaniError, KotaniResult};
pub use webhook::verify_signature as verify_webhook_signature;
