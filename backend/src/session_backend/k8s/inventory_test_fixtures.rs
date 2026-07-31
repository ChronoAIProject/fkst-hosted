//! Shared fixtures for the Kubernetes inventory tests: the sample substrate
//! session Pod, the rendering policy, and a backend wired to a mock apiserver.
//! Kept separate so the pure-projection and live-wiring suites each stay small.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{ContainerState, ContainerStatus, Pod, PodStatus};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use k8s_openapi::chrono::{DateTime, Utc};
use wiremock::MockServer;

use crate::config::PodConfig;
use crate::k8s::session_launcher::{
    ANNOTATION_INSTALLATION, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER, ANNOTATION_REPO,
    ANNOTATION_TRIGGER_ISSUE, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE, SESSION_ID_LABEL,
};
use crate::k8s::KubeClient;
use crate::runtime_identity::{stamp_pairs, RuntimeIdentityMetadata, K8S_IDENTITY_KEYS};
use crate::session_backend::inventory::RuntimeLifetimePolicy;
use crate::session_backend::k8s::K8sBackend;

pub(super) fn ts(rfc3339: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(rfc3339)
        .expect("rfc3339")
        .with_timezone(&Utc)
}

/// Unlimited maximum lifetime, the shipped shield/grace defaults.
pub(super) fn policy() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 0,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
        max_warnings: 256,
    }
}

/// A fully stamped substrate-session pod for `acme/site`, session `sess-1`.
pub(super) fn sample_pod(phase: Option<&str>, terminating: bool) -> Pod {
    let mut annotations = BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_INSTALLATION.to_string(), "900".to_string()),
        (ANNOTATION_TRIGGER_ISSUE.to_string(), "7".to_string()),
        (
            ANNOTATION_LAST_PENDING_AT.to_string(),
            "2026-07-01T11:30:00+00:00".to_string(),
        ),
    ]);
    let identity = RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol");
    for (key, value) in stamp_pairs(&K8S_IDENTITY_KEYS, &identity) {
        annotations.insert(key.to_string(), value);
    }
    Pod {
        metadata: ObjectMeta {
            name: Some("fkst-sess-sess-1".to_string()),
            uid: Some("uid-1".to_string()),
            // Both labels a real substrate-session Pod carries: the session id
            // the reconciler correlates on, and the component marker the LIST
            // selector matches (and the inventory reads back as `managed`).
            labels: Some(BTreeMap::from([
                (SESSION_ID_LABEL.to_string(), "sess-1".to_string()),
                (
                    COMPONENT_LABEL_KEY.to_string(),
                    COMPONENT_LABEL_VALUE.to_string(),
                ),
            ])),
            annotations: Some(annotations),
            creation_timestamp: Some(Time(ts("2026-07-01T09:00:00Z"))),
            deletion_timestamp: terminating.then(|| Time(ts("2026-07-01T11:45:00Z"))),
            ..Default::default()
        },
        status: Some(PodStatus {
            phase: phase.map(str::to_string),
            ..Default::default()
        }),
        ..Default::default()
    }
}

pub(super) fn container(
    name: &str,
    restarts: i32,
    state: Option<ContainerState>,
) -> ContainerStatus {
    ContainerStatus {
        name: name.to_string(),
        restart_count: restarts,
        state,
        ..Default::default()
    }
}

/// A backend whose kube client talks to `server`.
pub(super) fn backend_against(server: &MockServer) -> K8sBackend {
    let uri: axum::http::Uri = server.uri().parse().expect("mock uri");
    let client = kube::Client::try_from(kube::Config::new(uri)).expect("kube client");
    K8sBackend::new(
        KubeClient::new(client, "chronoai-fkst"),
        PodConfig::default(),
        30,
        300,
        2,
    )
}

/// The `PodList` body a mocked apiserver answers a LIST with.
pub(super) fn pod_list_body(pods: Vec<Pod>) -> serde_json::Value {
    serde_json::json!({
        "apiVersion": "v1",
        "kind": "PodList",
        "metadata": { "resourceVersion": "1" },
        "items": pods,
    })
}
