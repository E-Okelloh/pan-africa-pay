//! HTTP routes and the application router.

use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

mod health;
mod mpesa;

/// Build the application router with all middleware applied.
///
/// Middleware order (outer to inner): tracing, panic catching. Each
/// layer is applied with `layer()` so it wraps every route below it.
pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health_check))
        .route("/health/ready", get(health::ready_check))
        .route("/mpesa/stk/push", post(mpesa::stk_push))
        .route("/webhooks/mpesa", post(mpesa::webhook))
        .layer(CatchPanicLayer::new())
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Standard JSON envelope for success responses.
#[derive(Debug, Clone, Serialize)]
pub struct OkEnvelope<T> {
    pub data: T,
}

/// A uniform success response.
///
/// All successful endpoints return `{ "data": ... }` so clients can
/// deserialize responses consistently.
pub fn ok<T>(data: T) -> Json<OkEnvelope<T>> {
    Json(OkEnvelope { data })
}
