//! Handler-level wiremock tests for the stop-session mutation: it must act with
//! the USER token, enforce request-time SESSION-MANAGEMENT authorization (only
//! the trigger author or a repo admin / org owner — never a work-item
//! collaborator), and map GitHub's refusals onto the error envelope.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::routes::canvas::test_support::{
    auth_headers, mount_repo_admin, test_state, viewer_user,
};

/// Mount the stop-session pre-flight GET returning an issue authored by
/// `author_id` with the given labels (and optionally flagged as a pull request).
async fn mount_get_issue(
    server: &MockServer,
    number: i64,
    author_id: i64,
    labels: &[&str],
    is_pr: bool,
) {
    let mut body = serde_json::json!({
        "number": number,
        "labels": labels.iter().map(|l| serde_json::json!({ "name": l })).collect::<Vec<_>>(),
        "user": { "id": author_id },
    });
    if is_pr {
        body["pull_request"] = serde_json::json!({ "url": "https://example.test/pull" });
    }
    Mock::given(method("GET"))
        .and(path(format!("/repos/acme/site/issues/{number}")))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

#[tokio::test]
async fn stop_session_closes_the_trigger_issue_as_the_author() {
    let server = MockServer::start().await;
    // The viewer (id 9) is the trigger author, so the author tier authorizes the
    // stop and no repo-admin lookup is needed (the `||` short-circuits).
    mount_get_issue(
        &server,
        21,
        viewer_user().id,
        &["fkst-substrate-trigger", "fkst-substrate-active"],
        false,
    )
    .await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/21"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({ "state": "closed" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 21, "state": "closed"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let status = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("204");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn stop_session_maps_a_github_404_to_not_found() {
    let server = MockServer::start().await;
    // The pre-flight GET itself 404s (no such issue / no access).
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/999"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 999)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("404 maps");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_refuses_a_non_trigger_issue_without_closing_it() {
    let server = MockServer::start().await;
    mount_get_issue(&server, 30, 100, &["bug", "fkst-substrate-active"], false).await;
    // No PATCH mock: the guard must reject before any close reaches GitHub.
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/30"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 30)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("missing the trigger label");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_refuses_a_pull_request() {
    let server = MockServer::start().await;
    mount_get_issue(&server, 42, 100, &["fkst-substrate-trigger"], true).await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/42"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 42)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a PR is not a trigger issue");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_rejects_issue_number_zero() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 0)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("0 is not an issue number");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_allows_a_repo_admin_or_org_owner() {
    let server = MockServer::start().await;
    // The trigger was authored by someone ELSE (id 100), but the viewer (id 9)
    // holds admin on the repo — a repo admin / org owner may stop any session.
    mount_get_issue(&server, 21, 100, &["fkst-substrate-trigger"], false).await;
    mount_repo_admin(&server, "acme", "site", true).await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/21"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({ "state": "closed" })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "number": 21, "state": "closed"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let status = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("an admin may stop the session");
    assert_eq!(status, StatusCode::NO_CONTENT);
}

#[tokio::test]
async fn stop_session_forbids_a_stranger_with_write_but_not_admin() {
    let server = MockServer::start().await;
    // Authored by someone else (id 100); the viewer (id 9) is neither the author
    // nor a repo admin — a mere write-capable collaborator cannot stop a session.
    mount_get_issue(&server, 21, 100, &["fkst-substrate-trigger"], false).await;
    mount_repo_admin(&server, "acme", "site", false).await;
    // The close must NEVER reach GitHub for an unauthorized caller.
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/21"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a non-author non-admin is forbidden");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_forbids_a_work_item_collaborator() {
    // Session management is NOT work-item authority: even a listed Session
    // Collaborator (who could queue work items) cannot STOP a session. stop_session
    // never consults the collaborator list at all — it admits only the author +
    // admins — so a collaborator who is neither author nor admin is rejected.
    let server = MockServer::start().await;
    mount_get_issue(&server, 21, 100, &["fkst-substrate-trigger"], false).await;
    mount_repo_admin(&server, "acme", "site", false).await;
    Mock::given(method("PATCH"))
        .and(path("/repos/acme/site/issues/21"))
        .respond_with(ResponseTemplate::new(200))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = stop_session(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a work-item collaborator cannot stop the session");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}
