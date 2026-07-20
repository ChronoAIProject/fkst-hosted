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

use axum::extract::{Path, State};
use axum::Json;

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
    Path(session_id): Path<String>,
    user: GithubUser,
) -> Result<Json<Vec<LogRun>>, AppError> {
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
        // Legacy session: no run index yet → surface the single latest bundle as one
        // synthetic run so `?run=latest` still reaches it.
        Err(StorageError::Status { status: 404 }) => Ok(Json(vec![LogRun {
            run_id: "latest".to_string(),
            started_at: String::new(),
            ended_at: None,
        }])),
        Err(err) => {
            tracing::warn!(session_id = %session_id, error = %err, "runs index download failed");
            Err(AppError::Upstream("log storage error".to_string()))
        }
    }
}
