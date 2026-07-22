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
    auth_headers, grant_global_admin, test_state, viewer_user,
};

fn work_item_request() -> CreateWorkItemRequest {
    CreateWorkItemRequest {
        title: "  build the landing page  ".to_string(),
        label: None,
        body: Some("## Details\n\n  keep this indentation".to_string()),
    }
}

#[test]
fn create_work_item_request_accepts_the_legacy_work_label_alias() {
    let request: CreateWorkItemRequest = serde_json::from_value(serde_json::json!({
        "title": "task",
        "work_label": "site-build"
    }))
    .expect("legacy request remains readable during a rolling deploy");

    assert_eq!(request.label.as_deref(), Some("site-build"));
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
    let author_login = if author_id == viewer_user().id {
        viewer_user().login
    } else {
        "session-owner".to_string()
    };
    mount_trigger_with_identity(
        server,
        number,
        author_id,
        &author_login,
        &[],
        body,
        labels,
        is_pr,
    )
    .await;
}

#[allow(clippy::too_many_arguments)]
async fn mount_trigger_with_identity(
    server: &MockServer,
    number: i64,
    author_id: i64,
    author_login: &str,
    assignees: &[&str],
    body: &str,
    labels: &[&str],
    is_pr: bool,
) {
    let mut payload = serde_json::json!({
        "number": number,
        "body": body,
        "state": "open",
        "labels": labels.iter().map(|l| serde_json::json!({ "name": l })).collect::<Vec<_>>(),
        "user": { "id": author_id, "login": author_login },
        "assignees": assignees.iter().map(|login| serde_json::json!({ "login": login })).collect::<Vec<_>>(),
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
    // work item.
    mount_trigger(
        &server,
        21,
        viewer_user().id,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger", "fkst-substrate-active"],
        false,
    )
    .await;
    // The new work issue must carry the SESSION's resolved work label, the
    // trimmed title, and be created with the USER token (never an App token).
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({
            "title": "build the landing page",
            "body": "## Details\n\n  keep this indentation",
            "labels": ["site-build"],
            "assignees": ["shining"]
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
async fn create_work_item_routes_a_bot_authored_trigger_to_its_sole_assignee() {
    let server = MockServer::start().await;
    mount_trigger_with_identity(
        &server,
        21,
        700,
        "fkst-test[bot]",
        &["ShInInG"],
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(header("authorization", "Bearer user-token"))
        .and(body_partial_json(serde_json::json!({
            "labels": ["site-build"],
            "assignees": ["ShInInG"]
        })))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 81,
            "html_url": "https://github.com/acme/site/issues/81"
        })))
        .expect(1)
        .mount(&server)
        .await;
    let mut state = test_state(&server.uri(), None);
    state.config.reconcile.github_bot_login = Some("fkst-test".to_string());

    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect("the sole assignee is the effective creator");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 81);
}

#[test]
fn work_registration_preserves_creator_and_authored_branches_in_the_hash() {
    let spec = parse_trigger_issue_body(
        "### Session Name\n\nsite\n\n### Packages\n\nacme/pkgs@main:packages/devloop\n\n\
         ### Work Label\n\nsite-build\n\n### Source Branch\n\nrelease/v1\n\n\
         ### Target Branch\n\nfeature/site\n",
    )
    .expect("valid trigger");
    let expected_hash = config_hash(
        &spec.packages,
        spec.work_label.as_deref(),
        spec.environment.as_deref(),
        spec.output_lang.as_deref(),
        &spec.engine_config,
        &spec.manifest_refs,
        spec.source_branch.as_deref(),
        spec.target_branch.as_deref(),
    );
    let reg = work_registration(
        "acme",
        "site",
        21,
        700,
        "fkst-test[bot]".to_string(),
        SessionCreator {
            login: "seed-owner".to_string(),
            id: None,
        },
        spec,
    );

    assert_eq!(reg.trigger_author_login, "fkst-test[bot]");
    assert_eq!(reg.creator_login, "seed-owner");
    assert_eq!(reg.creator_id, None);
    assert_eq!(reg.def.source_branch.as_deref(), Some("release/v1"));
    assert_eq!(reg.def.target_branch.as_deref(), Some("feature/site"));
    assert_eq!(reg.config_hash, expected_hash);
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
            label: None,
            body: None,
        }),
    )
    .await
    .expect_err("a blank title is a 400");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_rejects_a_populated_but_blank_label_before_github() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(CreateWorkItemRequest {
            title: "task".to_string(),
            label: Some("   ".to_string()),
            body: None,
        }),
    )
    .await
    .expect_err("a present label must not be blank");
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
async fn create_work_item_refuses_a_session_without_any_resolved_work_label() {
    let server = MockServer::start().await;
    // A valid trigger with no explicit label whose package contributes no
    // discoverable labels. The viewer is the author, so authz passes and the
    // handler reaches the effective-label check.
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
    .expect_err("no resolved work label");
    assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_accepts_a_package_discovered_label() {
    let server = MockServer::start().await;
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
    Mock::given(method("GET"))
        .and(path("/repos/acme/pkgs/contents/packages/devloop/fkst.toml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[github]\nwork_labels = [\"fkst-dev\", \"fkst-security\"]\n"),
        )
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .and(body_partial_json(
            serde_json::json!({ "labels": ["fkst-security"] }),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "number": 80,
            "html_url": "https://github.com/acme/site/issues/80"
        })))
        .expect(1)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let mut request = work_item_request();
    request.label = Some("fkst-security".to_string());
    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(request),
    )
    .await
    .expect("a discovered work label is applicable");
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(created.issue_number, 80);
}

#[tokio::test]
async fn create_work_item_requires_a_choice_for_discovered_only_labels() {
    let server = MockServer::start().await;
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
    Mock::given(method("GET"))
        .and(path("/repos/acme/pkgs/contents/packages/devloop/fkst.toml"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("[github]\nwork_labels = [\"fkst-dev\", \"fkst-security\"]\n"),
        )
        .mount(&server)
        .await;
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
    .expect_err("discovered-only sessions require an explicit request choice");
    match err {
        AppError::Unprocessable(message) => {
            assert!(message.contains("no explicit work label"), "{message}");
            assert!(message.contains("fkst-dev"), "{message}");
            assert!(message.contains("fkst-security"), "{message}");
        }
        other => panic!("expected Unprocessable, got {other:?}"),
    }
}

#[tokio::test]
async fn create_work_item_rejects_a_label_outside_the_sessions_resolved_set() {
    let server = MockServer::start().await;
    mount_trigger(
        &server,
        21,
        viewer_user().id,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let mut request = work_item_request();
    request.label = Some("unrelated".to_string());
    let err = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(request),
    )
    .await
    .expect_err("an unrelated label is rejected before the write");
    match err {
        AppError::Unprocessable(message) => {
            assert!(message.contains("not applicable"), "got {message}");
            assert!(message.contains("site-build"), "got {message}");
        }
        other => panic!("expected unprocessable, got {other:?}"),
    }
}

#[tokio::test]
async fn create_work_item_refuses_a_closed_session() {
    let server = MockServer::start().await;
    let payload = serde_json::json!({
        "number": 21,
        "body": trigger_body("site-build"),
        "state": "closed",
        "labels": [{ "name": "fkst-substrate-trigger" }],
        "user": { "id": viewer_user().id, "login": viewer_user().login },
        "assignees": [],
    });
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues/21"))
        .respond_with(ResponseTemplate::new(200).set_body_json(payload))
        .mount(&server)
        .await;
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
    .expect_err("closed triggers cannot accept work");
    assert!(matches!(err, AppError::Conflict(_)), "got {err:?}");
}

#[tokio::test]
async fn create_work_item_rejects_repo_admin_without_an_explicit_authority_tier() {
    let server = MockServer::start().await;
    // Authored by someone else (id 100), with no explicit authority tier. Repo
    // administrator status is deliberately no longer sufficient.
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
    Mock::given(method("POST"))
        .and(path("/repos/acme/site/issues"))
        .respond_with(ResponseTemplate::new(201))
        .expect(0)
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), None);
    let error = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect_err("repo role alone does not grant work authority");
    assert!(matches!(error, AppError::Forbidden(_)));
}

#[tokio::test]
async fn create_work_item_allows_a_deployment_global_admin() {
    let server = MockServer::start().await;
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
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

    let mut state = test_state(&server.uri(), None);
    grant_global_admin(&mut state, "Shining");
    let (status, Json(created)) = create_work_item(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 21)),
        viewer_user(),
        auth_headers(),
        Json(work_item_request()),
    )
    .await
    .expect("a deployment global admin may queue work items");
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
    // a global admin — a stranger with mere write access has no work-item authority.
    mount_trigger(
        &server,
        21,
        100,
        &trigger_body("site-build"),
        &["fkst-substrate-trigger"],
        false,
    )
    .await;
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
