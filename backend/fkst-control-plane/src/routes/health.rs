//! `GET /health` / `GET /api/v1/health`: liveness for the datastore-free
//! controller. There is no datastore to probe (issue #143 removed MongoDB), so
//! the controller reports ready unconditionally — it is up iff it can serve.

use axum::Json;
use serde::Serialize;

/// Health response body. Field order is the wire contract: `status`, `version`.
#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub version: &'static str,
}

/// Report ready. The controller holds no datastore (database-free since #143),
/// so liveness is sufficient: a process that can answer this route is healthy.
pub async fn health() -> Json<HealthResponse> {
    Json(HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    })
}
