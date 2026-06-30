//! `GET /api/v1/sessions/{owner}/{repo}/{issue}` and `…/stop`: the K8s-backed
//! session read + stop surface.
//!
//! v1 model: one GitHub issue = one fkst-substrate session = one Kubernetes Job
//! (`fkst-sess-<id>`). There is no in-memory session store — these handlers read
//! the Job (and its pod) directly from the Kubernetes API, finding it by the
//! `owner`/`repo`/`issue-number` annotations the launcher stamps. `stop` deletes
//! the Job (cascading the pod + the owner-referenced Secret).

use axum::extract::{Path, State};
use axum::Json;
use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams};
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::k8s::{job_disposition, JobDisposition, KubeClient};
use crate::state::AppState;

/// Label selector matching the per-session Jobs the launcher creates.
const SESSION_SELECTOR: &str = "app.kubernetes.io/component=session";
/// Path of the engine-version provenance file baked into the image.
const ENGINE_VERSION_FILE: &str = "/etc/fkst-engine-version";

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

fn status_str(disposition: JobDisposition, has_pod: bool) -> &'static str {
    match disposition {
        JobDisposition::Completed => "completed",
        JobDisposition::Failed => "failed",
        // A Job with no running pod yet is still scheduling.
        JobDisposition::Running if has_pod => "running",
        JobDisposition::Running => "pending",
    }
}

/// Read the engine version baked into the image (`None` outside a built image).
fn engine_version() -> Option<String> {
    std::fs::read_to_string(ENGINE_VERSION_FILE)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Build a Kubernetes client for the configured sessions namespace, or a clear
/// error when pod dispatch is not configured / the cluster is unreachable.
async fn kube_client(state: &AppState) -> Result<KubeClient, AppError> {
    if !state.config.pod.dispatch {
        return Err(AppError::Validation(
            "pod dispatch is not enabled (FKST_POD_DISPATCH=false)".to_string(),
        ));
    }
    KubeClient::from_inferred(&state.config.pod.namespace)
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("kubernetes client: {e}")))
}

/// Find the session Job for `(owner, repo, issue)` by the launcher's annotations.
async fn find_session_job(
    kube: &KubeClient,
    owner: &str,
    repo: &str,
    issue: i64,
) -> Result<Option<Job>, AppError> {
    let jobs: Api<Job> = Api::namespaced(kube.client().clone(), kube.namespace());
    let list = jobs
        .list(&ListParams::default().labels(SESSION_SELECTOR))
        .await
        .map_err(|e| AppError::Internal(anyhow::anyhow!("list session jobs: {e}")))?;
    let issue = issue.to_string();
    Ok(list.items.into_iter().find(|job| {
        job.metadata.annotations.as_ref().is_some_and(|a| {
            a.get("fkst.chrono-ai.fun/owner").map(String::as_str) == Some(owner)
                && a.get("fkst.chrono-ai.fun/repo").map(String::as_str) == Some(repo)
                && a.get("fkst.chrono-ai.fun/issue-number").map(String::as_str)
                    == Some(issue.as_str())
        })
    }))
}

/// The first pod of a Job (by the `job-name` label), if scheduled.
async fn job_pod(kube: &KubeClient, job_name: &str) -> Option<Pod> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    pods.list(&ListParams::default().labels(&format!("batch.kubernetes.io/job-name={job_name}")))
        .await
        .ok()?
        .items
        .into_iter()
        .next()
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
    let jobs: Api<Job> = Api::namespaced(kube.client().clone(), kube.namespace());
    // Background propagation deletes the pod (and cascade-deletes the
    // owner-referenced Secret) along with the Job.
    match jobs.delete(&job_name, &DeleteParams::background()).await {
        Ok(_) => {}
        Err(kube::Error::Api(e)) if e.code == 404 => {}
        Err(e) => {
            return Err(AppError::Internal(anyhow::anyhow!(
                "delete session job: {e}"
            )))
        }
    }
    tracing::info!(owner, repo, issue, "session stop: deleted job");
    Ok(Json(StopResponse { stopped: true }))
}

/// The sessions router (nested under `/api/v1`).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_one, stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_str_maps_disposition_and_pod_presence() {
        assert_eq!(status_str(JobDisposition::Completed, false), "completed");
        assert_eq!(status_str(JobDisposition::Failed, true), "failed");
        assert_eq!(status_str(JobDisposition::Running, true), "running");
        assert_eq!(status_str(JobDisposition::Running, false), "pending");
    }
}
