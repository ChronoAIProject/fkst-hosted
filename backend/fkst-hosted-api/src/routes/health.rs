//! `GET /health` / `GET /api/v1/health`: liveness plus a real Mongo ping.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

/// Health response body. Field order is the wire contract:
/// `status`, `mongo`, `version`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub mongo: &'static str,
    pub version: &'static str,
}

/// Ping Mongo and report `200 ok/up` or `503 degraded/down`. The ping is
/// bounded by the driver's server-selection timeout, so a dead Mongo yields
/// a fast 503 instead of a hang; the ping error is logged, never echoed.
pub async fn health(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
    match state.db.ping().await {
        Ok(()) => (
            StatusCode::OK,
            Json(HealthResponse {
                status: "ok",
                mongo: "up",
                version: env!("CARGO_PKG_VERSION"),
            }),
        ),
        Err(e) => {
            tracing::error!(error = ?e, "health mongo ping failed");
            (
                StatusCode::SERVICE_UNAVAILABLE,
                Json(HealthResponse {
                    status: "degraded",
                    mongo: "down",
                    version: env!("CARGO_PKG_VERSION"),
                }),
            )
        }
    }
}
