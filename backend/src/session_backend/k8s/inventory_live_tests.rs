//! The live wiring: exactly ONE namespace-scoped LIST, no per-Pod GET, one clock
//! per snapshot, and explicit failure instead of a fabricated empty fleet.

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::k8s::session_launcher::{
    ANNOTATION_INSTALLATION, COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE,
};
use crate::runtime_identity::RuntimeBackendKind;
use crate::session_backend::inventory::warning::InventoryWarningCode;
use crate::session_backend::inventory::{RuntimeLifetimePolicy, RuntimeMetadataState};
use crate::session_backend::{BackendError, SessionBackend};

use super::inventory_test_fixtures::{backend_against, pod_list_body, policy, sample_pod, ts};

#[tokio::test]
async fn the_inventory_read_costs_exactly_one_list_and_no_per_pod_get() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/api/v1/namespaces/chronoai-fkst/pods"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod_list_body(vec![
            sample_pod(Some("Running"), false),
            sample_pod(Some("Pending"), false),
        ])))
        .expect(1)
        .mount(&server)
        .await;

    let snapshot = backend_against(&server)
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");

    assert_eq!(snapshot.items.len(), 2);
    assert_eq!(snapshot.backend, RuntimeBackendKind::Kubernetes);
    // Every request the mock saw must be the ONE list; a per-pod GET would appear
    // here as a second, differently-pathed request.
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1, "{:?}", requests);
    let query = requests[0].url.query().unwrap_or_default();
    // The selector rides the query percent-encoded (`/` -> %2F, `=` -> %3D);
    // asserting it here keeps the inventory pinned to the SAME managed selector
    // every other fleet read uses.
    let expected = format!(
        "labelSelector={}%3D{COMPONENT_LABEL_VALUE}",
        COMPONENT_LABEL_KEY.replace('/', "%2F")
    );
    assert!(
        query.contains(&expected),
        "list must carry the managed selector {expected}: {query}"
    );
}

#[tokio::test]
async fn every_item_shares_one_observation_instant() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod_list_body(vec![
            sample_pod(Some("Running"), false),
            sample_pod(Some("Running"), false),
        ])))
        .mount(&server)
        .await;

    let snapshot = backend_against(&server)
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    // Both pods share a creation timestamp, so a single clock makes their ages
    // identical; a per-field `now()` would let them drift.
    let ages: Vec<_> = snapshot.items.iter().map(|i| i.age_seconds).collect();
    assert_eq!(ages[0], ages[1]);
    assert!(snapshot.observed_at >= ts("2026-07-01T09:00:00Z"));
}

#[tokio::test]
async fn an_oversized_fleet_fails_explicitly_rather_than_returning_a_short_list() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod_list_body(vec![
            sample_pod(Some("Running"), false),
            sample_pod(Some("Running"), false),
            sample_pod(Some("Running"), false),
        ])))
        .mount(&server)
        .await;

    let tight = RuntimeLifetimePolicy {
        max_items: 2,
        ..policy()
    };
    match backend_against(&server)
        .list_runtime_inventory(&tight)
        .await
    {
        Err(BackendError::InventoryTooLarge { limit }) => assert_eq!(limit, 2),
        other => panic!("expected an explicit ceiling error, got {other:?}"),
    }
}

#[tokio::test]
async fn a_list_failure_propagates_instead_of_reporting_an_empty_fleet() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let error = backend_against(&server)
        .list_runtime_inventory(&policy())
        .await
        .expect_err("must not fabricate an empty snapshot");
    assert!(matches!(error, BackendError::Other(_)), "{error:?}");
}

#[tokio::test]
async fn a_malformed_pod_is_listed_with_warnings_not_dropped() {
    let server = MockServer::start().await;
    let mut orphan = sample_pod(Some("Running"), false);
    orphan.metadata.name = Some("fkst-sess-orphan".to_string());
    orphan.metadata.labels = None;
    orphan
        .metadata
        .annotations
        .as_mut()
        .expect("annotations")
        .insert(
            ANNOTATION_INSTALLATION.to_string(),
            "not-a-number".to_string(),
        );
    Mock::given(method("GET"))
        .respond_with(ResponseTemplate::new(200).set_body_json(pod_list_body(vec![orphan])))
        .mount(&server)
        .await;

    let snapshot = backend_against(&server)
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(snapshot.items.len(), 1);
    let item = &snapshot.items[0];
    assert_eq!(item.session_id, None);
    assert_eq!(item.installation_id, None);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Malformed);
    let codes: Vec<_> = snapshot.warnings.iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&InventoryWarningCode::MissingSessionId),
        "{codes:?}"
    );
    assert!(
        codes.contains(&InventoryWarningCode::MalformedCorrelation),
        "{codes:?}"
    );
}
