//! `GET /api/v1/sessions/{owner}/{repo}/{issue}` and `…/stop`: the K8s-backed
//! session read + stop surface.
//!
//! v1 model: one GitHub issue = one fkst-substrate session = one Kubernetes Job
//! (`fkst-sess-<id>`). There is no in-memory session store — these handlers read
//! the Job (and its pod) directly from the Kubernetes API, finding it by the
//! `owner`/`repo`/`issue-number` annotations the launcher stamps. `stop` deletes
//! the Job (cascading the pod + the owner-referenced Secret).
//!
//! The find/read/stop machinery lives in [`crate::routes::session_ops`] so the
//! issue-comment control path (`/stop`, `/status`) drives the exact same logic.

use axum::extract::{Path, State};
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::k8s::job_disposition;
use crate::routes::session_ops::{
    delete_session_job, engine_version, find_session_job, job_pod, kube_client, status_str,
};
use crate::state::AppState;

/// The rich session view returned by `GET`.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionView {
    /// `pending` | `running` | `completed` | `failed`.
    pub session_status: &'static str,
    /// The pod's name, once scheduled.
    pub pod_id: Option<String>,
    /// The pod's start time (RFC3339, UTC).
    pub start_timestamp: Option<String>,
    /// The fkst-substrate engine version baked into the runner image.
    pub fkst_substrate_version: Option<String>,
    /// `https://github.com/<owner>/<repo>`.
    pub github_repo_url: String,
    /// The triggering issue number.
    pub github_issue_number: i64,
}

/// Confirmation that a stop was issued.
#[derive(Debug, Serialize, ToSchema)]
pub struct StopResponse {
    pub stopped: bool,
}

/// `GET /api/v1/sessions/{owner}/{repo}/{issue}` — the rich, K8s-backed view.
#[utoipa::path(
    get,
    path = "/sessions/{owner}/{repo}/{issue}",
    tag = "sessions",
    operation_id = "get_session",
    params(
        ("owner" = String, Path, description = "GitHub repo owner"),
        ("repo" = String, Path, description = "GitHub repo name"),
        ("issue" = i64, Path, description = "GitHub issue number"),
    ),
    responses(
        (status = 200, description = "The session view", body = SessionView),
        (status = 404, description = "No session for this issue", body = ErrorEnvelope),
    )
)]
async fn get_one(
    State(state): State<AppState>,
    Path((owner, repo, issue)): Path<(String, String, i64)>,
) -> Result<Json<SessionView>, AppError> {
    let kube = kube_client(&state).await?;
    let job = find_session_job(&kube, &owner, &repo, issue)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("no session for {owner}/{repo}#{issue}")))?;
    let job_name = job
        .metadata
        .name
        .clone()
        .unwrap_or_else(|| "fkst-sess".to_string());

    let pod = job_pod(&kube, &job_name).await;
    let pod_id = pod.as_ref().and_then(|p| p.metadata.name.clone());
    let start_timestamp = pod
        .as_ref()
        .and_then(|p| p.status.as_ref())
        .and_then(|s| s.start_time.as_ref())
        .map(|t| t.0.to_rfc3339());

    Ok(Json(SessionView {
        session_status: status_str(job_disposition(&job), pod_id.is_some()),
        pod_id,
        start_timestamp,
        fkst_substrate_version: engine_version(),
        github_repo_url: format!("https://github.com/{owner}/{repo}"),
        github_issue_number: issue,
    }))
}

/// `POST /api/v1/sessions/{owner}/{repo}/{issue}/stop` — delete the Job.
#[utoipa::path(
    post,
    path = "/sessions/{owner}/{repo}/{issue}/stop",
    tag = "sessions",
    operation_id = "stop_session",
    params(
        ("owner" = String, Path, description = "GitHub repo owner"),
        ("repo" = String, Path, description = "GitHub repo name"),
        ("issue" = i64, Path, description = "GitHub issue number"),
    ),
    responses(
        (status = 200, description = "Stop issued (or already gone)", body = StopResponse),
    )
)]
async fn stop(
    State(state): State<AppState>,
    Path((owner, repo, issue)): Path<(String, String, i64)>,
) -> Result<Json<StopResponse>, AppError> {
    let kube = kube_client(&state).await?;
    let Some(job) = find_session_job(&kube, &owner, &repo, issue).await? else {
        // Idempotent: nothing to stop.
        return Ok(Json(StopResponse { stopped: false }));
    };
    let job_name = job.metadata.name.clone().unwrap_or_default();
    // Background propagation deletes the pod (and cascade-deletes the
    // owner-referenced Secret) along with the Job; a 404 is already-gone.
    delete_session_job(&kube, &job_name).await?;
    tracing::info!(owner, repo, issue, "session stop: deleted job");
    Ok(Json(StopResponse { stopped: true }))
}

/// The sessions router (nested under `/api/v1`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_one, stop))
}
