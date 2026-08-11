//! M-Pesa Daraja integration crate.
//!
//! A typed client for the Safaricom Daraja API covering the two rails
//! the platform needs:
//!
//! - **STK Push (C2B collect)** - prompt a customer's phone for payment
//! - **B2C payout** - send money from a business account to a customer
//! - **Transaction query** - poll the status of either operation
//!
//! Authentication uses OAuth2 `client_credentials` against Daraja's
//! token endpoint with the consumer key/secret. Tokens are cached in
//! memory and refreshed before they expire.
//!
//! ## Environments
//!
//! - `Sandbox` - `https://sandbox.safaricom.co.ke`
//! - `Production` - `https://api.safaricom.co.ke`
//!
//! ## Testing
//!
//! HTTP interactions are tested against a mock server ([`wiremock`]);
//! request signing and timestamp generation have pure unit tests.

pub mod auth;
pub mod client;
pub mod config;
pub mod error;
pub mod security;
pub mod types;

pub use auth::TokenCache;
pub use client::MpesaClient;
pub use config::{Environment, MpesaConfig, DEFAULT_TIMEOUT_SECS, DEFAULT_TOKEN_TTL_SECS};
pub use error::{MpesaError, MpesaResult};
pub use security::{security_credential, stk_password};
pub use types::{
    B2cRequest, B2cResponse, StkPushRequest, StkPushResponse, StkQueryRequest, StkQueryResponse,
};
