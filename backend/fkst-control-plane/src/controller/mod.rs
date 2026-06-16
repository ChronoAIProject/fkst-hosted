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

pub mod claims;
pub mod handle;
pub mod internal_auth;
pub mod placement;
pub mod reassign;
pub mod registry;
pub mod token_minter;

pub use claims::{ClaimEntry, ClaimError, ClaimMap, ClaimStatus, FencingId};
pub use handle::ControllerHandle;
pub use placement::{place, select_worker, Placement, PlacementError, WorkerLoad};
pub use reassign::{NoopSecretRedispatch, ReassignDriver, SecretRedispatch};
pub use token_minter::{
    GithubAppMinter, MintResult, SessionTokenMinter, MAX_CONSECUTIVE_MINT_FAILURES,
};

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};

use fkst_shared::models::SessionStatus as ClaimSessionStatus;
use fkst_shared::protocol::{
    check_protocol_version, CredentialRefreshRequest, CredentialRefreshResponse, Draining,
    Heartbeat, HeartbeatResponse, PullRequest, PullResponse, RefreshedToken, RegisterRequest,
    RegisterResponse, Released, SessionStatus as WireSessionStatus, StatusReport,
    INTERNAL_AUTH_HEADER, PROTOCOL_VERSION,
};

pub use internal_auth::InternalAuth;
pub use registry::{WorkerEntry, WorkerRegistry};

/// Shared state for the internal handlers.
#[derive(Clone)]
struct InternalState {
    registry: WorkerRegistry,
    heartbeat_interval_secs: u64,
    /// The controller's claim authority. The credential-refresh and
    /// status-report handlers fence-guard against this map (#151). In prod it is
    /// a fresh empty map (placement does not insert into it yet), so the two new
    /// routes are reachable but inert until a later increment populates it.
    claims: Arc<ClaimMap>,
    /// Session token minter (#151). `None` when the GitHub App is unconfigured,
    /// in which case credential-refresh answers `503`.
    minter: Option<Arc<dyn SessionTokenMinter>>,
}

/// Build the internal worker-protocol router, guarded by the shared secret.
/// `heartbeat_interval_secs` is the authoritative cadence the controller hands
/// back to workers on registration. `claims` is the controller's claim authority
/// (fence-guards the mid-run channels, #151); `minter` mints session tokens for
/// credential-refresh (`None` => the App is unconfigured and refresh answers
/// `503`).
pub fn internal_router(
    registry: WorkerRegistry,
    auth: InternalAuth,
    heartbeat_interval_secs: u64,
    claims: Arc<ClaimMap>,
    minter: Option<Arc<dyn SessionTokenMinter>>,
) -> Router {
    let state = InternalState {
        registry,
        heartbeat_interval_secs,
        claims,
        minter,
    };
    Router::new()
        .route("/internal/v1/register", post(register))
        .route("/internal/v1/heartbeat", post(heartbeat))
        .route("/internal/v1/pull", post(pull))
        .route("/internal/v1/draining", post(draining))
        .route("/internal/v1/released", post(released))
        .route("/internal/v1/credential-refresh", post(credential_refresh))
        .route("/internal/v1/status-report", post(status_report))
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
    // Deliver any control messages queued for this worker (point-to-point, #151).
    // Dormant until activation enqueues: the queue is empty, so this returns
    // `control: vec![]` exactly as before. The drain is once-only — a message is
    // delivered to exactly one heartbeat.
    let control = st.registry.take_control(&hb.worker_id).await;
    Json(HeartbeatResponse {
        acknowledged: true,
        control,
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

/// Parse a wire `session_id` String into a `bson::Uuid`. `None` on a malformed
/// id; the caller maps that to a `400`. Centralised so both new handlers reject
/// a bad uuid identically (and logged once here, never the id-bytes context).
fn parse_session_id(raw: &str) -> Option<bson::Uuid> {
    match bson::Uuid::parse_str(raw) {
        Ok(id) => Some(id),
        Err(_) => {
            tracing::warn!("internal request rejected: malformed session_id");
            None
        }
    }
}

/// The shared `400` response for a malformed `session_id`.
fn bad_session_id() -> Response {
    (StatusCode::BAD_REQUEST, "invalid session_id").into_response()
}

/// Milliseconds since the Unix epoch for `t`, saturating at 0 for any pre-epoch
/// clock (a freshly-minted token expiry is always in the future, so this is a
/// defensive floor, not an expected path).
fn unix_ms(t: SystemTime) -> i64 {
    t.duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Map a worker-reported wire [`WireSessionStatus`] to the controller's
/// [`ClaimSessionStatus`] (=`ClaimStatus`) plus the allowed `from` set the
/// fence-guarded transition accepts.
///
/// Mapping rationale (the claim lifecycle is
/// `Pending -> Validating -> Running -> {Stopped|Failed}`):
/// - `Validating` is the first thing the engine reports off a fresh `Pending`
///   claim, so `from = [Pending]`.
/// - `Running` may follow either `Validating` or a `Pending` that skipped an
///   observable validating report, so `from = [Pending, Validating]`.
/// - `Stopped`/`Failed` are terminal and may be reached from any active state
///   (the engine can stop or fail at any point), so `from` is the full active
///   set `[Pending, Validating, Running, Stopping]`.
fn map_status(wire: WireSessionStatus) -> (ClaimSessionStatus, &'static [ClaimSessionStatus]) {
    use ClaimSessionStatus::{Failed, Pending, Running, Stopped, Stopping, Validating};
    const ACTIVE: &[ClaimSessionStatus] = &[Pending, Validating, Running, Stopping];
    match wire {
        WireSessionStatus::Validating => (Validating, &[Pending]),
        WireSessionStatus::Running => (Running, &[Pending, Validating]),
        WireSessionStatus::Stopped => (Stopped, ACTIVE),
        WireSessionStatus::Failed => (Failed, ACTIVE),
    }
}

/// Worker -> controller mid-run credential refresh (#151). Fence-guarded: a
/// superseded worker is refused a token (never a token, only `credentials:
/// None`). When the App is unconfigured the minter is absent and the route
/// answers `503`. Never logs the token value.
async fn credential_refresh(
    State(st): State<InternalState>,
    Json(req): Json<CredentialRefreshRequest>,
) -> Response {
    if let Err(e) = check_protocol_version(req.protocol_version) {
        tracing::warn!(error = %e, worker_id = %req.worker_id, "credential-refresh rejected: protocol mismatch");
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    let session_id = match parse_session_id(&req.session_id) {
        Some(id) => id,
        None => return bad_session_id(),
    };

    // Fence guard FIRST: a superseded worker never reaches the minter, so it can
    // never be handed a token (the at-most-one-engine guarantee on this channel).
    if !st.claims.fence_ok_for_session(session_id, req.fencing_id) {
        tracing::info!(
            worker_id = %req.worker_id,
            session_id = %session_id,
            reason = ?req.reason,
            "credential-refresh refused: stale fence (no token returned)"
        );
        return Json(CredentialRefreshResponse {
            credentials: None,
            gone: false,
        })
        .into_response();
    }

    let Some(minter) = st.minter.as_ref() else {
        tracing::warn!(
            session_id = %session_id,
            "credential-refresh unavailable: github app minter not configured"
        );
        return (StatusCode::SERVICE_UNAVAILABLE, "minting unavailable").into_response();
    };

    tracing::debug!(
        worker_id = %req.worker_id,
        session_id = %session_id,
        reason = ?req.reason,
        "credential-refresh: fence ok, minting"
    );
    match minter.mint(session_id, &req.repo_ref).await {
        MintResult::Token { token, expires_at } => Json(CredentialRefreshResponse {
            credentials: Some(RefreshedToken {
                token,
                expires_at_unix_ms: unix_ms(expires_at),
            }),
            gone: false,
        })
        .into_response(),
        MintResult::Gone => Json(CredentialRefreshResponse {
            credentials: None,
            gone: true,
        })
        .into_response(),
        // Transient: the worker keeps its current token and retries later.
        MintResult::Failed => Json(CredentialRefreshResponse {
            credentials: None,
            gone: false,
        })
        .into_response(),
    }
}

/// Worker -> controller session status report (#151). Fence-guarded via
/// [`ClaimMap::set_status_for_session`]: a stale fence (or unknown session, or a
/// disallowed transition) is a no-op that answers `{"applied": false}` — never
/// an error, so a superseded worker cannot overwrite the claim's status.
async fn status_report(State(st): State<InternalState>, Json(req): Json<StatusReport>) -> Response {
    if let Err(e) = check_protocol_version(req.protocol_version) {
        tracing::warn!(error = %e, worker_id = %req.worker_id, "status-report rejected: protocol mismatch");
        return (StatusCode::BAD_REQUEST, e.to_string()).into_response();
    }
    let session_id = match parse_session_id(&req.session_id) {
        Some(id) => id,
        None => return bad_session_id(),
    };

    let (to, from) = map_status(req.status);
    let applied = st
        .claims
        .set_status_for_session(session_id, req.fencing_id, from, to);
    tracing::debug!(
        worker_id = %req.worker_id,
        session_id = %session_id,
        status = ?req.status,
        applied,
        "status-report processed"
    );
    Json(serde_json::json!({ "applied": applied })).into_response()
}

#[cfg(test)]
#[path = "internal_tests.rs"]
mod internal_tests;
