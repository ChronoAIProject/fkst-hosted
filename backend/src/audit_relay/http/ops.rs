//! `/health`, `/ready`, and `/metrics`.
//!
//! The split between the first two is the whole operational contract of the
//! relay:
//!
//! - **`/health` is liveness.** The process is up and serving. It never consults
//!   PostHog or storage, so a restart loop cannot be caused by a dependency.
//! - **`/ready` is DURABLE INGRESS.** It is `200` only while a record can still
//!   be committed: writable disk, a completed migration, no corruption, and
//!   capacity remaining. A PostHog outage deliberately does NOT make it false —
//!   an outbox whose destination is down is doing exactly its job, and taking the
//!   relay out of service would turn a delivery delay into a control-plane
//!   outage in `required` mode.
//!
//! `/metrics` renders bounded, closed-label series only (see
//! [`super::super::metrics`]).

use axum::extract::State;
use axum::http::{header, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use super::RelayState;

/// The Prometheus text content type (version 0.0.4 exposition format).
const PROMETHEUS_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// Liveness. Deliberately dependency-free.
pub async fn health() -> impl IntoResponse {
    (StatusCode::OK, Json(json!({ "status": "ok" })))
}

/// Durable-ingress readiness.
pub async fn ready(State(state): State<RelayState>) -> impl IntoResponse {
    let storage_ready = state.db.ingress_ready();
    let at_capacity = state.is_at_capacity();
    let ready = storage_ready && !at_capacity;
    let status = if ready {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    (
        status,
        Json(json!({
            "ready": ready,
            "storage_ready": storage_ready,
            "at_capacity": at_capacity,
        })),
    )
}

/// Bounded delivery telemetry.
pub async fn metrics(State(state): State<RelayState>) -> impl IntoResponse {
    state
        .metrics
        .set_writer_queue_depth(state.db.writer_queue_depth());
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, PROMETHEUS_CONTENT_TYPE)],
        state.metrics.render(state.db.ingress_ready()),
    )
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
