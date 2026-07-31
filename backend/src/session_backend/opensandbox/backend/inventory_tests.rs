//! The OpenSandbox inventory: every documented state plus an unknown one, the
//! paginated walk, and the proof that no per-sandbox GET is ever issued.

use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::runtime_identity::{
    stamp_pairs, AttributionSource, RuntimeIdentityMetadata, OSB_IDENTITY_KEYS,
};
use crate::session_backend::inventory::warning::InventoryWarningCode;
use crate::session_backend::inventory::{RuntimeLifetimePolicy, RuntimeMetadataState};
use crate::session_backend::{BackendError, SessionBackend};

use super::super::backend_test_support::{
    backend, correlation_metadata, list_page, osb_config, sandbox_json, SESSION_ID,
};
use super::*;

const WORK_LABEL_HEX: &str = "666b73742d776f726b";

fn policy() -> RuntimeLifetimePolicy {
    RuntimeLifetimePolicy {
        max_lifetime_seconds: 0,
        minimum_lifetime_seconds: 120,
        idle_grace_seconds: 300,
        max_items: 5000,
    }
}

/// The `metadata` filter value the fleet-wide inventory walk sends.
fn managed_filter() -> String {
    "fkst-managed=true".to_string()
}

/// Correlation metadata plus a complete launch attribution stamp.
fn stamped_metadata(session: &str) -> Value {
    let mut metadata = correlation_metadata(session, "acme", "site", WORK_LABEL_HEX);
    let identity = RuntimeIdentityMetadata::new(Some(11), "alice", 22, "carol");
    let object = metadata.as_object_mut().expect("object");
    for (key, value) in stamp_pairs(&OSB_IDENTITY_KEYS, &identity) {
        object.insert(key.to_string(), Value::String(value));
    }
    metadata
}

fn sandbox(id: &str, state: &str) -> Value {
    sandbox_json(
        id,
        state,
        "2026-07-01T09:00:00Z",
        stamped_metadata(SESSION_ID),
    )
}

/// Mount a single-page managed list returning `items`, asserting the walk asks for
/// exactly one page.
async fn mount_single_page(server: &MockServer, items: Value) {
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("metadata", managed_filter()))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(items)))
        .expect(1)
        .mount(server)
        .await;
}

#[tokio::test]
async fn a_stamped_sandbox_projects_every_correlation_and_attribution_fact() {
    let server = MockServer::start().await;
    mount_single_page(&server, json!([sandbox("sbx-1", "Running")])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");

    assert_eq!(snapshot.backend, RuntimeBackendKind::OpenSandbox);
    assert_eq!(snapshot.items.len(), 1);
    let item = &snapshot.items[0];
    assert_eq!(item.runtime_id, "sbx-1");
    // OpenSandbox assigns an id and nothing else; inventing a name or uid would be
    // structure the backend does not have.
    assert_eq!(item.runtime_name, None);
    assert_eq!(item.runtime_uid, None);
    assert_eq!(item.session_id.as_deref(), Some(SESSION_ID));
    assert!(item.managed);
    assert_eq!(item.status, RuntimeInventoryStatus::Running);
    assert_eq!(item.raw_status, "Running");
    assert_eq!(item.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(item.installation_id, Some(42));
    assert_eq!(item.trigger_issue, Some(7));
    assert_eq!(item.creator_id, Some(11));
    assert_eq!(item.creator_login.as_deref(), Some("alice"));
    assert_eq!(item.trigger_author_id, Some(22));
    assert_eq!(item.attribution_source, AttributionSource::LaunchMetadata);
    assert_eq!(item.metadata_state, RuntimeMetadataState::Complete);
    // The lifecycle API exposes no restart count and no pending-deletion instant.
    assert_eq!(item.restart_count, None);
    assert_eq!(item.deletion_timestamp, None);
    // The backend location is the bounded host label, never the base URL.
    let location = item.backend_location.as_deref().expect("location");
    assert!(!location.contains("http"), "{location}");
    assert!(location.starts_with("127.0.0.1:"), "{location}");
}

#[tokio::test]
async fn the_inventory_read_issues_no_per_sandbox_get() {
    let server = MockServer::start().await;
    mount_single_page(
        &server,
        json!([sandbox("sbx-1", "Running"), sandbox("sbx-2", "Paused")]),
    )
    .await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(snapshot.items.len(), 2);

    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 1, "{requests:?}");
    // A per-sandbox GET would carry the id in the path; the list never does.
    for request in &requests {
        assert_eq!(request.url.path(), "/v1/sandboxes", "{request:?}");
    }
}

#[tokio::test]
async fn every_documented_state_maps_and_an_unknown_one_stays_visible() {
    let cases = [
        ("Pending", RuntimeInventoryStatus::Pending),
        ("Running", RuntimeInventoryStatus::Running),
        ("Paused", RuntimeInventoryStatus::Paused),
        ("Pausing", RuntimeInventoryStatus::Transitioning),
        ("Resuming", RuntimeInventoryStatus::Transitioning),
        ("Stopping", RuntimeInventoryStatus::Terminating),
        ("Terminated", RuntimeInventoryStatus::Terminated),
        ("Failed", RuntimeInventoryStatus::Failed),
        ("Hibernating", RuntimeInventoryStatus::Unknown),
    ];
    for (state, expected) in cases {
        let server = MockServer::start().await;
        mount_single_page(&server, json!([sandbox("sbx-1", state)])).await;
        let snapshot = backend(&server.uri(), osb_config())
            .list_runtime_inventory(&policy())
            .await
            .expect("snapshot");
        let item = &snapshot.items[0];
        assert_eq!(item.status, expected, "state {state}");
        // The native spelling survives independently, including the unmapped one.
        assert_eq!(item.raw_status, state, "state {state}");
    }
}

#[tokio::test]
async fn an_unknown_state_is_warned_but_never_breaks_the_snapshot() {
    let server = MockServer::start().await;
    mount_single_page(&server, json!([sandbox("sbx-1", "Hibernating")])).await;
    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(snapshot.items.len(), 1);
    assert!(snapshot
        .warnings
        .iter()
        .any(|w| w.code == InventoryWarningCode::UnknownStatus));
}

#[tokio::test]
async fn the_walk_aggregates_pages_as_one_logical_operation() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("page", "1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "items": [sandbox("sbx-1", "Running")],
            "pagination": { "page": 1, "pageSize": 100, "totalItems": 2,
                            "totalPages": 2, "hasNextPage": true }
        })))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("page", "2"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(list_page(json!([sandbox("sbx-2", "Running")]))),
        )
        .expect(1)
        .mount(&server)
        .await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(snapshot.items.len(), 2);
    // Two page requests, zero per-sandbox GETs.
    let requests = server.received_requests().await.expect("requests");
    assert_eq!(requests.len(), 2);
    for request in &requests {
        assert_eq!(request.url.path(), "/v1/sandboxes");
    }
}

#[tokio::test]
async fn a_missing_created_at_is_null_and_a_malformed_one_is_flagged() {
    let server = MockServer::start().await;
    let missing = json!({
        "id": "sbx-missing",
        "status": { "state": "Running" },
        "metadata": stamped_metadata(SESSION_ID),
    });
    let malformed = json!({
        "id": "sbx-bad",
        "status": { "state": "Running" },
        "metadata": stamped_metadata(SESSION_ID),
        "createdAt": "the day before yesterday",
    });
    mount_single_page(&server, json!([missing, malformed])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    let by_id = |id: &str| {
        snapshot
            .items
            .iter()
            .find(|item| item.runtime_id == id)
            .expect("item")
            .clone()
    };
    // Neither is defaulted to `now`: that would make an old sandbox look new.
    assert_eq!(by_id("sbx-missing").created_at, None);
    assert_eq!(by_id("sbx-missing").age_seconds, None);
    assert_eq!(by_id("sbx-bad").created_at, None);
    assert_eq!(
        by_id("sbx-bad").metadata_state,
        RuntimeMetadataState::Malformed
    );
    let codes: Vec<_> = snapshot.warnings.iter().map(|w| w.code).collect();
    assert!(
        codes.contains(&InventoryWarningCode::MissingCreatedAt),
        "{codes:?}"
    );
    assert!(
        codes.contains(&InventoryWarningCode::MalformedCreatedAt),
        "{codes:?}"
    );
}

#[tokio::test]
async fn reason_and_message_are_bounded_and_secret_safe() {
    let server = MockServer::start().await;
    let noisy = json!({
        "id": "sbx-1",
        "status": {
            "state": "Failed",
            "reason": "PullFailed\nRetrying",
            "message": format!(
                "auth to https://bob:hunter2@registry.example.com/img?token=ghp_{} failed {}",
                "a".repeat(36),
                "x".repeat(2000),
            ),
        },
        "metadata": stamped_metadata(SESSION_ID),
        "createdAt": "2026-07-01T09:00:00Z",
    });
    mount_single_page(&server, json!([noisy])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    let item = &snapshot.items[0];
    assert_eq!(item.status_reason.as_deref(), Some("PullFailed Retrying"));
    let message = item.status_message.as_deref().expect("message");
    assert!(message.len() <= 512, "{}", message.len());
    assert!(!message.contains("hunter2"), "{message}");
    assert!(!message.contains("token="), "{message}");
    assert!(!message.contains("ghp_aaaa"), "{message}");
}

#[tokio::test]
async fn the_last_transition_instant_is_read_from_the_list_response() {
    let server = MockServer::start().await;
    let with_transition = json!({
        "id": "sbx-1",
        "status": { "state": "Running", "lastTransitionAt": "2026-07-01T10:15:00Z" },
        "metadata": stamped_metadata(SESSION_ID),
        "createdAt": "2026-07-01T09:00:00Z",
    });
    mount_single_page(&server, json!([with_transition])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(
        snapshot.items[0].last_transition_at,
        Some(
            DateTime::parse_from_rfc3339("2026-07-01T10:15:00Z")
                .expect("rfc3339")
                .with_timezone(&Utc)
        )
    );
}

#[tokio::test]
async fn an_orphan_or_partly_stamped_sandbox_is_retained() {
    let server = MockServer::start().await;
    let orphan = json!({
        "id": "sbx-orphan",
        "status": { "state": "Running" },
        "metadata": { "fkst-managed": "true" },
        "createdAt": "2026-07-01T09:00:00Z",
    });
    let malformed = json!({
        "id": "sbx-bad-install",
        "status": { "state": "Running" },
        "metadata": {
            "fkst-managed": "true",
            "fkst-session-id": SESSION_ID,
            "fkst-installation-id": "forty-two",
        },
        "createdAt": "2026-07-01T09:00:00Z",
    });
    mount_single_page(&server, json!([orphan, malformed])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(
        snapshot.items.len(),
        2,
        "a managed sandbox is never dropped"
    );
    let orphan_item = snapshot
        .items
        .iter()
        .find(|item| item.runtime_id == "sbx-orphan")
        .expect("orphan");
    assert_eq!(orphan_item.session_id, None);
    assert_eq!(
        orphan_item.attribution_source,
        AttributionSource::UnknownLegacy
    );
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

#[tokio::test]
async fn a_drifted_managed_marker_is_reported_rather_than_assumed() {
    let server = MockServer::start().await;
    let drifted = json!({
        "id": "sbx-drift",
        "status": { "state": "Running" },
        "metadata": { "fkst-session-id": SESSION_ID },
        "createdAt": "2026-07-01T09:00:00Z",
    });
    mount_single_page(&server, json!([drifted])).await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert!(!snapshot.items[0].managed);
}

#[tokio::test]
async fn an_oversized_fleet_fails_explicitly_rather_than_returning_a_short_list() {
    let server = MockServer::start().await;
    mount_single_page(
        &server,
        json!([
            sandbox("sbx-1", "Running"),
            sandbox("sbx-2", "Running"),
            sandbox("sbx-3", "Running")
        ]),
    )
    .await;

    let tight = RuntimeLifetimePolicy {
        max_items: 2,
        ..policy()
    };
    match backend(&server.uri(), osb_config())
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
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(503).set_body_string("unavailable"))
        .mount(&server)
        .await;

    let error = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect_err("must not fabricate an empty snapshot");
    assert!(matches!(error, BackendError::Other(_)), "{error:?}");
}

#[tokio::test]
async fn the_read_never_writes_metadata() {
    let server = MockServer::start().await;
    mount_single_page(&server, json!([sandbox("sbx-1", "Running")])).await;

    backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");

    // Inventory is read-only: an operator opening a dashboard must not be able to
    // refresh last-pending and thereby extend a session's life.
    let requests = server.received_requests().await.expect("requests");
    for request in &requests {
        assert_eq!(request.method.as_str(), "GET", "{request:?}");
    }
}

#[test]
fn the_server_label_carries_no_credential_material() {
    let client = crate::session_backend::opensandbox::OsbLifecycleClient::new(
        reqwest::Url::parse("https://user:pw@osb.internal:8443/base/").expect("url"),
        secrecy::SecretString::from("key".to_string()),
        reqwest::Client::new(),
    );
    assert_eq!(client.server_label(), Some("osb.internal:8443"));
}
