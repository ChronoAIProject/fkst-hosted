//! The per-run listing endpoint: `GET /api/v1/logs/{session_id}/runs`.
//!
//! A session is served by a SEQUENCE of pod incarnations (idle-reap → auto-revive);
//! each incarnation is one RUN with its own immutable bundle (per-pod-incarnation log
//! separation, issue #568). This enumerates them from the per-session run index
//! (`logs/<sid>/runs.json`) so a caller can then download or view a specific run via
//! `?run=<run_id>` on the download / manifest / file endpoints.
//!
//! It authorizes IDENTICALLY to the rest of the log surface: the [`GithubUser`]
//! extractor establishes identity, then [`super::authorize`] runs the same
//! deny-by-default three-tier check (unknown session → 404, unauthorized caller → 403).
//! Legacy sessions (bundled before per-run separation) have no run index; their single
//! `latest.tar.gz` is surfaced as one synthetic run with id `latest`, so `?run=latest`
//! keeps working for them.
//!
//! That fallback is conditional on the bundle ACTUALLY EXISTING (#5765). A missing
//! run index means either "legacy session with a latest bundle" or "session that has
//! never flushed a log" -- opposite situations that need opposite answers. Advertising
//! a synthetic run for the second made a brand-new or idle session render as a hard
//! error: the viewer trusted the descriptor, asked for its manifest, got a 404, and
//! showed "Unable to load session logs" with a retry that could never succeed.

use axum::extract::State;
use axum::http::Extensions;
use axum::Json;

use crate::audit::arguments::logs::SafeListSessionRuns;
use crate::audit::arguments::{record_safe, AuditedPath};
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::session_pod::log_stream::runs::{self, LogRun};
use crate::state::AppState;
use crate::storage::StorageError;

/// `GET /api/v1/logs/{session_id}/runs`.
#[utoipa::path(
    get,
    path = "/logs/{session_id}/runs",
    tag = "logs",
    operation_id = "list_session_runs",
    params(("session_id" = String, Path, description = "The deterministic session id")),
    responses(
        (status = 200, description = "The session's runs, newest first", body = Vec<LogRun>),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized for this session's logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session", body = ErrorEnvelope),
        (status = 502, description = "The run index could not be read", body = ErrorEnvelope),
        (status = 503, description = "Log storage is not configured", body = ErrorEnvelope),
    )
)]
pub(super) async fn list_session_runs(
    State(state): State<AppState>,
    extensions: Extensions,
    AuditedPath(session_id): AuditedPath<String>,
    user: GithubUser,
) -> Result<Json<Vec<LogRun>>, AppError> {
    // Recorded before authorization, so a denied read still describes which
    // session was asked for. The run index's object keys never leave storage.
    record_safe(&extensions, &SafeListSessionRuns::new(&session_id));
    super::record_session_correlation(&extensions, &session_id);
    // Same deny-by-default authorization as the download + viewer paths (unknown
    // session → 404, unauthorized caller → 403).
    super::authorize(&state, &session_id, &user)?;
    let Some(storage) = state.storage.as_ref() else {
        return Err(AppError::Unavailable(
            "log storage is not configured".to_string(),
        ));
    };
    let key = runs::runs_index_key(&session_id);
    match storage.download(&key).await {
        Ok(bytes) => {
            let mut list = runs::parse_runs(&bytes);
            // Newest first: by start time desc, then run id desc as a stable tiebreak.
            list.sort_by(|a, b| {
                b.started_at
                    .cmp(&a.started_at)
                    .then_with(|| b.run_id.cmp(&a.run_id))
            });
            Ok(Json(list))
        }
        // No run index. Legacy session ⇒ surface its single latest bundle as one
        // synthetic run so `?run=latest` still reaches it. Never-flushed session ⇒
        // an empty list, which is the honest answer and lets the viewer render its
        // empty state instead of chasing a manifest that does not exist.
        //
        // 404 is deliberately NOT reused for this: on this endpoint it already means
        // "unknown session", and overloading it would make "no logs yet" and "no such
        // session" indistinguishable to a caller.
        Err(StorageError::Status { status: 404 }) => {
            let bundle = super::object_key_for(&session_id, None);
            match storage.exists(&bundle).await {
                Ok(true) => Ok(Json(vec![LogRun {
                    run_id: "latest".to_string(),
                    started_at: String::new(),
                    ended_at: None,
                }])),
                Ok(false) => Ok(Json(Vec::new())),
                // Storage could not answer. Reporting "no logs" here would turn a
                // transient outage into a confident, wrong empty state.
                Err(err) => {
                    tracing::warn!(
                        session_id = %session_id,
                        error = %err,
                        "latest-bundle probe failed while resolving an absent run index"
                    );
                    Err(AppError::Upstream("log storage error".to_string()))
                }
            }
        }
        Err(err) => {
            tracing::warn!(session_id = %session_id, error = %err, "runs index download failed");
            Err(AppError::Upstream("log storage error".to_string()))
        }
    }
}
