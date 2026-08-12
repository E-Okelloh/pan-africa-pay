//! HTTP routes and the application router.

use axum::routing::{get, post};
use axum::{Json, Router};
use serde::Serialize;
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::trace::TraceLayer;

use crate::state::AppState;

mod health;
mod kotani;
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
        .route(
            "/mpesa/stk/query/{checkout_request_id}",
            get(mpesa::stk_query),
        )
        .route("/webhooks/mpesa", post(mpesa::webhook))
        .route("/kotani/customers", post(kotani::create_customer))
        .route("/kotani/deposit", post(kotani::deposit))
        .route("/kotani/withdraw", post(kotani::withdraw))
        .route(
            "/kotani/deposit/status/{reference_id}",
            get(kotani::deposit_status),
        )
        .route(
            "/kotani/withdraw/status/{reference_id}",
            get(kotani::withdraw_status),
        )
        .route("/webhooks/kotani", post(kotani::webhook))
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
