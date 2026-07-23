//! Handler-level wiremock tests for the create-session mutation: it must act
//! with the USER token, run the work-label-collision pre-flight, and map GitHub's
//! refusals onto the error envelope. (Stop-session lives in `mutate_stop_tests`.)

use axum::extract::{Path, State};
use axum::http::StatusCode;
use wiremock::matchers::{body_partial_json, header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::routes::canvas::test_support::{
    auth_headers, grant_global_admin, mount_app_token, test_app, test_state, viewer_user,
};

fn create_request() -> CreateSessionRequest {
    CreateSessionRequest {
        name: "site".to_string(),
        packages: vec!["acme/pkgs@main:packages/devloop".to_string()],
        manifests: Vec::new(),
        work_label: Some("site-build".to_string()),
        environment: None,
        disposable_environment: None,
        source_branch: None,
        target_branch: None,
        auto_merge: Some(true),
        log_access: Vec::new(),
        collaborators: Vec::new(),
        output_lang: None,
    }
}

async fn state_with_creator_role(
    server: &MockServer,
    owner: &str,
    name: &str,
    status: u16,
    role: Option<&str>,
) -> AppState {
    mount_app_token(server, owner, name, 42).await;
    let response = match role {
        Some(role) => {
            ResponseTemplate::new(status).set_body_json(serde_json::json!({ "role_name": role }))
        }
        None => ResponseTemplate::new(status)
            .set_body_json(serde_json::json!({ "message": "role lookup failed" })),
    };
    Mock::given(method("GET"))
        .and(path(format!(
            "/repos/{owner}/{name}/collaborators/shining/permission"
        )))
        .and(header(
            "authorization",
            "Bearer ghs_test_installation_token",
        ))
        .respond_with(response)
        .mount(server)
        .await;
    test_state(&server.uri(), Some(test_app(&server.uri())))
}

/// Mount the create-session pre-flight read: `GET /repos/{owner}/{name}/issues`
/// (the repo's OPEN trigger issues), returning one minimal trigger per
/// `(number, work_label)`. The USER token must authenticate it, matching the
/// create write. Distinct GET method from the POST create mock on the same path.
async fn mount_open_triggers(
    server: &MockServer,
    owner: &str,
    name: &str,
    sessions: &[(i64, &str, &str)],
) {
    let issues: Vec<serde_json::Value> = sessions
        .iter()
        .map(|(number, label, creator)| {
            serde_json::json!({
                "number": number,
                "title": format!("session-{number}"),
                "body": format!(
                    "### Session Name\n\nsession-{number}\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n\n### Work Label\n\n{label}\n"
                ),
                "labels": [{ "name": "fkst-substrate-trigger" }],
                "state": "open",
                "user": { "login": creator, "id": number + 1000 },
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
    mount_open_triggers(&server, "acme", "site", &[(7, "other-build", "shining")]).await;
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

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
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

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
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

    let state = state_with_creator_role(&server, "acme", "gone", 200, Some("maintain")).await;
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
    mount_open_triggers(&server, "acme", "site", &[(19, "site-build", "ShInInG")]).await;
    // The create write must never fire once the collision is detected.
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
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
    mount_open_triggers(&server, "acme", "site", &[(19, "other-build", "shining")]).await;
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

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
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

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("admin")).await;
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
async fn create_session_from_a_manifest_with_no_work_label_succeeds() {
    let server = MockServer::start().await;
    // A manifest-only request (no explicit packages) with no work label: the
    // pre-flight is skipped (no explicit label) and the create must proceed —
    // the `### Manifest` reference is a valid package source on its own (I7).
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({
            "title": "site",
            "labels": ["fkst-substrate-trigger"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 25,
            "html_url": "https://github.com/acme/site/issues/25"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
    let (status, Json(created)) = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(CreateSessionRequest {
            packages: Vec::new(),
            manifests: vec!["acme/manifests@main:bundles/site".to_string()],
            work_label: None,
            ..create_request()
        }),
    )
    .await
    .expect("201");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 25);
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

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
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

#[tokio::test]
async fn create_session_rejects_a_write_only_creator() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = state_with_creator_role(&server, "acme", "site", 200, Some("write")).await;
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect_err("write is below the creator threshold");
    match err {
        AppError::Forbidden(message) => {
            assert!(message.contains("admin or maintain"), "{message}");
        }
        other => panic!("expected Forbidden, got {other:?}"),
    }
}

#[tokio::test]
async fn create_session_fails_closed_when_role_lookup_is_unavailable() {
    let server = MockServer::start().await;
    let state = state_with_creator_role(&server, "acme", "site", 500, None).await;
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect_err("role lookup failure must be retryable");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn create_session_requires_the_github_app_for_a_non_admin_creator() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect_err("missing App cannot establish the repo role");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn create_session_global_admin_short_circuits_the_repo_role_lookup() {
    let server = MockServer::start().await;
    mount_open_triggers(&server, "acme", "site", &[]).await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 26,
            "html_url": "https://github.com/acme/site/issues/26"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut state = test_state(&server.uri(), None);
    grant_global_admin(&mut state, "@Shining");

    let (status, _) = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect("global admin does not require an App role read");
    assert_eq!(status, StatusCode::CREATED);
}

#[tokio::test]
async fn create_session_allows_another_creators_overlapping_label() {
    let server = MockServer::start().await;
    mount_open_triggers(
        &server,
        "acme",
        "site",
        &[(19, "site-build", "another-creator")],
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 27,
            "html_url": "https://github.com/acme/site/issues/27"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;

    let (status, Json(created)) = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(create_request()),
    )
    .await
    .expect("collision keys include creator");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 27);
}

#[tokio::test]
async fn create_session_rejects_an_invalid_branch_with_422_before_github() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(CreateSessionRequest {
            source_branch: Some("bad branch".to_string()),
            ..create_request()
        }),
    )
    .await
    .expect_err("invalid branch");
    assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
}

#[tokio::test]
async fn disposable_environment_is_handed_off_without_leaking_into_github() {
    use crate::disposable_environment::{
        DisposableEnvironmentLookup, DisposableEnvironmentRequest, DISPOSABLE_ENVIRONMENT_MARKER,
    };
    use crate::reconcile::ReconcileDispatcher;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 31,
            "html_url": "https://github.com/acme/site/issues/31"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut state = state_with_creator_role(&server, "acme", "site", 200, Some("maintain")).await;
    state.reconciler = Some(ReconcileDispatcher::new());
    let registry = state.disposable_environments.clone();
    let request = CreateSessionRequest {
        work_label: None,
        disposable_environment: Some(DisposableEnvironmentRequest {
            install: vec!["install private-tool".to_string()],
            variables: std::collections::BTreeMap::from([(
                "PRIVATE_MODE".to_string(),
                "private-value".to_string(),
            )]),
            secrets: std::collections::BTreeMap::from([(
                "PRIVATE_TOKEN".to_string(),
                "secret-value".to_string(),
            )]),
        }),
        ..create_request()
    };

    let (status, Json(created)) = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(request),
    )
    .await
    .expect("created");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 31);

    let DisposableEnvironmentLookup::Found(material) = registry.resolve("acme", "site", 31, 9)
    else {
        panic!("verified creator should own the private handoff")
    };
    assert_eq!(material.install.len(), 1);
    assert_eq!(material.user_env.len(), 2);

    let requests = server.received_requests().await.expect("request journal");
    let github_write = requests
        .iter()
        .find(|request| {
            request.method.as_str() == "POST" && request.url.path() == "/repos/acme/site/issues"
        })
        .expect("GitHub issue write");
    let payload: serde_json::Value =
        serde_json::from_slice(&github_write.body).expect("GitHub request JSON");
    let body = payload["body"].as_str().expect("issue body");
    assert!(body.contains(DISPOSABLE_ENVIRONMENT_MARKER));
    for private in [
        "install private-tool",
        "PRIVATE_MODE",
        "private-value",
        "PRIVATE_TOKEN",
        "secret-value",
    ] {
        assert!(!body.contains(private), "GitHub request leaked {private:?}");
    }
}

#[tokio::test]
async fn disposable_environment_requires_a_local_reconciler_before_github_write() {
    use crate::disposable_environment::DisposableEnvironmentRequest;

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;
    let request = CreateSessionRequest {
        environment: None,
        disposable_environment: Some(DisposableEnvironmentRequest {
            install: vec!["true".to_string()],
            variables: Default::default(),
            secrets: Default::default(),
        }),
        ..create_request()
    };
    let state = test_state(&server.uri(), None);
    let err = create_session(
        State(state),
        Path(("acme".to_string(), "site".to_string())),
        viewer_user(),
        auth_headers(),
        Json(request),
    )
    .await
    .expect_err("no private consumer");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}
