//! `GET /health` / `GET /api/v1/health`: liveness for the datastore-free
//! controller. There is no datastore to probe (issue #143 removed MongoDB), so
//! the controller reports ready unconditionally — it is up iff it can serve.

use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

/// Health response body. Field order is the wire contract: `status`, `version`.
#[derive(Debug, Serialize, ToSchema)]
pub struct HealthResponse {
    /// Always `"ok"` — a process that can answer this route is healthy.
    #[schema(example = "ok")]
    pub status: &'static str,
    /// The running controller version (the unified product version).
    #[schema(example = "0.1.0")]
    pub version: &'static str,
}

/// Build the (always-ready) health body. Shared by both mounted paths.
fn health_body() -> HealthResponse {
    HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// `GET /health`: report ready. The controller holds no datastore (database-free
/// since #143), so liveness is sufficient.
#[utoipa::path(
    get,
    path = "/health",
    tag = "system",
    operation_id = "health",
    responses(
        (status = 200, description = "Controller is live", body = HealthResponse)
    )
)]
pub async fn health() -> Json<HealthResponse> {
    Json(health_body())
}

/// `GET /api/v1/health`: the same liveness probe under the `/api/v1` prefix (the
/// proxy may route everything under `/api/v1`). Kept as a distinct documented
/// operation so the spec lists both live paths.
#[utoipa::path(
    get,
    path = "/api/v1/health",
    tag = "system",
    operation_id = "health_v1",
    responses(
        (status = 200, description = "Controller is live", body = HealthResponse)
    )
)]
pub async fn health_v1() -> Json<HealthResponse> {
    Json(health_body())
}
