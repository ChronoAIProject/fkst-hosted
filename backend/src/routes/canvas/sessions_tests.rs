//! Handler-level wiremock tests for the per-repo canvas sessions surface,
//! plus unit tests of its pure helpers. The wiremock server plays the
//! user-token reads, the App-token mint, and the issue/PR reads.

use std::sync::Arc;

use axum::extract::{Path, State};
use k8s_openapi::chrono::Utc;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::reconcile::desired::LivePod;
use crate::routes::canvas::test_support::{
    auth_headers, mount_app_token, test_app, test_state, viewer_user,
};
use crate::session_backend::test_support::FakeSessionBackend;
use crate::session_spec::derive_session_id;

const VALID_TRIGGER_BODY: &str = "### Session Name\nsite\n\n### Packages\n\
acme/pkgs@main:packages/devloop\n\n### Work Label\nsite-build\n";

fn issue_json(number: i64, body: &str, label: &str, state: &str) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("issue-{number}"),
        "body": body,
        "state": state,
        "labels": [{ "name": label }],
        "user": { "login": "shining", "id": 9 },
        "html_url": format!("https://github.com/acme/site/issues/{number}"),
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z",
        "closed_at": if state == "closed" { serde_json::json!("2026-07-03T00:00:00Z") } else { serde_json::Value::Null }
    })
}

fn pull_json(
    number: i64,
    author: &str,
    head_ref: &str,
    title: &str,
    state: &str,
    merged: bool,
) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": title,
        "html_url": format!("https://github.com/acme/site/pull/{number}"),
        "state": state,
        "merged_at": if merged { serde_json::json!("2026-07-04T00:00:00Z") } else { serde_json::Value::Null },
        "user": { "login": author },
        "head": { "ref": head_ref }
    })
}

async fn mount_installation_covering_site(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [
                { "id": 77, "account": { "login": "acme" }, "repository_selection": "all" }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/77/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "repositories": [{ "name": "site", "owner": { "login": "acme" } }]
        })))
        .mount(server)
        .await;
}

#[tokio::test]
async fn repo_sessions_assembles_the_full_detail() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            issue_json(5, VALID_TRIGGER_BODY, "fkst-substrate-trigger", "open"),
            issue_json(6, "no headings", "fkst-substrate-trigger", "open"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "site-build"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            issue_json(8, "work", "site-build", "open"),
            issue_json(9, "done work", "site-build", "closed"),
        ])))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .and(query_param("state", "all"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            // The one devloop PR belonging to work issue #8 (merged).
            pull_json(
                12,
                "fkst-test[bot]",
                "devloop/issue/acme/site/8/ready-1",
                "devloop implementation for #8",
                "closed",
                true
            ),
            // A human PR on a devloop-looking branch: filtered by author.
            pull_json(
                13,
                "human",
                "devloop/issue/acme/site/8/ready-2",
                "manual fix",
                "open",
                false
            ),
            // A bot PR that is NOT devloop (no parseable issue): filtered.
            pull_json(
                14,
                "fkst-test[bot]",
                "fkst/issue-templates-v3",
                "fkst issue templates",
                "open",
                false
            ),
            // A bot devloop PR for an issue outside this session's work label.
            pull_json(
                15,
                "fkst-test[bot]",
                "devloop/issue/acme/site/99/ready-1",
                "devloop implementation for #99",
                "open",
                false
            ),
        ])))
        .mount(&server)
        .await;

    let session_id = derive_session_id(77, "acme", "site", 5);
    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some("fkst-test[bot]".to_string());
    state.config.log.public_base_url = Some("https://fkst.example/".to_string());
    state.session_backend = Some(Arc::new(FakeSessionBackend::default().with_observed(vec![
        LivePod {
            session_id: session_id.clone(),
            trigger_issue: 5,
            liveness: PodLiveness::Live,
            created_at: Utc::now(),
            last_pending_at: None,
            config_hash: None,
            work_label: Some("site-build".to_string()),
        },
    ])));

    let Json(view) = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200");

    assert_eq!(view.owner, "acme");
    assert_eq!(view.name, "site");
    assert!(view.installed);
    assert_eq!(view.sessions.len(), 2);

    let session = &view.sessions[0];
    assert_eq!(session.session_id.as_deref(), Some(session_id.as_str()));
    assert_eq!(session.name.as_deref(), Some("site"));
    assert_eq!(session.work_label.as_deref(), Some("site-build"));
    assert_eq!(session.auto_merge, Some(false));
    assert_eq!(
        session.packages,
        vec!["acme/pkgs@main:packages/devloop".to_string()]
    );
    assert!(session.invalid_reason.is_none());
    assert_eq!(
        session.status_labels,
        vec!["fkst-substrate-trigger".to_string()]
    );
    assert_eq!(session.trigger.number, 5);
    assert_eq!(
        session.trigger.html_url,
        "https://github.com/acme/site/issues/5"
    );
    assert_eq!(session.work_issues.len(), 2);
    assert_eq!(session.work_issues[1].number, 9);
    assert_eq!(
        session.work_issues[1].closed_at.as_deref(),
        Some("2026-07-03T00:00:00Z")
    );
    assert_eq!(
        session.log_url.as_deref(),
        Some(format!("https://fkst.example/api/v1/logs/{session_id}").as_str()),
        "trailing base-URL slash must not double up"
    );
    assert_eq!(session.liveness.as_deref(), Some("live"));
    assert_eq!(
        session.prs.len(),
        1,
        "only the bot devloop PR for #8 belongs"
    );
    assert_eq!(session.prs[0].number, 12);
    assert!(session.prs[0].merged);
    assert_eq!(session.prs[0].work_issue, Some(8));

    let invalid = &view.sessions[1];
    assert!(invalid.session_id.is_none());
    assert!(invalid.invalid_reason.is_some());
    assert!(invalid.work_issues.is_empty());
    assert!(invalid.prs.is_empty());
    assert!(invalid.liveness.is_none());
    assert!(invalid.log_url.is_none());
}

#[tokio::test]
async fn repo_sessions_canonicalizes_a_case_variant_path() {
    let server = MockServer::start().await;
    // The installation listing carries GitHub's canonical `acme/site`.
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    // No `### Work Label` section, so the scan reads no work issues.
    let body = "### Session Name\nsite\n\n### Packages\nacme/pkgs@main:packages/devloop\n";
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "all"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                5,
                body,
                "fkst-substrate-trigger",
                "open"
            )])),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = repo_sessions(
        State(state),
        Path(("ACME".to_string(), "Site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200 for the case-variant path");

    assert_eq!(view.owner, "acme", "the response echoes GitHub's casing");
    assert_eq!(view.name, "site");
    assert!(view.installed);
    assert_eq!(view.sessions.len(), 1);
    assert_eq!(
        view.sessions[0].session_id.as_deref(),
        Some(derive_session_id(77, "acme", "site", 5).as_str()),
        "the session id must derive from the canonical casing, never the caller's"
    );
}

#[tokio::test]
async fn repo_sessions_outside_the_callers_installations_is_not_installed() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0,
            "installations": []
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200 with installed=false");
    assert!(!view.installed);
    assert!(view.sessions.is_empty());
}

#[tokio::test]
async fn repo_sessions_without_an_app_is_unavailable() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;

    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("no App configured is a 503");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn repo_sessions_propagates_a_github_failure() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a 500 from GitHub must error");
    assert!(matches!(err, AppError::Upstream(_)), "got {err:?}");
}

#[tokio::test]
async fn repo_sessions_rejects_a_malformed_owner() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = repo_sessions(
        State(state),
        Path(("bad owner".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("malformed owner is a 400");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

// ---- pure helpers ----------------------------------------------------------

#[test]
fn liveness_label_maps_only_the_three_visible_phases() {
    assert_eq!(liveness_label(PodLiveness::Starting), Some("starting"));
    assert_eq!(liveness_label(PodLiveness::Live), Some("live"));
    assert_eq!(
        liveness_label(PodLiveness::Terminating),
        Some("terminating")
    );
    assert_eq!(liveness_label(PodLiveness::Absent), None);
    assert_eq!(liveness_label(PodLiveness::Terminal), None);
}

#[test]
fn validate_repo_segment_accepts_github_charset_only() {
    for good in ["acme", "a.b_c-d", "Repo1"] {
        validate_repo_segment(good, "owner").expect(good);
    }
    for bad in ["", "a b", "a/b", "a\nb", "é"] {
        assert!(
            validate_repo_segment(bad, "owner").is_err(),
            "{bad:?} must be rejected"
        );
    }
}

#[test]
fn devloop_prs_without_a_bot_login_lists_nothing() {
    let pulls = vec![crate::routes::canvas::github::RepoPull {
        number: 1,
        title: "devloop implementation for #8".to_string(),
        html_url: "u".to_string(),
        state: "open".to_string(),
        merged: false,
        author: "fkst-test[bot]".to_string(),
        head_ref: "devloop/issue/acme/site/8/ready-1".to_string(),
    }];
    assert!(devloop_prs(&pulls, None).is_empty());
    let prs = devloop_prs(&pulls, Some("fkst-test[bot]"));
    assert_eq!(prs.len(), 1);
    assert_eq!(prs[0].work_issue, Some(8));
}
