//! Pan-Africa Pay API server.
//!
//! Phase 2 implementation: Axum server with payment, wallet, webhook,
//! and health routes. This module currently contains scaffolding only.

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt().init();
    tracing::info!("pan-africa-pay API scaffold; routes arrive in Phase 2");
}
