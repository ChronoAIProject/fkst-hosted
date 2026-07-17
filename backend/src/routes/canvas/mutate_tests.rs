//! Handler-level wiremock tests for the create/stop session mutations: both
//! must act with the USER token, and both must map GitHub's refusals onto the
//! error envelope.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::routes::canvas::test_support::{auth_headers, test_state, viewer_user};

fn create_request() -> CreateSessionRequest {
    CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        work_label: Some("site-build".to_string()),
        environment: None,
        auto_merge: Some(true),
        log_access: Vec::new(),
        output_lang: None,
    }
}

#[tokio::test]
async fn create_session_opens_the_trigger_issue_as_the_user() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        // The USER token (never an App token) must authenticate the write, the
        // title must be the session name, and the trigger label must be applied.
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({
            "title": "site",
            "labels": ["fkst-substrate-trigger"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 21,
            "html_url": "https://github.com/acme/site/issues/21"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let (status, Json(created)) = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect("201");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 21);
    assert_eq!(created.html_url, "https://github.com/acme/site/issues/21");
}

#[tokio::test]
async fn create_session_rejects_an_invalid_spec_before_any_github_write() {
    // No POST mock is mounted: reaching GitHub would fail the test loudly.
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(CreateSessionRequest {
            packages: vec!["broken".to_string()],
            ..create_request()
        }),
    )
    .await
    .expect_err("an invalid package ref is a 400");
    match err {
        AppError::Validation(message) => {
            assert!(
                message.contains("### Packages"),
                "parser message: {message}"
            )
        }
        other => panic!("expected Validation, got {other:?}"),
    }
}

#[tokio::test]
async fn create_session_maps_a_github_403_to_forbidden() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(403).set_body_json(serde_json::json!({
            "message": "Resource not accessible by personal access token"
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect_err("403 maps");
    match err {
        AppError::Forbidden(message) => {
            assert!(message.contains("Resource not accessible"), "{message}")
        }
        other => panic!("expected Forbidden, got {other:?}"),
    }
}

#[tokio::test]
async fn create_session_maps_a_github_404_to_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/gone/issues"))
        .respond_with(ResponseTemplate::new(404).set_body_json(serde_json::json!({
            "message": "Not Found"
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "gone".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect_err("404 maps");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn stop_session_closes_the_trigger_issue_as_the_user() {
    let server = MockServer::start().await;
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
    Mock::given(method("PATCH"))
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
