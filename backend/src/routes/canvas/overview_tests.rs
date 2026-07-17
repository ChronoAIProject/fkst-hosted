//! Handler-level wiremock tests for `GET /api/v1/overview`: the wiremock
//! server plays BOTH the user-token GitHub reads and the App-token mint +
//! trigger reads, and the handler is invoked directly with a pre-verified
//! [`GithubUser`] (identity extraction is covered by the extractor's own tests).

use axum::extract::State;
use axum::http::HeaderMap;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::routes::canvas::test_support::{
    auth_headers, mount_app_token, test_app, test_state, viewer_user,
};

const VALID_TRIGGER_BODY: &str = "### Session Name\nsite\n\n### Packages\n\
acme/pkgs@main:packages/devloop\n\n### Work Label\nsite-build\n";

fn repo_json(owner: &str, kind: &str, name: &str, id: i64) -> serde_json::Value {
    serde_json::json!({
        "id": id,
        "name": name,
        "owner": { "login": owner, "type": kind },
        "private": kind == "User",
        "permissions": { "admin": true }
    })
}

async fn mount_user_reads(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            repo_json("shining", "User", "notes", 1),
            repo_json("acme", "Organization", "site", 2),
        ])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/orgs"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([{ "login": "acme" }])),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/memberships/orgs"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            { "role": "admin", "organization": { "login": "acme" } }
        ])))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [
                { "id": 77, "account": { "login": "acme" }, "repository_selection": "selected" }
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
async fn overview_assembles_accounts_counts_and_totals() {
    let server = MockServer::start().await;
    mount_user_reads(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .and(query_param("state", "open"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 5, "title": "trigger", "body": VALID_TRIGGER_BODY, "state": "open",
                "labels": [{ "name": "fkst-substrate-trigger" }],
                "user": { "login": "shining", "id": 9 }
            },
            {
                "number": 6, "title": "broken", "body": "no headings", "state": "open",
                "labels": [{ "name": "fkst-substrate-trigger" }],
                "user": { "login": "shining", "id": 9 }
            }
        ])))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = overview(State(state), viewer_user(), auth_headers())
        .await
        .expect("200");

    assert_eq!(view.app_slug.as_deref(), Some("fkst-test"));
    assert_eq!(view.viewer.login, "shining");
    assert_eq!(view.accounts.len(), 2, "personal first, then the org");

    let personal = &view.accounts[0];
    assert_eq!(personal.login, "shining");
    assert_eq!(personal.kind, "personal");
    assert!(personal.owner, "the personal account is always owned");
    assert!(!personal.installed);
    assert!(personal.installation_id.is_none());
    assert!(personal.counts_complete);
    assert_eq!(personal.repos.len(), 1);
    assert_eq!(personal.repos[0].name, "notes");
    assert!(!personal.repos[0].installed);
    assert_eq!(personal.repos[0].active_sessions, 0);

    let org = &view.accounts[1];
    assert_eq!(org.login, "acme");
    assert_eq!(org.kind, "org");
    assert!(org.owner, "membership role admin marks the org owned");
    assert!(org.installed);
    assert_eq!(org.installation_id, Some(77));
    assert_eq!(org.repository_selection.as_deref(), Some("selected"));
    assert!(org.counts_complete);
    assert_eq!(org.repos.len(), 1);
    let site = &org.repos[0];
    assert!(site.installed);
    assert_eq!(
        site.active_sessions, 1,
        "only the parsing trigger counts as active"
    );
    assert_eq!(
        site.packages,
        vec!["acme/pkgs@main:packages/devloop".to_string()]
    );

    assert_eq!(view.totals.sessions, 1);
    assert_eq!(view.totals.packages.len(), 1);
    assert_eq!(
        view.totals.packages[0].package,
        "acme/pkgs@main:packages/devloop"
    );
    assert_eq!(view.totals.packages[0].count, 1);
}

#[tokio::test]
async fn overview_marks_counts_incomplete_when_a_trigger_read_fails() {
    let server = MockServer::start().await;
    mount_user_reads(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let Json(view) = overview(State(state), viewer_user(), auth_headers())
        .await
        .expect("a failing repo scan must NOT fail the whole call");

    let org = &view.accounts[1];
    assert!(!org.counts_complete, "the failed scan flags the account");
    assert_eq!(org.repos[0].active_sessions, 0);
    assert_eq!(view.totals.sessions, 0);
}

#[tokio::test]
async fn overview_marks_counts_incomplete_when_the_app_is_unconfigured() {
    let server = MockServer::start().await;
    mount_user_reads(&server).await;

    let state = test_state(&server.uri(), None);
    let Json(view) = overview(State(state), viewer_user(), auth_headers())
        .await
        .expect("200 without an App");

    assert!(view.app_slug.is_none());
    let org = &view.accounts[1];
    assert!(
        !org.counts_complete,
        "an installed repo with no App creds cannot be counted"
    );
    assert!(
        view.accounts[0].counts_complete,
        "no installed repos, nothing missing"
    );
}

#[tokio::test]
async fn overview_propagates_a_rejected_user_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/repos"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = overview(State(state), viewer_user(), auth_headers())
        .await
        .expect_err("401 from GitHub rejects the call");
    assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
}

#[tokio::test]
async fn overview_requires_a_bearer_token() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = overview(State(state), viewer_user(), HeaderMap::new())
        .await
        .expect_err("missing Authorization header is a 401");
    assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
}
