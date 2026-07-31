//! The OpenSandbox inventory's bounds, secrecy, and failure modes: redacted
//! operational text, secret canaries, the item and warning ceilings, the two ways
//! a page can fail to become a fleet, and the read-only guarantee.
//!
//! The projection half lives in `inventory_tests`.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::session_backend::inventory::warning::InventoryWarningCode;
use crate::session_backend::inventory::RuntimeLifetimePolicy;
use crate::session_backend::{BackendError, SessionBackend};

use super::super::backend_test_support::{backend, osb_config, SESSION_ID};
use super::inventory_test_fixtures::{
    mount_single_page, orphan, policy, sandbox, stamped_metadata,
};

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

/// A secret canary planted in every part of a sandbox response the projection is
/// NOT allowed to read must be absent from the snapshot. See the Kubernetes twin
/// for why this is asserted despite the projection being structurally safe.
#[tokio::test]
async fn a_secret_planted_outside_the_projected_fields_never_reaches_the_snapshot() {
    const CANARY: &str = "ghp_canary000111222333444555666777888999";
    let server = MockServer::start().await;
    let mut metadata = stamped_metadata(SESSION_ID);
    let object = metadata.as_object_mut().expect("object");
    object.insert("customer-token".to_string(), json!(CANARY));
    object.insert("fkst-unknown-future-key".to_string(), json!(CANARY));
    mount_single_page(
        &server,
        json!([{
            "id": "sbx-1",
            "status": { "state": "Running" },
            "metadata": metadata,
            "createdAt": "2026-07-01T09:00:00Z",
            // Everything below is present on real list responses and read by
            // nothing in the projection.
            "extensions": { "envSecret": CANARY },
            "image": { "uri": format!("registry.example.com/fkst@sha256:{CANARY}") },
            "entrypoint": [format!("--token={CANARY}")],
            "env": { "FKST_GITHUB_TOKEN": CANARY },
        }]),
    )
    .await;

    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect("snapshot");
    assert_eq!(snapshot.items.len(), 1);
    let rendered = format!("{snapshot:?}");
    assert!(!rendered.contains(CANARY), "{rendered}");
    assert!(!rendered.contains("FKST_GITHUB_TOKEN"), "{rendered}");
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
async fn an_undecodable_item_fails_the_page_instead_of_yielding_a_plausible_fleet() {
    // The "otherwise" half of the item-level-recovery rule. `createdAt` and an
    // unknown `state` recover per item (proved in `inventory_tests`); `id` and
    // `status` do NOT — they are required with no serde default, so an item
    // missing one fails the whole page decode. That must surface as an explicit
    // error: a page silently reduced to its decodable items would read as a
    // smaller but COMPLETE fleet, which is the one outcome the spec forbids. This
    // test is the guard against a future `#[serde(default)]` making that possible.
    let server = MockServer::start().await;
    let corrupt = json!({
        "id": "sbx-corrupt",
        "metadata": stamped_metadata(SESSION_ID),
        "createdAt": "2026-07-01T09:00:00Z",
    });
    mount_single_page(&server, json!([sandbox("sbx-1", "Running"), corrupt])).await;

    let error = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&policy())
        .await
        .expect_err("a corrupt page must not decode to a plausible partial fleet");
    assert!(matches!(error, BackendError::Other(_)), "{error:?}");
}

#[tokio::test]
async fn the_warning_ceiling_is_operator_configured_and_announces_its_overflow() {
    // Every sandbox below is an orphan, so each contributes a warning; a ceiling
    // of 3 leaves room for two of them plus the truncation marker.
    let server = MockServer::start().await;
    mount_single_page(
        &server,
        json!([orphan("sbx-1"), orphan("sbx-2"), orphan("sbx-3")]),
    )
    .await;

    let clipped = RuntimeLifetimePolicy {
        max_warnings: 3,
        ..policy()
    };
    let snapshot = backend(&server.uri(), osb_config())
        .list_runtime_inventory(&clipped)
        .await
        .expect("snapshot");
    // Items are never clipped by the warning ceiling — only the diagnostics are.
    assert_eq!(snapshot.items.len(), 3);
    assert_eq!(snapshot.warnings.len(), 3);
    assert_eq!(
        snapshot.warnings.last().map(|w| w.code),
        Some(InventoryWarningCode::WarningsTruncated),
        "a clipped warning list must say so as its last entry"
    );
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
