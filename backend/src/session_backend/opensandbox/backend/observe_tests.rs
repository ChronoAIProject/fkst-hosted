//! Wiremock tests for the observe-side verbs: `observe_repo` (state mapping through
//! the real path, the owner/repo metadata filter, and the respawn-shield injection +
//! expiry) and `mark_pending` (touches ONLY the last-pending key; empty resolve is
//! `NotFound`).

use std::time::Duration;

use serde_json::{json, Value};
use wiremock::matchers::{header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::models::RepoRef;
use crate::reconcile::desired::PodLiveness;
use crate::session_backend::BackendError;

use super::super::backend_test_support::{
    backend, correlation_metadata, list_page, osb_config, sandbox_json, API_KEY, SESSION_ID,
};

/// Hex of the `fkst-work` work label (round-tripped by `to_live_pod`).
const WORK_LABEL_HEX: &str = "666b73742d776f726b";

fn acme_site() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// An `acme/site` sandbox for `session` in `state`.
fn sbx(id: &str, state: &str, session: &str) -> Value {
    sandbox_json(
        id,
        state,
        "2026-07-09T00:00:00Z",
        correlation_metadata(session, "acme", "site", WORK_LABEL_HEX),
    )
}

#[tokio::test]
async fn observe_repo_maps_states_and_applies_the_owner_repo_filter() {
    let server = MockServer::start().await;
    let items = json!([
        sbx("s-run", "Running", "sess-run"),
        sbx("s-pend", "Pending", "sess-pend"),
        sbx("s-fail", "Failed", "sess-fail"),
    ]);
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        // The filter pins managed + owner + repo (proving repo-scoping).
        .and(query_param(
            "metadata",
            "fkst-managed=true&fkst-owner=acme&fkst-repo=site",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(items)))
        .mount(&server)
        .await;

    let pods = backend(&server.uri(), osb_config())
        .observe_repo_impl(&acme_site())
        .await
        .expect("observed");
    assert_eq!(pods.len(), 3);
    let by = |sid: &str| {
        pods.iter()
            .find(|p| p.session_id == sid)
            .unwrap_or_else(|| panic!("pod {sid}"))
    };
    assert_eq!(by("sess-run").liveness, PodLiveness::Live);
    assert_eq!(by("sess-pend").liveness, PodLiveness::Starting);
    assert_eq!(by("sess-fail").liveness, PodLiveness::Terminal);
    // The decoded work label + reassembled config hash ride through the projection.
    assert_eq!(by("sess-run").work_label.as_deref(), Some("fkst-work"));
    assert_eq!(
        by("sess-run").config_hash.as_deref(),
        Some("a".repeat(64).as_str())
    );
}

#[tokio::test]
async fn observe_repo_injects_a_synthetic_terminating_pod_for_a_just_stopped_session() {
    let server = MockServer::start().await;
    // The repo currently has NO live sandbox (the just-stopped one already 404s).
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let backend = backend(&server.uri(), osb_config());
    backend.record_shield("sess-x", acme_site(), 5);

    let pods = backend
        .observe_repo_impl(&acme_site())
        .await
        .expect("observed");
    assert_eq!(pods.len(), 1, "the shield injects the just-stopped session");
    assert_eq!(pods[0].session_id, "sess-x");
    assert_eq!(pods[0].liveness, PodLiveness::Terminating);
    assert_eq!(pods[0].trigger_issue, 5);
}

#[tokio::test]
async fn observe_repo_does_not_inject_an_expired_shield_entry() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    // A zero-length window means the entry is already expired when observe prunes it.
    let mut config = osb_config();
    config.reconcile_window = Duration::ZERO;
    let backend = backend(&server.uri(), config);
    backend.record_shield("sess-x", acme_site(), 5);

    let pods = backend
        .observe_repo_impl(&acme_site())
        .await
        .expect("observed");
    assert!(
        pods.is_empty(),
        "an expired shield entry must not be injected"
    );
}

#[tokio::test]
async fn mark_pending_patches_only_the_last_pending_key() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(list_page(json!([sbx("sbx-1", "Running", SESSION_ID)]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("PATCH"))
        .and(path("/v1/sandboxes/sbx-1/metadata"))
        .and(header("OPEN-SANDBOX-API-KEY", API_KEY))
        .respond_with(ResponseTemplate::new(200).set_body_json(sandbox_json(
            "sbx-1",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(&server)
        .await;

    backend(&server.uri(), osb_config())
        .mark_pending_impl(SESSION_ID)
        .await
        .expect("patched");

    // The merge-patch body must carry ONLY the last-pending key (immutability of the
    // rest of the correlation metadata).
    let requests = server.received_requests().await.expect("recorded requests");
    let patch = requests
        .iter()
        .find(|r| r.url.path() == "/v1/sandboxes/sbx-1/metadata")
        .expect("a metadata patch");
    let body: Value = serde_json::from_slice(&patch.body).expect("json body");
    let obj = body.as_object().expect("object body");
    assert_eq!(obj.len(), 1, "patch touches exactly one key");
    assert!(obj.contains_key("fkst-last-pending-at"));
}

#[tokio::test]
async fn mark_pending_is_not_found_when_no_sandbox_resolves() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .mark_pending_impl(SESSION_ID)
        .await
        .expect_err("not found");
    assert!(matches!(err, BackendError::NotFound), "got {err:?}");
}
