//! Cross-adapter contract tests.
//!
//! One logical runtime — same session, same repo, same creator, same creation
//! instant — is fed through BOTH adapters, and the normalized rows must agree
//! wherever the two backends expose the same fact. Where they genuinely differ
//! (Kubernetes reports restarts and a deletion instant; OpenSandbox reports
//! neither) the difference is asserted explicitly rather than glossed over, so a
//! future adapter cannot quietly start guessing a value it does not have.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{Pod, PodStatus};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{ObjectMeta, Time};
use serde_json::{json, Value};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::config::PodConfig;
use crate::k8s::session_launcher::{
    ANNOTATION_INSTALLATION, ANNOTATION_LAST_PENDING_AT, ANNOTATION_OWNER, ANNOTATION_REPO,
    ANNOTATION_TRIGGER_ISSUE, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE, SESSION_ID_LABEL,
};
use crate::k8s::KubeClient;
use crate::runtime_identity::{
    stamp_pairs, AttributionSource, RuntimeIdentityMetadata, K8S_IDENTITY_KEYS, OSB_IDENTITY_KEYS,
};
use crate::session_backend::k8s::K8sBackend;
use crate::session_backend::opensandbox::backend::backend_test_support as osb_support;
use crate::session_backend::SessionBackend;

use super::status::RuntimeInventoryStatus;
use super::{RuntimeInventoryItem, RuntimeLifetimePolicy, RuntimeMetadataState};

const SESSION: &str = "11111111-2222-3333-4444-555555555555";
const CREATED_AT: &str = "2026-07-01T09:00:00Z";
/// The same last-pending instant, which the two backends encode differently: a
/// Kubernetes annotation spells it RFC3339, an OpenSandbox metadata value spells
/// it decimal epoch seconds (an RFC3339 string is not a valid label value).
const LAST_PENDING_RFC3339: &str = "2026-07-01T09:30:00Z";

fn parse_utc(rfc3339: &str) -> k8s_openapi::chrono::DateTime<k8s_openapi::chrono::Utc> {
    k8s_openapi::chrono::DateTime::parse_from_rfc3339(rfc3339)
        .expect("rfc3339")
        .with_timezone(&k8s_openapi::chrono::Utc)
}

fn policy() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 7200,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
        max_warnings: 256,
    }
}

fn identity() -> RuntimeIdentityMetadata {
    RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol")
}

/// The logical runtime as a Kubernetes Pod.
fn pod() -> Pod {
    let mut annotations = BTreeMap::from([
        (ANNOTATION_OWNER.to_string(), "acme".to_string()),
        (ANNOTATION_REPO.to_string(), "site".to_string()),
        (ANNOTATION_INSTALLATION.to_string(), "42".to_string()),
        (ANNOTATION_TRIGGER_ISSUE.to_string(), "7".to_string()),
        (
            ANNOTATION_LAST_PENDING_AT.to_string(),
            LAST_PENDING_RFC3339.to_string(),
        ),
    ]);
    for (key, value) in stamp_pairs(&K8S_IDENTITY_KEYS, &identity()) {
        annotations.insert(key.to_string(), value);
    }
    Pod {
        metadata: ObjectMeta {
            name: Some(format!("fkst-sess-{SESSION}")),
            labels: Some(BTreeMap::from([
                (SESSION_ID_LABEL.to_string(), SESSION.to_string()),
                (
                    COMPONENT_LABEL_KEY.to_string(),
                    COMPONENT_LABEL_VALUE.to_string(),
                ),
            ])),
            annotations: Some(annotations),
            creation_timestamp: Some(Time(parse_utc(CREATED_AT))),
            ..Default::default()
        },
        status: Some(PodStatus {
            phase: Some("Running".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The SAME logical runtime as an OpenSandbox list item.
fn sandbox() -> Value {
    let mut metadata = serde_json::Map::from_iter([
        ("fkst-managed".to_string(), json!("true")),
        ("fkst-session-id".to_string(), json!(SESSION)),
        ("fkst-installation-id".to_string(), json!("42")),
        ("fkst-trigger-issue".to_string(), json!("7")),
        ("fkst-owner".to_string(), json!("acme")),
        ("fkst-repo".to_string(), json!("site")),
        (
            "fkst-last-pending-at".to_string(),
            json!(parse_utc(LAST_PENDING_RFC3339).timestamp().to_string()),
        ),
    ]);
    for (key, value) in stamp_pairs(&OSB_IDENTITY_KEYS, &identity()) {
        metadata.insert(key.to_string(), json!(value));
    }
    json!({
        "id": format!("fkst-sess-{SESSION}"),
        "status": { "state": "Running" },
        "metadata": Value::Object(metadata),
        "createdAt": CREATED_AT,
    })
}

async fn kubernetes_item(server: &MockServer) -> RuntimeInventoryItem {
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/chronoai-fkst/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "apiVersion": "v1",
            "kind": "PodList",
            "metadata": { "resourceVersion": "1" },
            "items": [pod()],
        })))
        .mount(server)
        .await;
    let uri: axum::http::Uri = server.uri().parse().expect("uri");
    let client = kube::Client::try_from(kube::Config::new(uri)).expect("kube client");
    let backend = K8sBackend::new(
        KubeClient::new(client, "chronoai-fkst"),
        PodConfig::default(),
        30,
        300,
        2,
    );
    backend
        .list_runtime_inventory(&policy())
        .await
        .expect("k8s snapshot")
        .items
        .pop()
        .expect("one item")
}

async fn opensandbox_item(server: &MockServer) -> RuntimeInventoryItem {
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(osb_support::list_page(json!([sandbox()]))),
        )
        .mount(server)
        .await;
    osb_support::backend(&server.uri(), osb_support::osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("osb snapshot")
        .items
        .pop()
        .expect("one item")
}

#[tokio::test]
async fn equivalent_runtimes_normalize_identically_across_both_adapters() {
    let k8s_server = MockServer::start().await;
    let osb_server = MockServer::start().await;
    let k8s = kubernetes_item(&k8s_server).await;
    let osb = opensandbox_item(&osb_server).await;

    // Identity + correlation: the shared stamp round-trips the same either way.
    assert_eq!(k8s.session_id, osb.session_id);
    assert_eq!(k8s.creator_id, osb.creator_id);
    assert_eq!(k8s.creator_login, osb.creator_login);
    assert_eq!(k8s.trigger_author_id, osb.trigger_author_id);
    assert_eq!(k8s.trigger_author_login, osb.trigger_author_login);
    assert_eq!(k8s.attribution_source, osb.attribution_source);
    assert_eq!(k8s.attribution_source, AttributionSource::LaunchMetadata);
    assert_eq!(k8s.repo_full_name, osb.repo_full_name);
    assert_eq!(k8s.installation_id, osb.installation_id);
    assert_eq!(k8s.trigger_issue, osb.trigger_issue);
    assert_eq!(k8s.metadata_state, osb.metadata_state);
    assert_eq!(k8s.metadata_state, RuntimeMetadataState::Complete);

    // Normalized status and the native spelling both agree here, because both
    // backends happen to call a live runtime "Running".
    assert_eq!(k8s.status, osb.status);
    assert_eq!(k8s.status, RuntimeInventoryStatus::Running);
    assert_eq!(k8s.raw_status, osb.raw_status);

    // Timing: same creation + last-pending instants, same configured policy, so
    // every derived value must match exactly.
    assert_eq!(k8s.created_at, osb.created_at);
    assert_eq!(k8s.last_pending_at, osb.last_pending_at);
    assert_eq!(k8s.max_lifetime_seconds, osb.max_lifetime_seconds);
    assert_eq!(k8s.expires_at, osb.expires_at);
    assert_eq!(k8s.minimum_lifetime_seconds, osb.minimum_lifetime_seconds);
    assert_eq!(k8s.idle_grace_seconds, osb.idle_grace_seconds);
    // Age/remaining/idle are measured against each snapshot's own instant, taken
    // milliseconds apart; assert the RELATIONSHIP rather than exact equality.
    let k8s_age = k8s.age_seconds.expect("k8s age");
    let osb_age = osb.age_seconds.expect("osb age");
    assert!(k8s_age.abs_diff(osb_age) <= 1, "{k8s_age} vs {osb_age}");
    assert_eq!(
        k8s_age - k8s.idle_for_seconds.expect("k8s idle"),
        osb_age - osb.idle_for_seconds.expect("osb idle"),
        "the idle clock must start at the same last-pending instant"
    );
}

#[tokio::test]
async fn the_backends_differ_only_where_they_genuinely_differ() {
    let k8s_server = MockServer::start().await;
    let osb_server = MockServer::start().await;
    let k8s = kubernetes_item(&k8s_server).await;
    let osb = opensandbox_item(&osb_server).await;

    assert_eq!(k8s.backend.as_str(), "kubernetes");
    assert_eq!(osb.backend.as_str(), "opensandbox");

    // Kubernetes reports container restarts; OpenSandbox has no such concept and
    // says `None` rather than guessing zero.
    assert_eq!(k8s.restart_count, Some(0));
    assert_eq!(osb.restart_count, None);

    // Kubernetes has a pending-deletion window; an OpenSandbox delete 404s at
    // once, so there is no instant to report.
    assert_eq!(k8s.deletion_timestamp, None);
    assert_eq!(osb.deletion_timestamp, None);

    // Kubernetes names its object; OpenSandbox only assigns an id.
    assert!(k8s.runtime_name.is_some());
    assert_eq!(osb.runtime_name, None);

    // Both report a bounded, credential-free location — a namespace and a host.
    assert_eq!(k8s.backend_location.as_deref(), Some("chronoai-fkst"));
    let osb_location = osb.backend_location.as_deref().expect("osb location");
    assert!(!osb_location.contains("://"), "{osb_location}");
}

#[tokio::test]
async fn a_terminal_state_normalizes_without_either_backend_claiming_success() {
    // Kubernetes `Succeeded` genuinely means the work exited zero; OpenSandbox
    // `Terminated` means only that the sandbox stopped existing. The normalized
    // enum keeps that distinction instead of collapsing both to "done".
    assert_eq!(
        RuntimeInventoryStatus::from_kubernetes(Some("Succeeded"), false),
        RuntimeInventoryStatus::Succeeded
    );
    assert_eq!(
        RuntimeInventoryStatus::from_opensandbox("Terminated"),
        RuntimeInventoryStatus::Terminated
    );
}
