//! Process liveness (`GET /health`) and recovery readiness (`GET /ready`).
//!
//! GitHub availability never affects liveness. Readiness is a separate projection
//! of the serialized full-resync coordinator, so a GitHub outage cannot create a
//! Kubernetes restart loop.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;

use crate::state::AppState;

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

/// Build the always-live health body.
fn health_body() -> HealthResponse {
    HealthResponse {
        status: "ok",
        version: env!("CARGO_PKG_VERSION"),
    }
}

/// `GET /health`: report process liveness independently of recovery dependencies.
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

/// Public recovery phases. `recovering` means no full pass has completed yet;
/// `degraded` means an attempt or reconciler prerequisite failed.
#[derive(Clone, Copy, Debug, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum ReadinessStatus {
    Ready,
    Recovering,
    Degraded,
}

/// Recovery readiness body. It deliberately exposes no repository, installation,
/// session, or error detail.
#[derive(Debug, Serialize, ToSchema)]
pub struct ReadinessResponse {
    pub status: ReadinessStatus,
    pub version: &'static str,
    pub startup_resync_complete: bool,
}

/// `GET /ready`: report whether the latest serialized discovery pass is complete.
/// Dispatch deliberately disabled is ready immediately; enabled dispatch remains
/// `503` until a complete pass succeeds and becomes `503` again if a later pass is
/// incomplete.
#[utoipa::path(
    get,
    path = "/ready",
    tag = "system",
    operation_id = "readiness",
    responses(
        (status = 200, description = "Recovery discovery is complete", body = ReadinessResponse),
        (status = 503, description = "Recovery discovery is incomplete or degraded", body = ReadinessResponse)
    )
)]
pub async fn readiness(State(state): State<AppState>) -> (StatusCode, Json<ReadinessResponse>) {
    let snapshot = state.recovery.snapshot();
    let status = if snapshot.ready {
        ReadinessStatus::Ready
    } else if snapshot.degraded {
        ReadinessStatus::Degraded
    } else {
        ReadinessStatus::Recovering
    };
    let http_status = if snapshot.ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        http_status,
        Json(ReadinessResponse {
            status,
            version: env!("CARGO_PKG_VERSION"),
            startup_resync_complete: snapshot.startup_resync_complete,
        }),
    )
}
