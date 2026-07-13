//! The identity-gated engine observe read-model endpoint (issue #473):
//! `GET /api/v1/sessions/{session_id}/observe`.
//!
//! Runs `fkst-framework observe --json` INSIDE the session's runtime through
//! the [`SessionBackend`](crate::session_backend::SessionBackend) abstraction
//! (pods/exec on Kubernetes, execd poll-and-assemble on OpenSandbox) and
//! returns the engine's snapshot verbatim: per-queue depth / in-flight /
//! retrying, DLQ tombstones, and codex-run records. The engine never emits
//! payload bodies (only schema/digest/byte counts), so the snapshot is safe to
//! serve to an authorized viewer.
//!
//! AUTHZ is byte-identical to the log-download endpoint: the same three-tier
//! check (trigger author / per-issue `### Log Access Allowlist` / global
//! `FKST_LOG_ADMINS`) via [`crate::routes::logs::authorize`] over the
//! reconciler-maintained registry. Someone allowed to read a session's logs is
//! allowed to read its queue state — one grant, two read-only views.
//!
//! Secret hygiene: the snapshot carries no credentials; error envelopes carry
//! fixed, generic messages (backend failure detail stays in the logs).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::session_backend::ObserveError;
use crate::state::AppState;

/// The engine's own `--limit` bounds (`observe.rs` clamps 1..=10000, default 500).
const DEFAULT_LIMIT: u32 = 500;
const MIN_LIMIT: u32 = 1;
const MAX_LIMIT: u32 = 10_000;

/// Query parameters for the observe endpoint.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct ObserveQuery {
    /// Maximum delivery entries in the snapshot; clamped to the engine's
    /// accepted 1..=10000 (default 500).
    pub limit: Option<u32>,
}

/// Serve the engine observe snapshot for one session.
#[utoipa::path(
    get,
    path = "/sessions/{session_id}/observe",
    tag = "sessions",
    operation_id = "observe_session",
    params(
        ("session_id" = String, Path, description = "The session id (from the trigger announce comment)"),
        ObserveQuery,
    ),
    responses(
        (status = 200, description = "The engine's observe read-model snapshot (raw engine JSON: queue depths, in-flight/retrying, DLQ tombstones, codex runs — never payload bodies)", body = serde_json::Value),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not the session author, allowlisted, or a log admin", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, or its runtime is gone", body = ErrorEnvelope),
        (status = 409, description = "The session has no durable delivery store to observe (its packages declare no reliable subscriptions)", body = ErrorEnvelope),
        (status = 503, description = "Session dispatch is disabled, or the runtime exec failed", body = ErrorEnvelope),
    )
)]
async fn observe_session(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    Query(query): Query<ObserveQuery>,
    user: GithubUser,
) -> Result<Json<serde_json::Value>, AppError> {
    // Same deny-by-default three-tier gate as the log download (404 for an
    // unknown session, 403 for an unauthorized caller).
    crate::routes::logs::authorize(&state, &session_id, &user)?;

    let Some(backend) = state.session_backend.as_ref() else {
        return Err(AppError::Unavailable(
            "session dispatch is disabled on this deployment".to_string(),
        ));
    };
    let limit = query
        .limit
        .unwrap_or(DEFAULT_LIMIT)
        .clamp(MIN_LIMIT, MAX_LIMIT);

    let raw = backend
        .engine_observe(&session_id, limit)
        .await
        .map_err(|error| map_observe_error(&session_id, error))?;

    // Parse-then-serve: proves the payload is the engine's JSON document (a
    // corrupted/interleaved capture becomes a clean 503, never garbage 200).
    let snapshot: serde_json::Value = serde_json::from_str(&raw).map_err(|error| {
        tracing::warn!(session_id = %session_id, error = %error, "observe: engine output was not valid JSON");
        AppError::Unavailable("the engine returned an unreadable snapshot".to_string())
    })?;
    Ok(Json(snapshot))
}

/// Map a backend [`ObserveError`] onto the public error surface. Detail stays
/// in the backend's own logs; the envelope carries only fixed messages.
fn map_observe_error(session_id: &str, error: ObserveError) -> AppError {
    match error {
        ObserveError::SessionNotFound => {
            AppError::NotFound("no live runtime for this session".to_string())
        }
        ObserveError::NoDurableStore => {
            AppError::Conflict("this session has no durable delivery store to observe".to_string())
        }
        ObserveError::Failed(detail) => {
            tracing::warn!(session_id = %session_id, detail = %detail, "observe: backend exec failed");
            AppError::Unavailable("could not read the session's observe snapshot".to_string())
        }
    }
}

/// The observe router, merged into the `/api/v1` subtree.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(observe_session))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn observe_errors_map_to_the_documented_statuses() {
        assert!(matches!(
            map_observe_error("s", ObserveError::SessionNotFound),
            AppError::NotFound(_)
        ));
        assert!(matches!(
            map_observe_error("s", ObserveError::NoDurableStore),
            AppError::Conflict(_)
        ));
        assert!(matches!(
            map_observe_error("s", ObserveError::Failed("x".to_string())),
            AppError::Unavailable(_)
        ));
    }
}
