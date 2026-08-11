//! Health check endpoints.
//!
//! - `GET /health`        - liveness: the process is up.
//! - `GET /health/ready`  - readiness: dependencies are reachable.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::error::status_for;
use crate::routes::{ok, OkEnvelope};
use crate::state::AppState;

/// Liveness check: always 200 while the process runs.
pub async fn health_check() -> Json<OkEnvelope<HealthStatus>> {
    ok(HealthStatus {
        status: "ok".to_string(),
    })
}

/// Readiness check: verifies PostgreSQL and Redis connectivity.
pub async fn ready_check(
    State(state): State<AppState>,
) -> Result<Json<OkEnvelope<HealthStatus>>, StatusCode> {
    match state.pool.health_check().await {
        Ok(()) => Ok(ok(HealthStatus {
            status: "ready".to_string(),
        })),
        Err(err) => Err(status_for(&err)),
    }
}

/// Status payload for health endpoints.
#[derive(Debug, Clone, Serialize)]
pub struct HealthStatus {
    pub status: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_status_serializes() {
        let json = serde_json::to_value(HealthStatus {
            status: "ok".to_string(),
        })
        .expect("serialize");
        assert_eq!(json["status"], "ok");
    }
}
