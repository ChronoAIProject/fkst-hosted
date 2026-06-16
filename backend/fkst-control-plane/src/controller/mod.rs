//! Controller side of the internal worker protocol (issue #134).
//!
//! Hosts the in-memory [`WorkerRegistry`] (worker liveness) and the
//! [`internal_router`] that receives register / heartbeat / pull / draining /
//! released. Every internal route is guarded by a constant-time shared-secret
//! check ([`InternalAuth`]); when no secret is configured the router is not
//! mounted at all, so the internal surface is closed by default.
//!
//! This issue ships the transport only: pull always returns no assignments
//! (claim authority is #135) and draining/released are received + logged
//! (reassignment is #140).

pub mod internal_auth;
pub mod registry;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use fkst_shared::protocol::{
    check_protocol_version, Draining, Heartbeat, HeartbeatResponse, PullRequest, PullResponse,
    RegisterRequest, RegisterResponse, Released, INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

pub use internal_auth::InternalAuth;
pub use registry::{WorkerEntry, WorkerRegistry};

/// Shared state for the internal handlers.
#[derive(Clone)]
struct InternalState {
    registry: WorkerRegistry,
    heartbeat_interval_secs: u64,
}

/// Build the internal worker-protocol router, guarded by the shared secret.
/// `heartbeat_interval_secs` is the authoritative cadence the controller hands
/// back to workers on registration.
pub fn internal_router(
    registry: WorkerRegistry,
    auth: InternalAuth,
    heartbeat_interval_secs: u64,
) -> Router {
    let state = InternalState {
        registry,
        heartbeat_interval_secs,
    };
    Router::new()
        .route("/internal/v1/register", post(register))
        .route("/internal/v1/heartbeat", post(heartbeat))
        .route("/internal/v1/pull", post(pull))
        .route("/internal/v1/draining", post(draining))
        .route("/internal/v1/released", post(released))
        .layer(middleware::from_fn_with_state(auth, require_internal_auth))
        .with_state(state)
}

/// Reject any internal request without a valid `INTERNAL_AUTH_HEADER`. Never
/// logs the token value.
async fn require_internal_auth(
    State(auth): State<InternalAuth>,
    req: Request,
    next: Next,
) -> Response {
    let ok = req
        .headers()
        .get(INTERNAL_AUTH_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| auth.verify(v))
        .unwrap_or(false);
    if ok {
        next.run(req).await
    } else {
        tracing::warn!("internal request rejected: missing or invalid auth header");
        (StatusCode::UNAUTHORIZED, "unauthorized").into_response()
    }
}

/// Empty `{}` acknowledgement body for the fire-and-forget drain endpoints.
fn ack() -> Response {
    Json(serde_json::json!({})).into_response()
}

async fn register(State(st): State<InternalState>, Json(req): Json<RegisterRequest>) -> Response {
    if let Err(e) = check_protocol_version(req.protocol_version) {
        tracing::warn!(error = %e, worker_id = %req.worker_id, "register rejected: protocol mismatch");
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    st.registry.register(&req).await;
    Json(RegisterResponse {
        accepted: true,
        heartbeat_interval_secs: st.heartbeat_interval_secs,
        controller_protocol_version: PROTOCOL_VERSION,
    })
    .into_response()
}

async fn heartbeat(State(st): State<InternalState>, Json(hb): Json<Heartbeat>) -> Response {
    if let Err(e) = check_protocol_version(hb.protocol_version) {
        tracing::warn!(error = %e, worker_id = %hb.worker_id, "heartbeat rejected: protocol mismatch");
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    st.registry.heartbeat(&hb).await;
    // No control messages from the controller in this issue (the path exists).
    Json(HeartbeatResponse {
        acknowledged: true,
        control: vec![],
    })
    .into_response()
}

async fn pull(State(_st): State<InternalState>, Json(req): Json<PullRequest>) -> Response {
    if let Err(e) = check_protocol_version(req.protocol_version) {
        tracing::warn!(error = %e, worker_id = %req.worker_id, "pull rejected: protocol mismatch");
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    // No claim authority yet (#135): always answer with no work.
    tracing::debug!(worker_id = %req.worker_id, "pull: no assignments (claim authority lands in #135)");
    Json(PullResponse {
        assignments: vec![],
    })
    .into_response()
}

async fn draining(State(st): State<InternalState>, Json(d): Json<Draining>) -> Response {
    st.registry.mark_draining(&d).await;
    ack()
}

async fn released(State(st): State<InternalState>, Json(r): Json<Released>) -> Response {
    st.registry.note_released(&r).await;
    ack()
}
