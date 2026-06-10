//! `GET /health` liveness endpoint.

use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

/// Health response body. Field order is the wire contract:
/// `status`, `version`, `checks`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
    pub checks: HealthChecks,
}

/// Per-dependency health checks.
#[derive(Debug, Serialize)]
pub struct HealthChecks {
    pub mongo: &'static str,
}

/// Liveness probe: reports process liveness only for now. The real Mongo
/// ping is wired by the Mongo-connection issue; until then `checks.mongo`
/// stays `"unknown"` so the response shape is already stable.
pub async fn health(State(_state): State<AppState>) -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
        checks: HealthChecks { mongo: "unknown" },
    })
}
