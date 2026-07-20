//! Handler-level wiremock tests for the queue-work-item mutation: it must
//! resolve the session's work label from the trigger issue body, stamp that
//! label on a NEW issue created with the USER token, and reject the bad-input
//! and wrong-target cases before any GitHub write.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::routes::canvas::test_support::{
    auth_headers, mount_repo_admin, test_state, viewer_user,
};

fn work_item_request() -> CreateWorkItemRequest {
    CreateWorkItemRequest {
        title: "  build the landing page  ".to_string(),
        body: Some("  do it well  ".to_string()),
    }
}

/// A minimal, valid trigger issue body carrying the given explicit work label.
fn trigger_body(work_label: &str) -> String {
    format!(
        "### Session Name\n\nsite\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n\n\
         ### Work Label\n\n{work_label}\n"
    )
}

/// A valid trigger body carrying an explicit work label AND a `### Session
/// Collaborators` list (used to prove the collaborator authority tier).
fn trigger_body_with_collaborators(work_label: &str, collaborators: &str) -> String {
    format!(
        "### Session Name\n\nsite\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n\n\
         ### Work Label\n\n{work_label}\n\n### Session Collaborators\n\n{collaborators}\n"
    )
}

/// Mount the trigger-read GET returning an issue authored by `author_id` with the
/// given body + labels (and optionally flagged as a pull request).
async fn mount_trigger(
    server: &MockServer,
    number: i64,
    author_id: i64,
    body: &str,
    labels: &[&str],
    is_pr: bool,
) {
    let mut payload = serde_json::json!({
        "number": number,
        "body": body,
        "labels": labels.iter().map(|l| serde_json::json!({ "name": l })).collect::<Vec<_>>(),
        "user": { "id": author_id },
    });
    if is_pr {
        payload["pull_request"] = serde_json::json!({ "url": "https://example.test/pull" });
    }
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/site/issues/{number}")))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_work_item_stamps_the_sessions_work_label_as_the_user() {
    let server = MockServer::start().await;
    // The viewer (id 9) is the trigger author, so the author tier authorizes the
    // work item (the repo-admin read still runs and returns non-admin).
    mount_trigger(
        &server,
        21,
        viewer_user().id,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger", "fkst-substrate-active"],
        false,
    )
    .await;
    mount_repo_admin(&server, "acme", "site", false).await;
    // The new work issue must carry the SESSION's resolved work label, the
    // trimmed title, and be created with the USER token (never an App token).
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({
            "title": "build the landing page",
            "body": "do it well",
            "labels": ["site-build"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 77,
            "html_url": "https://github.com/acme/site/issues/77"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect("201");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 77);
    assert_eq!(created.html_url, "https://github.com/acme/site/issues/77");
}

#[tokio::test]
async fn create_work_item_rejects_a_blank_title_before_any_github_call() {
    // No mock is mounted: reaching GitHub would fail the test loudly.
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(CreateWorkItemRequest {
            title: "   ".to_string(),
            body: None,
        }),
    )
    .await
    .expect_err("a blank title is a 400");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_maps_a_missing_trigger_to_not_found() {
    let server = MockServer::start().await;
    // The trigger-read GET itself 404s (no such issue / no access); no POST
    // mock, so a create attempt would fail loudly.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/999"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 999)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("404 maps");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_refuses_a_non_trigger_issue() {
    let server = MockServer::start().await;
    // A real issue, but not a session trigger (missing the trigger label).
    mount_trigger(&server, 30, 100, "unrelated body", &["bug"], false).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 30)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("missing the trigger label");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_refuses_a_session_without_an_explicit_work_label() {
    let server = MockServer::start().await;
    // A valid trigger, but with no `### Work Label` section — the wake labels
    // are auto-discovered, so there is no single label to stamp. The viewer (id 9)
    // is the author, so authz passes and the handler reaches the work-label check.
    let body = "### Session Name\n\nsite\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n";
    mount_trigger(
        &server,
        21,
        viewer_user().id,
        body,
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    mount_repo_admin(&server, "acme", "site", false).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("no explicit work label");
    assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_allows_a_repo_admin_or_org_owner() {
    let server = MockServer::start().await;
    // Authored by someone else (id 100), no collaborators — but the viewer (id 9)
    // holds admin on the repo, so the admin / org-owner tier authorizes the queue.
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    mount_repo_admin(&server, "acme", "site", true).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(
            serde_json::json!({ "labels": ["site-build"] }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 78,
            "html_url": "https://github.com/acme/site/issues/78"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect("an admin may queue work items");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 78);
}

#[tokio::test]
async fn create_work_item_allows_a_listed_session_collaborator() {
    let server = MockServer::start().await;
    // Authored by someone else (id 100); the viewer (login "shining") is NOT a
    // repo admin, but IS a listed Session Collaborator (matched case-insensitively
    // by login) — the collaborator tier authorizes the queue.
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body_with_collaborators("site-build", "@Shining other-dev"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    mount_repo_admin(&server, "acme", "site", false).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(
            serde_json::json!({ "labels": ["site-build"] }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 79,
            "html_url": "https://github.com/acme/site/issues/79"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect("a listed collaborator may queue work items");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 79);
}

#[tokio::test]
async fn create_work_item_forbids_a_stranger() {
    let server = MockServer::start().await;
    // Authored by someone else (id 100), no collaborators, and the viewer is not
    // a repo admin — a stranger with mere write access has no work-item authority.
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    mount_repo_admin(&server, "acme", "site", false).await;
    // The work issue must NEVER be created for an unauthorized caller.
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("a stranger is forbidden");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_rejects_issue_number_zero() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 0)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("0 is not an issue number");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}
