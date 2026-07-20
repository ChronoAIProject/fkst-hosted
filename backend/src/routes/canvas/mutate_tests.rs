//! Handler-level wiremock tests for the create-session mutation: it must act
//! with the USER token, run the work-label-collision pre-flight, and map GitHub's
//! refusals onto the error envelope. (Stop-session lives in `mutate_stop_tests`.)

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
        collaborators: Vec::new(),
        output_lang: None,
    }
}

/// Mount the create-session pre-flight read: `GET /repos/{owner}/{name}/issues`
/// (the repo's OPEN trigger issues), returning one minimal trigger per
/// `(number, work_label)`. The USER token must authenticate it, matching the
/// create write. Distinct GET method from the POST create mock on the same path.
async fn mount_open_triggers(
    server: &MockServer,
    owner: &str,
    name: &str,
    sessions: &[(i64, &str)],
) {
    let issues: Vec<serde_json::Value> = sessions
        .iter()
        .map(|(number, label)| {
            serde_json::json!({
                "number": number,
                "title": format!("session-{number}"),
                "body": format!(
                    "### Session Name\n\nsession-{number}\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n\n### Work Label\n\n{label}\n"
                ),
                "labels": [{ "name": "fkst-substrate-trigger" }],
                "state": "open",
                "user": { "login": "someone", "id": 7 },
                "html_url": format!("https://github.com/{owner}/{name}/issues/{number}"),
                "created_at": "",
                "updated_at": "",
            })
        })
        .collect();
    Mock::given(method("GET"))
        .and(path(format!("/repos/{owner}/{name}/issues")))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(issues)))
        .mount(server)
        .await;
}

#[tokio::test]
async fn create_session_opens_the_trigger_issue_as_the_user() {
    let server = MockServer::start().await;
    // Pre-flight sees a distinct existing session — no collision, create proceeds.
    mount_open_triggers(&server, "acme", "site", &[(7, "other-build")]).await;
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
    // Pre-flight passes (no open sessions); the create write is what 403s.
    mount_open_triggers(&server, "acme", "site", &[]).await;
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
    // Pre-flight passes (no open sessions); the create write is what 404s.
    mount_open_triggers(&server, "acme", "gone", &[]).await;
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
async fn create_session_rejects_a_colliding_work_label_before_creating() {
    let server = MockServer::start().await;
    // An existing OPEN session already owns the requested "site-build" label.
    mount_open_triggers(&server, "acme", "site", &[(19, "site-build")]).await;
    // The create write must never fire once the collision is detected.
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
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
    .expect_err("a colliding explicit work label is a 409");
    match err {
        AppError::Conflict(message) => {
            assert!(message.contains("site-build"), "message: {message}");
            assert!(message.contains("#19"), "message: {message}");
        }
        other => panic!("expected Conflict, got {other:?}"),
    }
}

#[tokio::test]
async fn create_session_allows_a_distinct_work_label() {
    let server = MockServer::start().await;
    // The open session uses a different label — no collision, create proceeds.
    mount_open_triggers(&server, "acme", "site", &[(19, "other-build")]).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 22,
            "html_url": "https://github.com/acme/site/issues/22"
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
    assert_eq!(created.issue_number, 22);
}

#[tokio::test]
async fn create_session_allows_when_no_existing_sessions() {
    let server = MockServer::start().await;
    // The repo has no open trigger issues at all — nothing to collide with.
    mount_open_triggers(&server, "acme", "site", &[]).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 23,
            "html_url": "https://github.com/acme/site/issues/23"
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
    assert_eq!(created.issue_number, 23);
}

#[tokio::test]
async fn create_session_without_explicit_work_label_skips_the_preflight() {
    let server = MockServer::start().await;
    // No open-triggers GET is mounted: the pre-flight must NOT run for a request
    // with no explicit work label. Were it to run, the unmatched GET would 404
    // and fail the create — reaching the 201 proves the pre-flight was skipped.
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 24,
            "html_url": "https://github.com/acme/site/issues/24"
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
        Json(CreateSessionRequest {
            work_label: None,
            ..create_request()
        }),
    )
    .await
    .expect("201");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 24);
}
