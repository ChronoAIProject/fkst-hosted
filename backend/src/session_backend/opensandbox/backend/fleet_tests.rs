//! Wiremock tests for the stop / GC / enumerate verbs: `stop_session` +
//! `remove_terminal` (delete + benign 404 + shield recorded), and `list_fleet`
//! (recover N sessions, reap duplicates keeping the oldest with an id tie-break, and
//! the single-writer convergence backstop).

use serde_json::{json, Value};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, PodLiveness};
use crate::session_backend::{BackendError, EnsureOutcome};

use super::super::backend_test_support::{
    backend, correlation_metadata, list_page, osb_config, sandbox_json, spec, SESSION_ID,
};

const WORK_LABEL_HEX: &str = "666b73742d776f726b";

fn acme_site() -> RepoRef {
    RepoRef {
        owner: "acme".to_string(),
        name: "site".to_string(),
    }
}

/// An `acme/site` sandbox for `session` at `created_at`.
fn sbx(id: &str, created_at: &str, session: &str) -> Value {
    sandbox_json(
        id,
        "Running",
        created_at,
        correlation_metadata(session, "acme", "site", WORK_LABEL_HEX),
    )
}

/// The `metadata` filter value `resolve_one` builds for a session.
fn resolve_filter() -> String {
    format!("fkst-managed=true&fkst-session-id={SESSION_ID}")
}

#[tokio::test]
async fn stop_session_deletes_the_resolved_sandbox_and_records_the_shield() {
    let server = MockServer::start().await;
    // resolve_one finds the sandbox...
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("metadata", resolve_filter()))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(list_page(json!([sbx(
                "sbx-1",
                "2026-07-09T00:00:00Z",
                SESSION_ID
            )]))),
        )
        .mount(&server)
        .await;
    // ...and the delete is issued.
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    // A subsequent observe of the repo lists NOTHING (the delete 404s instantly).
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param(
            "metadata",
            "fkst-managed=true&fkst-owner=acme&fkst-repo=site",
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let backend = backend(&server.uri(), osb_config());
    backend
        .stop_session_impl(SESSION_ID, KillReason::Idle)
        .await
        .expect("stopped");

    // The shield reports the just-stopped session as Terminating (so the planner does
    // not thrash-respawn it the same tick it was killed).
    let pods = backend
        .observe_repo_impl(&acme_site())
        .await
        .expect("observed");
    assert_eq!(pods.len(), 1);
    assert_eq!(pods[0].session_id, SESSION_ID);
    assert_eq!(pods[0].liveness, PodLiveness::Terminating);
    assert_eq!(pods[0].trigger_issue, 7);
}

#[tokio::test]
async fn stop_session_delete_404_is_a_benign_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(list_page(json!([sbx(
                "sbx-1",
                "2026-07-09T00:00:00Z",
                SESSION_ID
            )]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-1"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .stop_session_impl(SESSION_ID, KillReason::Idle)
        .await
        .expect_err("404 delete surfaces as NotFound");
    assert!(matches!(err, BackendError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn stop_session_is_not_found_when_nothing_resolves() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .stop_session_impl(SESSION_ID, KillReason::Idle)
        .await
        .expect_err("not found");
    assert!(matches!(err, BackendError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn remove_terminal_deletes_the_resolved_sandbox() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(list_page(json!([sbx(
                "sbx-1",
                "2026-07-09T00:00:00Z",
                SESSION_ID
            )]))),
        )
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/sbx-1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    backend(&server.uri(), osb_config())
        .remove_terminal_impl(SESSION_ID)
        .await
        .expect("removed");
}

#[tokio::test]
async fn list_fleet_recovers_each_session_and_reaps_duplicates_keeping_oldest() {
    let server = MockServer::start().await;
    // sess-a: single. sess-b: a duplicate pair; the NEWER must be reaped.
    let items = json!([
        sbx("a1", "2026-07-09T00:00:00Z", "sess-a"),
        sbx("b-new", "2026-07-09T01:00:00Z", "sess-b"),
        sbx("b-old", "2026-07-09T00:00:00Z", "sess-b"),
    ]);
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(items)))
        .mount(&server)
        .await;
    // Only the newer duplicate is deleted; the survivors are left alone.
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/b-new"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/b-old"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/a1"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;

    let handles = backend(&server.uri(), osb_config())
        .list_fleet_impl()
        .await
        .expect("fleet");
    let mut sessions: Vec<String> = handles.into_iter().map(|h| h.session_id).collect();
    sessions.sort();
    assert_eq!(sessions, vec!["sess-a".to_string(), "sess-b".to_string()]);
}

#[tokio::test]
async fn list_fleet_reaper_breaks_a_created_at_tie_by_id() {
    let server = MockServer::start().await;
    // Same createdAt → the lexicographically smaller id (`c-aaa`) is kept.
    let items = json!([
        sbx("c-bbb", "2026-07-09T00:00:00Z", "sess-c"),
        sbx("c-aaa", "2026-07-09T00:00:00Z", "sess-c"),
    ]);
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(items)))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/c-bbb"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/c-aaa"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;

    let handles = backend(&server.uri(), osb_config())
        .list_fleet_impl()
        .await
        .expect("fleet");
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].session_id, "sess-c");
}

#[tokio::test]
async fn two_serialized_ensures_both_create_then_list_fleet_converges_to_one() {
    let server = MockServer::start().await;
    // The list-guard reports empty for BOTH ensures (a brief single-writer violation),
    // so both create a sandbox for the same session.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("metadata", resolve_filter()))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(202).set_body_json(sandbox_json(
            "sbx-created",
            "Running",
            "2026-07-09T00:00:00Z",
            json!({}),
        )))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/v1/sandboxes/sbx-created/proxy/44772/files/upload"))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;
    // list_fleet then sees the two duplicate sandboxes and reaps the newer.
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .and(query_param("metadata", "fkst-managed=true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([
            sbx("dup-new", "2026-07-09T01:00:00Z", SESSION_ID),
            sbx("dup-old", "2026-07-09T00:00:00Z", SESSION_ID),
        ]))))
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/dup-new"))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;
    Mock::given(method("DELETE"))
        .and(path("/v1/sandboxes/dup-old"))
        .respond_with(ResponseTemplate::new(204))
        .expect(0)
        .mount(&server)
        .await;

    let backend = backend(&server.uri(), osb_config());
    let creds = || {
        std::collections::BTreeMap::from([(
            "github-token".to_string(),
            secrecy::SecretString::from("ghs".to_string()),
        )])
    };
    // Serialized, single-writer: two ensures in sequence, both create.
    assert_eq!(
        backend.ensure_session_impl(&spec(), creds()).await.unwrap(),
        EnsureOutcome::Created
    );
    assert_eq!(
        backend.ensure_session_impl(&spec(), creds()).await.unwrap(),
        EnsureOutcome::Created
    );

    // The reaper converges the duplicate fleet back to exactly one, keeping the oldest.
    let handles = backend.list_fleet_impl().await.expect("fleet");
    assert_eq!(handles.len(), 1);
    assert_eq!(handles[0].session_id, SESSION_ID);
}
