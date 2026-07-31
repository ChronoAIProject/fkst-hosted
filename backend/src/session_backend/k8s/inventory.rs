//! The one-pass Kubernetes live inventory verb (issue #5674).
//!
//! Exactly ONE namespace-scoped Pod LIST — the same
//! `app.kubernetes.io/component=substrate-session` selector every other fleet
//! read uses — followed by pure in-memory projection. Deliberately NOT reused
//! here: [`crate::session_backend::k8s::K8sBackend::status_summary`] (a GET per
//! pod) and any per-pod refetch. A dashboard polling every five seconds must cost
//! the apiserver one LIST, not one LIST plus N GETs.
//!
//! The shared primitives ARE reused — the annotation accessor, the annotation/
//! label key constants, and [`crate::runtime_identity::read`] — but the sibling
//! `lifecycle::pod_to_live` projection is not, and must not be: it defaults an
//! absent `creationTimestamp` to `now` so a malformed pod is shielded from
//! idle-kill, which is exactly the substitution an operations view is forbidden
//! to make. Reconciliation semantics are untouched by anything here.
//!
//! Everything below is a pure function of a listed `Pod`, so the whole projection
//! is unit-testable off fixtures and the network path has nothing left to get
//! wrong beyond the LIST itself.

use k8s_openapi::api::core::v1::{ContainerStatus, Pod, PodCondition};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::Time;
use k8s_openapi::chrono::{DateTime, Utc};

use crate::k8s::session_launcher::{
    ANNOTATION_INSTALLATION, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER, ANNOTATION_REPO,
    ANNOTATION_TRIGGER_ISSUE, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE, SESSION_ID_LABEL,
};
use crate::runtime_identity::{RuntimeBackendKind, K8S_IDENTITY_KEYS};
use crate::session_backend::inventory::build::{build_item, RawRuntimeFacts};
use crate::session_backend::inventory::status::RuntimeInventoryStatus;
use crate::session_backend::inventory::warning::WarningSink;
use crate::session_backend::inventory::{RuntimeInventorySnapshot, RuntimeLifetimePolicy};

use super::super::BackendError;
use super::{annotation, K8sBackend};

/// The placeholder a Pod with neither a name nor a uid is listed under. Such an
/// object cannot exist through the apiserver; the fallback exists so a
/// pathological response still yields a visible row instead of a dropped one.
const UNNAMED_RUNTIME_ID: &str = "<unnamed>";

impl K8sBackend {
    pub(super) async fn list_runtime_inventory_impl(
        &self,
        policy: &RuntimeLifetimePolicy,
    ) -> Result<RuntimeInventorySnapshot, BackendError> {
        let pods = self.list_component_pods().await?;
        // The ceiling is checked BEFORE any projection: the point is to bound the
        // work, and an oversized fleet must fail loudly rather than be clipped.
        if pods.len() > policy.max_items {
            tracing::error!(
                listed = pods.len(),
                limit = policy.max_items,
                "kubernetes runtime inventory: fleet exceeds the configured ceiling; refusing to \
                 return a partial snapshot"
            );
            return Err(BackendError::InventoryTooLarge {
                limit: policy.max_items,
            });
        }

        // ONE clock for the whole snapshot, taken after the list so no item can
        // report a negative age purely because the list took a moment.
        let observed_at = Utc::now();
        let mut warnings = WarningSink::new(policy.max_warnings);
        let namespace = self.kube.namespace().to_string();
        let items = pods
            .iter()
            .map(|pod| {
                build_item(
                    facts_from_pod(pod, &namespace),
                    RuntimeBackendKind::Kubernetes,
                    observed_at,
                    policy,
                    &mut warnings,
                )
            })
            .collect();

        Ok(RuntimeInventorySnapshot {
            observed_at,
            backend: RuntimeBackendKind::Kubernetes,
            items,
            warnings: warnings.into_warnings(),
        })
    }
}

/// Project one listed Pod into the backend-neutral raw facts.
///
/// Never returns `None`: a Pod that matched the managed selector is one of ours by
/// definition, and dropping it because a stamp is missing would hide exactly the
/// orphan an operator is looking for.
fn facts_from_pod(pod: &Pod, namespace: &str) -> RawRuntimeFacts {
    let terminating = pod.metadata.deletion_timestamp.is_some();
    let phase = pod.status.as_ref().and_then(|s| s.phase.as_deref());
    let last_pending_raw = annotation(pod, ANNOTATION_LAST_PENDING_AT);
    let last_pending_at = last_pending_raw
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|dt| dt.with_timezone(&Utc));
    let identity = pod
        .metadata
        .annotations
        .as_ref()
        .map(|annotations| crate::runtime_identity::read(&K8S_IDENTITY_KEYS, annotations))
        .unwrap_or_default();
    let (status_reason, status_message) = operational_detail(pod);

    RawRuntimeFacts {
        runtime_id: pod
            .metadata
            .name
            .clone()
            .or_else(|| pod.metadata.uid.clone())
            .unwrap_or_else(|| UNNAMED_RUNTIME_ID.to_string()),
        runtime_name: pod.metadata.name.clone(),
        runtime_uid: pod.metadata.uid.clone(),
        // The namespace the client is BOUND to, not the one the object claims:
        // a bounded, credential-free location that cannot be influenced by a
        // response body.
        backend_location: Some(namespace.to_string()),

        session_id: pod
            .metadata
            .labels
            .as_ref()
            .and_then(|l| l.get(SESSION_ID_LABEL))
            .cloned(),
        // Read back from the object rather than assumed from the selector that
        // fetched it. The two agree on every response a healthy apiserver can
        // return — which is exactly why reading it is free — but assuming it
        // would make the field incapable of ever reporting the drift it exists
        // for, and would leave the Kubernetes adapter the only one that cannot.
        managed: pod
            .metadata
            .labels
            .as_ref()
            .and_then(|labels| labels.get(COMPONENT_LABEL_KEY))
            .map(String::as_str)
            == Some(COMPONENT_LABEL_VALUE),
        identity,

        owner: annotation(pod, ANNOTATION_OWNER).map(str::to_string),
        repo: annotation(pod, ANNOTATION_REPO).map(str::to_string),
        installation_id_raw: annotation(pod, ANNOTATION_INSTALLATION).map(str::to_string),
        trigger_issue_raw: annotation(pod, ANNOTATION_TRIGGER_ISSUE).map(str::to_string),

        status: RuntimeInventoryStatus::from_kubernetes(phase, terminating),
        raw_status: phase.unwrap_or_default().to_string(),
        status_reason,
        status_message,

        // `creationTimestamp` is a typed `Time` the apiserver either sets or
        // omits — it cannot arrive unparseable, so there is no malformed case
        // here (unlike OpenSandbox's RFC3339 string).
        created_at: pod.metadata.creation_timestamp.as_ref().map(|Time(t)| *t),
        created_at_malformed: false,
        last_pending_at,
        // A present-but-unparseable annotation is malformed; an absent one simply
        // means the session has never reported pending.
        last_pending_malformed: last_pending_raw.is_some() && last_pending_at.is_none(),

        // Only when the object HAS a status. A Pod the kubelet has not reported
        // on yet knows nothing about its containers, and `Some(0)` there would
        // assert "never restarted" on no evidence — the same zero-as-guess the
        // OpenSandbox adapter is forbidden to make. A present status with no
        // container statuses is a genuine report of zero restarts.
        restart_count: pod.status.as_ref().map(|_| total_restarts(pod)),
        last_transition_at: latest_transition(pod),
        deletion_timestamp: pod.metadata.deletion_timestamp.as_ref().map(|Time(t)| *t),
    }
}

/// The operational `(reason, message)` pair, most specific first.
///
/// Container-level detail wins over the pod-level summary because it is what
/// actually explains a stuck session (`ImagePullBackOff` on one container beats
/// the pod's generic `Pending`). Only reason/message strings are read — never env,
/// never image-pull auth, never the serialized object. Both are bounded and
/// redacted downstream in [`crate::session_backend::inventory::text`].
fn operational_detail(pod: &Pod) -> (Option<String>, Option<String>) {
    for status in all_container_statuses(pod) {
        let Some(state) = status.state.as_ref() else {
            continue;
        };
        if let Some(waiting) = state.waiting.as_ref() {
            if waiting.reason.is_some() || waiting.message.is_some() {
                return (waiting.reason.clone(), waiting.message.clone());
            }
        }
        if let Some(terminated) = state.terminated.as_ref() {
            if terminated.reason.is_some() || terminated.message.is_some() {
                return (terminated.reason.clone(), terminated.message.clone());
            }
        }
    }
    let status = pod.status.as_ref();
    (
        status.and_then(|s| s.reason.clone()),
        status.and_then(|s| s.message.clone()),
    )
}

/// Every container status on the pod, app containers before init containers (an
/// app-container failure is the more interesting one once init has completed).
fn all_container_statuses(pod: &Pod) -> impl Iterator<Item = &ContainerStatus> {
    let status = pod.status.as_ref();
    let containers = status.and_then(|s| s.container_statuses.as_ref());
    let init = status.and_then(|s| s.init_container_statuses.as_ref());
    containers
        .into_iter()
        .flatten()
        .chain(init.into_iter().flatten())
}

/// Sum every container and init-container restart count.
///
/// The wire type is a signed `i32`; a negative value is impossible from a real
/// kubelet, so it is DISCARDED rather than allowed to reduce the total — a
/// hostile/corrupt response must not be able to hide restarts. The sum saturates
/// at `u32::MAX`, which no real pod approaches.
fn total_restarts(pod: &Pod) -> u32 {
    all_container_statuses(pod)
        .filter_map(|status| u32::try_from(status.restart_count).ok())
        .fold(0u32, |total, count| total.saturating_add(count))
}

/// The most recent state transition the LISTED object already reveals.
///
/// Documented choice: the maximum over (a) every pod condition's
/// `lastTransitionTime` and (b) every container state's own instant — a running
/// container's `startedAt`, a terminated container's `finishedAt` (falling back to
/// its `startedAt`), and a waiting container's… nothing, since the API reports no
/// instant for waiting. All of it is already in the LIST response, so this never
/// costs an extra GET. `None` when the object reveals no transition at all.
fn latest_transition(pod: &Pod) -> Option<DateTime<Utc>> {
    let status = pod.status.as_ref()?;
    let from_conditions = status
        .conditions
        .iter()
        .flatten()
        .filter_map(condition_instant);
    let from_containers = all_container_statuses(pod).filter_map(container_instant);
    from_conditions.chain(from_containers).max()
}

fn condition_instant(condition: &PodCondition) -> Option<DateTime<Utc>> {
    condition
        .last_transition_time
        .as_ref()
        .map(|Time(t)| *t)
        .or_else(|| condition.last_probe_time.as_ref().map(|Time(t)| *t))
}

fn container_instant(status: &ContainerStatus) -> Option<DateTime<Utc>> {
    let state = status.state.as_ref()?;
    if let Some(terminated) = state.terminated.as_ref() {
        return terminated
            .finished_at
            .as_ref()
            .or(terminated.started_at.as_ref())
            .map(|Time(t)| *t);
    }
    state
        .running
        .as_ref()
        .and_then(|running| running.started_at.as_ref())
        .map(|Time(t)| *t)
}

#[cfg(test)]
#[path = "inventory_live_tests.rs"]
mod inventory_live_tests;
#[cfg(test)]
#[path = "inventory_test_fixtures.rs"]
mod inventory_test_fixtures;
#[cfg(test)]
#[path = "inventory_tests.rs"]
mod inventory_tests;
