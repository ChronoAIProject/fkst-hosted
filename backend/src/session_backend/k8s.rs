//! The direct-Kubernetes [`SessionBackend`] implementation (issue #412).
//!
//! This is the concrete pod-driving machinery moved verbatim out of the reconciler
//! (`execute.rs` / `repo.rs` / the sweep in `loops.rs`) and placed behind the
//! backend-neutral [`SessionBackend`] contract. Everything here — the pod LIST +
//! `LivePod` projection, the last-pending merge-patch, the grace-honouring delete,
//! the terminal background delete, and the pod/Secret create — is the SAME code with
//! the SAME logs and the SAME 404/409 tolerance the reconciler had inline; only its
//! HOME changed. The reconciler now reaches it only through `Arc<dyn SessionBackend>`.

use std::collections::{BTreeMap, HashSet};

use async_trait::async_trait;
use k8s_openapi::api::core::v1::Pod;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::{DateTime, Utc};
use kube::api::{Api, DeleteParams, ListParams, Patch, PatchParams};
use secrecy::SecretString;

use crate::config::PodConfig;
use crate::k8s::session_launcher::{
    ANNOTATION_CONFIG_HASH, ANNOTATION_INSTALLATION, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER,
    ANNOTATION_REPO, ANNOTATION_TRIGGER_ISSUE, ANNOTATION_WORK_LABEL, COMPONENT_LABEL_KEY,
    COMPONENT_LABEL_VALUE, SESSION_ID_LABEL,
};
use crate::k8s::{
    build_session_pod, create_session_pod, session_object_name, KubeClient, SessionPodOutcome,
    SessionPodSpec,
};
use crate::models::RepoRef;
use crate::reconcile::desired::{LivePod, PodLiveness};
use crate::reconcile::RepoKey;

use super::{BackendError, EnsureOutcome, KillReason, SessionBackend};

/// The direct-Kubernetes session backend: drives one long-lived Pod (+ its
/// owner-referenced creds Secret) per substrate session. Cheap to clone via its
/// `Arc`-backed [`KubeClient`]; held by the reconciler as `Arc<dyn SessionBackend>`.
pub struct K8sBackend {
    kube: KubeClient,
    pod_config: PodConfig,
    termination_grace_secs: u64,
}

impl K8sBackend {
    /// Build from the namespace-bound Kubernetes client, the pod-launch knobs, and
    /// the configured termination grace (the delete drain window).
    pub fn new(kube: KubeClient, pod_config: PodConfig, termination_grace_secs: u64) -> Self {
        Self {
            kube,
            pod_config,
            termination_grace_secs,
        }
    }

    /// A namespaced Pod API bound to the reconciler's namespace.
    fn pods_api(&self) -> Api<Pod> {
        Api::namespaced(self.kube.client().clone(), self.kube.namespace())
    }
}

#[async_trait]
impl SessionBackend for K8sBackend {
    async fn check_reachable(&self) -> Result<String, BackendError> {
        self.kube
            .check_reachable()
            .await
            .map_err(|e| BackendError::Other(anyhow::Error::new(e)))
    }

    async fn ensure_session(
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

    async fn observe_repo(&self, repo: &RepoRef) -> Result<Vec<LivePod>, BackendError> {
        let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
        let list = self
            .pods_api()
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(|e| BackendError::Other(anyhow::Error::new(e)))?;
        Ok(list
            .items
            .iter()
            .filter(|pod| pod_matches_repo(pod, repo))
            .filter_map(pod_to_live)
            .collect())
    }

    async fn mark_pending(&self, session_id: &str) -> Result<(), BackendError> {
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

    async fn stop_session(
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

    async fn remove_terminal(&self, session_id: &str) -> Result<(), BackendError> {
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

/// LIST the substrate-session pods, group them into the `(installation, repo)` keys
/// they belong to via their stamped annotations, and return the unique set. Used by
/// the sweep to enqueue every repo that currently has a live pod; the `active_repos`
/// merge stays in the sweep. A list error is returned as [`BackendError::Other`].
pub async fn live_repo_keys(kube: &KubeClient) -> Result<HashSet<RepoKey>, BackendError> {
    let pods: Api<Pod> = Api::namespaced(kube.client().clone(), kube.namespace());
    let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
    let list = pods
        .list(&ListParams::default().labels(&selector))
        .await
        .map_err(|e| BackendError::Other(anyhow::Error::new(e)))?;

    let mut keys: HashSet<RepoKey> = HashSet::new();
    for pod in &list.items {
        if let Some(key) = repo_key_from_pod(pod) {
            keys.insert(key);
        }
    }
    Ok(keys)
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

/// Read a pod annotation as `&str`, if present.
fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .map(String::as_str)
}

/// Recover the `(installation, repo)` reconcile key a live pod belongs to from its
/// stamped annotations. `None` when any of the three annotations is missing /
/// unparseable (the pod is not one of ours, or is malformed). Used by the sweep to
/// enqueue every repo that currently has a live pod.
pub fn repo_key_from_pod(pod: &Pod) -> Option<(i64, RepoRef)> {
    let owner = annotation(pod, ANNOTATION_OWNER)?;
    let name = annotation(pod, ANNOTATION_REPO)?;
    let installation = annotation(pod, ANNOTATION_INSTALLATION)?
        .parse::<i64>()
        .ok()?;
    Some((
        installation,
        RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
    ))
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
#[path = "k8s_tests.rs"]
mod tests;
