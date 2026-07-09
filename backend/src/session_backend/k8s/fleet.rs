//! The fleet-enumeration verb (issue #413): LIST every substrate-session pod and
//! project each into a kube-free [`SessionHandle`] the loops address a session by.
//! The `(installation, repo)` recovery + the trigger-issue parse are the SAME
//! projections the token-rotation / health-scrape / sweep loops did inline.

use k8s_openapi::api::core::v1::Pod;

use crate::k8s::session_launcher::{
    ANNOTATION_INSTALLATION, ANNOTATION_OWNER, ANNOTATION_REPO, ANNOTATION_TRIGGER_ISSUE,
    SESSION_ID_LABEL,
};
use crate::models::RepoRef;

use super::super::{BackendError, SessionHandle};
use super::{annotation, K8sBackend};

impl K8sBackend {
    pub(super) async fn list_fleet_impl(&self) -> Result<Vec<SessionHandle>, BackendError> {
        let pods = self.list_component_pods().await?;
        Ok(pods.iter().filter_map(pod_to_handle).collect())
    }
}

/// Project one listed pod into a [`SessionHandle`]. `None` when the pod is not fully
/// one of ours — it carries no session-id label, or its owner/repo/installation
/// annotations do not resolve. The trigger issue is optional (a pod predating it, or
/// carrying an unparseable/zero value, still yields a handle with `trigger_issue: None`).
fn pod_to_handle(pod: &Pod) -> Option<SessionHandle> {
    let session_id = pod
        .metadata
        .labels
        .as_ref()
        .and_then(|l| l.get(SESSION_ID_LABEL))
        .cloned()?;
    let (installation_id, repo) = repo_key_from_pod(pod)?;
    let trigger_issue = trigger_issue_from_pod(pod);
    Some(SessionHandle {
        session_id,
        installation_id,
        repo,
        trigger_issue,
    })
}

/// Recover the `(installation, repo)` a live pod belongs to from its stamped
/// annotations. `None` when any of the three annotations is missing / unparseable
/// (the pod is not one of ours, or is malformed).
fn repo_key_from_pod(pod: &Pod) -> Option<(i64, RepoRef)> {
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

/// Read the trigger-issue number a pod was stamped with (the same
/// [`ANNOTATION_TRIGGER_ISSUE`] the session launcher writes + the reconciler reads).
/// `None` when the annotation is missing / unparseable / zero (the sentinel the
/// live-pod projection uses for "unknown").
fn trigger_issue_from_pod(pod: &Pod) -> Option<u64> {
    let raw = pod
        .metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(ANNOTATION_TRIGGER_ISSUE))?;
    let number = raw.parse::<u64>().ok()?;
    (number != 0).then_some(number)
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
