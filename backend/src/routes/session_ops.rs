//! Shared session-Job query + control helpers.
//!
//! The issue-comment control path
//! (`routes::github_app_webhook::comment_control`) locates, reads, and stops a
//! session's Kubernetes Job through THESE functions — a session is controlled
//! solely through its GitHub issue (`/status` reads, `/stop` deletes the Job),
//! so there is one place that knows how to "find / read / stop a session".

use k8s_openapi::api::batch::v1::Job;
use k8s_openapi::api::core::v1::Pod;
use kube::api::{Api, DeleteParams, ListParams};

use crate::error::AppError;
use crate::k8s::{JobDisposition, KubeClient};
use crate::state::AppState;

/// Label selector matching the per-session Jobs the launcher creates.
const SESSION_SELECTOR: &str = "app.kubernetes.io/component=session";
/// Path of the engine-version provenance file baked into the image.
const ENGINE_VERSION_FILE: &str = "/etc/fkst-engine-version";

/// Map a Job's disposition + pod presence to the public status string.
pub(crate) fn status_str(disposition: JobDisposition, has_pod: bool) -> &'static str {
    match disposition {
        JobDisposition::Completed => "completed",
        JobDisposition::Failed => "failed",
        // A Job with no running pod yet is still scheduling.
        JobDisposition::Running if has_pod => "running",
        JobDisposition::Running => "pending",
    }
}

/// Read the engine version baked into the image (`None` outside a built image).
pub(crate) fn engine_version() -> Option<String> {
    std::fs::read_to_string(ENGINE_VERSION_FILE)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Build a Kubernetes client for the configured sessions namespace, or a clear
/// error when pod dispatch is not configured / the cluster is unreachable.
pub(crate) async fn kube_client(state: &AppState) -> Result<KubeClient, AppError> {
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
pub(crate) async fn find_session_job(
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
pub(crate) async fn job_pod(kube: &KubeClient, job_name: &str) -> Option<Pod> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    pods.list(&ListParams::default().labels(&format!("batch.kubernetes.io/job-name={job_name}")))
        .await
        .ok()?
        .items
        .into_iter()
        .next()
}

/// Delete a session Job by name with background propagation, which cascades the
/// pod and the Job-owner-referenced Secret. A `404` is treated as success (the
/// Job is already gone), so the call is idempotent — both the REST stop and a
/// `/stop` comment can safely re-issue it on a webhook redelivery.
pub(crate) async fn delete_session_job(kube: &KubeClient, job_name: &str) -> Result<(), AppError> {
    let jobs: Api<Job> = Api::namespaced(kube.client().clone(), kube.namespace());
    match jobs.delete(job_name, &DeleteParams::background()).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(e)) if e.code == 404 => Ok(()),
        Err(e) => Err(AppError::Internal(anyhow::anyhow!(
            "delete session job: {e}"
        ))),
    }
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
