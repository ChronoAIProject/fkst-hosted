//! The session-lifecycle verbs (issue #412): ensure / observe / mark-pending /
//! stop / GC, plus the pod → [`LivePod`] projection they feed the planner. Moved
//! verbatim from the reconciler; the logs + 404/409 tolerance are unchanged.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{DeleteParams, Patch, PatchParams};
use secrecy::SecretString;

use crate::k8s::session_launcher::{
    ANNOTATION_CONFIG_HASH, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER, ANNOTATION_REPO,
    ANNOTATION_TRIGGER_ISSUE, ANNOTATION_WORK_LABEL, SESSION_ID_LABEL,
};
use crate::k8s::{
    build_session_pod, create_session_pod, session_object_name, SessionPodOutcome, SessionPodSpec,
};
use crate::models::RepoRef;
use crate::reconcile::desired::{LivePod, PodLiveness};

use super::super::{BackendError, EnsureOutcome, KillReason};
use super::{annotation, K8sBackend};

impl K8sBackend {
    pub(super) async fn ensure_session_impl(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        // Build the pod (409 = already-live no-op). The creds Secret is built +
        // owner-referenced to the created pod inside `create_session_pod`.
        let pod = match build_session_pod(spec, &self.pod_config) {
            Ok(pod) => pod,
            Err(error) => {
                tracing::error!(session_id = %spec.session_id, error = %error, "reconcile spawn: pod build failed; not spawning");
                return Err(BackendError::Other(anyhow::Error::new(error)));
            }
        };
        match create_session_pod(self.kube.client(), spec, pod, creds).await {
            Ok(SessionPodOutcome::Created) => Ok(EnsureOutcome::Created),
            Ok(SessionPodOutcome::AlreadyLive) => Ok(EnsureOutcome::AlreadyLive),
            Err(error) => {
                tracing::error!(session_id = %spec.session_id, error = %error, "reconcile spawn: session pod create failed");
                Err(BackendError::Other(anyhow::Error::new(error)))
            }
        }
    }

    pub(super) async fn observe_repo_impl(
        &self,
        repo: &RepoRef,
    ) -> Result<Vec<LivePod>, BackendError> {
        let pods = self.list_component_pods().await?;
        Ok(pods
            .iter()
            .filter(|pod| pod_matches_repo(pod, repo))
            .filter_map(pod_to_live)
            .collect())
    }

    pub(super) async fn mark_pending_impl(&self, session_id: &str) -> Result<(), BackendError> {
        let name = session_object_name(session_id);
        let patch = last_pending_patch(Utc::now());
        match self
            .pods_api()
            .patch(&name, &PatchParams::default(), &Patch::Merge(patch))
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Err(BackendError::NotFound),
            Err(error) => Err(BackendError::Other(anyhow::Error::new(error))),
        }
    }

    pub(super) async fn stop_session_impl(
        &self,
        session_id: &str,
        _reason: KillReason,
    ) -> Result<(), BackendError> {
        // `_reason` is part of the contract (the executor logs it); the delete itself
        // does not need it — only the configured termination grace.
        let name = session_object_name(session_id);
        let params = kill_delete_params(self.termination_grace_secs);
        match self.pods_api().delete(&name, &params).await {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Err(BackendError::NotFound),
            Err(error) => Err(BackendError::Other(anyhow::Error::new(error))),
        }
    }

    pub(super) async fn remove_terminal_impl(&self, session_id: &str) -> Result<(), BackendError> {
        let name = session_object_name(session_id);
        match self
            .pods_api()
            .delete(&name, &DeleteParams::background())
            .await
        {
            Ok(_) => Ok(()),
            Err(kube::Error::Api(e)) if e.code == 404 => Err(BackendError::NotFound),
            Err(error) => Err(BackendError::Other(anyhow::Error::new(error))),
        }
    }
}

/// The JSON merge patch that sets `last-pending-at` to `now` (RFC3339). Pure +
/// unit-tested so the annotation key + shape can never drift from the builder.
fn last_pending_patch(now: DateTime<Utc>) -> serde_json::Value {
    let annotations = serde_json::Map::from_iter([(
        ANNOTATION_LAST_PENDING_AT.to_string(),
        serde_json::Value::String(now.to_rfc3339()),
    )]);
    serde_json::json!({ "metadata": { "annotations": serde_json::Value::Object(annotations) } })
}

/// `DeleteParams` carrying the drain window (`terminationGracePeriodSeconds`). Pure
/// + unit-tested. A grace beyond `u32::MAX` is clamped (never realistically hit).
fn kill_delete_params(grace_secs: u64) -> DeleteParams {
    DeleteParams {
        grace_period_seconds: Some(u32::try_from(grace_secs).unwrap_or(u32::MAX)),
        ..DeleteParams::default()
    }
}

/// Whether a listed pod's owner/repo annotations match `repo` (the LIST selector
/// spans every repo + installation, so this scopes it to the one being reconciled).
fn pod_matches_repo(pod: &Pod, repo: &RepoRef) -> bool {
    annotation(pod, ANNOTATION_OWNER) == Some(repo.owner.as_str())
        && annotation(pod, ANNOTATION_REPO) == Some(repo.name.as_str())
}

/// Project the coarse liveness from the pod phase + deletion state: a set
/// `deletionTimestamp` always wins (Terminating); else Pending→Starting,
/// Running→Live, Succeeded/Failed→Terminal, anything else (Unknown / not-yet-set)
/// → Starting (not yet observed running).
fn phase_to_liveness(phase: Option<&str>, terminating: bool) -> PodLiveness {
    if terminating {
        return PodLiveness::Terminating;
    }
    match phase {
        Some("Running") => PodLiveness::Live,
        Some("Succeeded") | Some("Failed") => PodLiveness::Terminal,
        _ => PodLiveness::Starting,
    }
}

/// Project one pod into a [`LivePod`]. `None` when the pod carries no session-id
/// label (not one of ours / malformed) — such a pod is skipped, never planned on.
fn pod_to_live(pod: &Pod) -> Option<LivePod> {
    let session_id = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SESSION_ID_LABEL))
        .cloned()?;

    let trigger_issue = annotation(pod, ANNOTATION_TRIGGER_ISSUE)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    let terminating = pod.metadata.deletion_timestamp.is_some();
    let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
    let liveness = phase_to_liveness(phase, terminating);

    // creationTimestamp is always present on a real pod; default to now so a
    // malformed pod is treated as freshly created (shielded from idle-kill) rather
    // than instantly idle.
    let created_at = pod
        .metadata
        .creation_timestamp
        .as_ref()
        .map(|Time(t)| *t)
        .unwrap_or_else(Utc::now);

    let last_pending_at = annotation(pod, ANNOTATION_LAST_PENDING_AT)
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));

    let config_hash = annotation(pod, ANNOTATION_CONFIG_HASH).map(str::to_string);

    // The work label lets the planner retire-notify this session's still-open work
    // issues when the pod is orphaned (its trigger issue closed).
    let work_label = annotation(pod, ANNOTATION_WORK_LABEL).map(str::to_string);

    Some(LivePod {
        session_id,
        trigger_issue,
        liveness,
        created_at,
        last_pending_at,
        config_hash,
        work_label,
    })
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
