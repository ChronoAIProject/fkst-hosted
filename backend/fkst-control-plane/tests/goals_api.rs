//! Goals HTTP API integration tests driven via `tower::ServiceExt::oneshot`
//! against the REAL `build_router(AppState)` — the full middleware stack, no
//! mock layer.
//!
//! The controller is datastore-free (#143): goals are GitHub-Issue + in-memory
//! backed (`GoalIssueStore`), so these tests need no Docker and no database
//! container. Every test runs unconditionally.

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::{GoalDoc, GoalIssueStore, GoalStatus, RepoRef};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tower::ServiceExt;

mod support;

/// Build the real application router over the datastore-free controller.
struct TestApp {
    router: axum::Router,
    goals: GoalIssueStore,
}

async fn app() -> TestApp {
    let config = Config::default();
    // Goals are GitHub-Issue + in-memory backed (#137); no datastore needed.
    let goals = GoalIssueStore::new(None);
    // Package validation is now format-only (#115); no PackageRepository needed.
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let vault = support::test_vault();
    let router = build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals: goals.clone(),
        vault,
        ornn: None,
    })
    .expect("router");
    TestApp { router, goals }
}

/// Drain a response into (status, headers, raw body string).
async fn drain(response: axum::response::Response) -> (StatusCode, HeaderMap, String) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
    (status, headers, body)
}

/// POST a JSON value to /api/v1/goals.
async fn post_goal(router: &axum::Router, body: &Value) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/goals")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// GET an arbitrary path.
async fn get_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// PATCH a JSON value to an arbitrary path.
async fn patch_path(
    router: &axum::Router,
    path: &str,
    body: &Value,
) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::patch(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// DELETE an arbitrary path.
async fn delete_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::delete(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// Seed a goal directly into the in-memory goal store with a given status.
async fn seed_goal_with_status(goals: &GoalIssueStore, status: GoalStatus) -> bson::Uuid {
    let id = bson::Uuid::new();
    let now = bson::DateTime::now();
    let goal = GoalDoc {
        id,
        title: "Seeded goal".to_string(),
        description: "Seeded description".to_string(),
        package_names: vec!["test-pkg".to_string()],
        repo: None,
        status,
        owner_user_id: "dev-local".to_string(),
        org_id: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };
    goals.insert(&goal).await.expect("seed goal");
    id
}

// ---- Tests ---------------------------------------------------------------

#[tokio::test]
async fn post_goal_creates_201_with_location_and_not_started() {
    let app = app().await;
    // Package names are validated for FORMAT only (#115); no store seeding needed.
    let body = json!({
        "title": "Build a billing pipeline",
        "description": "Process all invoices",
        "package_names": ["my-pkg"],
    });

    let (status, headers, response_body) = post_goal(&app.router, &body).await;
    assert_eq!(status, StatusCode::CREATED);

    // Location header present.
    let location = headers
        .get(header::LOCATION)
        .expect("Location header")
        .to_str()
        .unwrap();
    assert!(location.starts_with("/api/v1/goals/"));

    let goal: Value = serde_json::from_str(&response_body).expect("parse goal");
    assert_eq!(goal["status"], "not_started");
    assert_eq!(goal["title"], "Build a billing pipeline");
    assert_eq!(goal["package_names"], json!(["my-pkg"]));
    assert!(goal["repo"].is_null());
    assert!(goal["org_id"].is_null());
    assert!(goal["active_session_id"].is_null());
    assert_eq!(goal["owner_user_id"], "dev-local");

    // GET the goal by its ID.
    let goal_id = goal["id"].as_str().expect("id");
    let (get_status, _, get_body) =
        get_path(&app.router, &format!("/api/v1/goals/{goal_id}")).await;
    assert_eq!(get_status, StatusCode::OK);
    let fetched: Value = serde_json::from_str(&get_body).expect("parse goal");
    assert_eq!(fetched["id"], goal_id);
    assert_eq!(fetched["status"], "not_started");
}

// post_goal_with_inaccessible_package_is_403 was deleted: the store-existence
// pre-check and can_use_package authorization were removed by #115. Package
// names are now validated for FORMAT only; existence is resolved at session
// spawn against the repo's .fkst/packages/ tree, not a store. A well-formed
// package name always passes goal creation, so the 400/403 assertion no longer
// holds and its premise (the store lookup) is gone.

#[tokio::test]
async fn list_scopes_to_own_plus_org_and_filters_by_status() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create two goals.
    let body1 = json!({
        "title": "Goal one",
        "description": "First goal",
        "package_names": ["list-pkg"],
    });
    let body2 = json!({
        "title": "Goal two",
        "description": "Second goal",
        "package_names": ["list-pkg"],
    });
    let _ = post_goal(&app.router, &body1).await;
    let _ = post_goal(&app.router, &body2).await;

    // List all goals.
    let (status, _, list_body) = get_path(&app.router, "/api/v1/goals").await;
    assert_eq!(status, StatusCode::OK);
    let goals: Vec<Value> = serde_json::from_str(&list_body).expect("parse goals");
    assert!(goals.len() >= 2);

    // All goals should have status not_started.
    for g in &goals {
        assert_eq!(g["status"], "not_started");
    }

    // Filter by status=not_started.
    let (status, _, filtered_body) =
        get_path(&app.router, "/api/v1/goals?status=not_started").await;
    assert_eq!(status, StatusCode::OK);
    let filtered: Vec<Value> = serde_json::from_str(&filtered_body).expect("parse filtered");
    assert_eq!(filtered.len(), goals.len());

    // Filter by status=running (should return empty).
    let (status, _, empty_body) = get_path(&app.router, "/api/v1/goals?status=running").await;
    assert_eq!(status, StatusCode::OK);
    let empty: Vec<Value> = serde_json::from_str(&empty_body).expect("parse empty");
    assert!(empty.is_empty());
}

#[tokio::test]
async fn patch_updates_title_anytime_but_packages_conflict_when_running() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create a goal.
    let body = json!({
        "title": "Original title",
        "description": "Original desc",
        "package_names": ["patch-pkg"],
    });
    let (_, _, create_body) = post_goal(&app.router, &body).await;
    let goal: Value = serde_json::from_str(&create_body).expect("parse");
    let goal_id = goal["id"].as_str().expect("id");

    // Patch title on not_started goal — should succeed.
    let patch = json!({"title": "Updated title"});
    let (status, _, patch_body) =
        patch_path(&app.router, &format!("/api/v1/goals/{goal_id}"), &patch).await;
    assert_eq!(status, StatusCode::OK);
    let patched: Value = serde_json::from_str(&patch_body).expect("parse");
    assert_eq!(patched["title"], "Updated title");

    // Seed a goal in running status.
    let running_id = seed_goal_with_status(&app.goals, GoalStatus::Running).await;

    // Patch title on running goal — title should be editable in any status.
    let patch = json!({"title": "Title update while running"});
    let (status, _, _) =
        patch_path(&app.router, &format!("/api/v1/goals/{running_id}"), &patch).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "title patch on running goal should succeed"
    );

    // Patch packages on running goal — should be 409.
    // Package name "other-pkg" is format-valid; no store entry needed (#115).
    let patch = json!({"package_names": ["other-pkg"]});
    let (status, _, conflict_body) =
        patch_path(&app.router, &format!("/api/v1/goals/{running_id}"), &patch).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "packages patch on running goal must be 409"
    );
    let err: Value = serde_json::from_str(&conflict_body).expect("parse");
    assert_eq!(err["error"], "conflict");
}

#[tokio::test]
async fn patch_repo_and_clear_repo_mutually_exclusive() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create a goal.
    let body = json!({
        "title": "Repo goal",
        "description": "desc",
        "package_names": ["repo-pkg"],
    });
    let (_, _, create_body) = post_goal(&app.router, &body).await;
    let goal: Value = serde_json::from_str(&create_body).expect("parse");
    let goal_id = goal["id"].as_str().expect("id");

    // Send both repo and clear_repo.
    let patch = json!({
        "repo": {"owner": "acme", "name": "repo"},
        "clear_repo": true,
    });
    let (status, _, err_body) =
        patch_path(&app.router, &format!("/api/v1/goals/{goal_id}"), &patch).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let err: Value = serde_json::from_str(&err_body).expect("parse");
    assert!(
        err["message"]
            .as_str()
            .unwrap()
            .contains("mutually exclusive"),
        "expected mutually exclusive error: {:?}",
        err
    );
}

#[tokio::test]
async fn delete_not_started_204_then_404() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create a goal.
    let body = json!({
        "title": "Delete me",
        "description": "desc",
        "package_names": ["del-pkg"],
    });
    let (_, _, create_body) = post_goal(&app.router, &body).await;
    let goal: Value = serde_json::from_str(&create_body).expect("parse");
    let goal_id = goal["id"].as_str().expect("id");

    // Delete it.
    let (status, _, _) = delete_path(&app.router, &format!("/api/v1/goals/{goal_id}")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // GET should now be 404.
    let (status, _, _) = get_path(&app.router, &format!("/api/v1/goals/{goal_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);

    // DELETE again should be 404.
    let (status, _, _) = delete_path(&app.router, &format!("/api/v1/goals/{goal_id}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn delete_running_is_409() {
    let app = app().await;

    let running_id = seed_goal_with_status(&app.goals, GoalStatus::Running).await;

    let (status, _, err_body) =
        delete_path(&app.router, &format!("/api/v1/goals/{running_id}")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let err: Value = serde_json::from_str(&err_body).expect("parse");
    assert_eq!(err["error"], "conflict");
}

#[tokio::test]
async fn get_malformed_uuid_is_400() {
    let app = app().await;

    let (status, _, body) = get_path(&app.router, "/api/v1/goals/not-a-uuid").await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let err: Value = serde_json::from_str(&body).expect("parse");
    assert_eq!(err["error"], "invalid_request");
    assert!(
        err["message"].as_str().unwrap().contains("invalid goal id"),
        "expected invalid goal id: {:?}",
        err
    );
}

#[tokio::test]
async fn goal_doc_round_trips_and_statuses_serialize_snake_case() {
    // Pure serde behavior; no router or datastore involved.
    use bson::Bson;

    let cases = [
        (GoalStatus::NotStarted, "not_started"),
        (GoalStatus::Triggered, "triggered"),
        (GoalStatus::Running, "running"),
        (GoalStatus::Stopped, "stopped"),
        (GoalStatus::Failed, "failed"),
    ];
    for (status, expected) in cases {
        let bson = bson::to_bson(&status).expect("serialize");
        assert_eq!(bson, Bson::String(expected.to_string()));
    }

    let now = bson::DateTime::now();
    let doc = GoalDoc {
        id: bson::Uuid::new(),
        title: "Test".to_string(),
        description: "desc".to_string(),
        package_names: vec!["p".to_string()],
        repo: None,
        status: GoalStatus::NotStarted,
        owner_user_id: "u".to_string(),
        org_id: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };
    let raw = bson::to_document(&doc).expect("serialize");

    // Explicit nulls.
    assert_eq!(raw.get("repo"), Some(&Bson::Null));
    assert_eq!(raw.get("org_id"), Some(&Bson::Null));
    assert_eq!(raw.get("active_session_id"), Some(&Bson::Null));

    let back: GoalDoc = bson::from_document(raw).expect("deserialize");
    assert_eq!(back, doc);
}

#[tokio::test]
async fn validation_matrix_rejects_invalid_goals() {
    // Pure validation function; no router or datastore involved.
    use fkst_control_plane::goals::validate_goal_fields;

    // Empty title.
    let err = validate_goal_fields("", "desc", &["p".to_string()], None).expect_err("empty title");
    assert!(err.starts_with("empty title"), "got: {err}");

    // Oversize description.
    let long = "x".repeat(16_385);
    let err =
        validate_goal_fields("title", &long, &["p".to_string()], None).expect_err("oversize desc");
    assert!(err.starts_with("description too large"), "got: {err}");

    // Zero packages.
    let err = validate_goal_fields("title", "desc", &[], None).expect_err("0 packages");
    assert!(err.starts_with("at least one package"), "got: {err}");

    // 17 packages.
    let pkgs: Vec<String> = (0..17).map(|i| format!("p{i}")).collect();
    let err = validate_goal_fields("title", "desc", &pkgs, None).expect_err("17 packages");
    assert!(err.starts_with("too many packages"), "got: {err}");

    // Duplicate package.
    let err = validate_goal_fields("title", "desc", &["a".to_string(), "a".to_string()], None)
        .expect_err("dup package");
    assert!(err.starts_with("duplicate package name"), "got: {err}");

    // Bad repo owner.
    let err = validate_goal_fields(
        "title",
        "desc",
        &["p".to_string()],
        Some(&RepoRef {
            owner: "-bad".to_string(),
            name: "ok".to_string(),
        }),
    )
    .expect_err("bad owner");
    assert!(err.starts_with("invalid repo owner"), "got: {err}");

    // Bad repo name.
    let err = validate_goal_fields(
        "title",
        "desc",
        &["p".to_string()],
        Some(&RepoRef {
            owner: "acme".to_string(),
            name: "".to_string(),
        }),
    )
    .expect_err("bad name");
    assert!(err.starts_with("invalid repo name"), "got: {err}");

    // Unknown field (serde deny_unknown_fields tested at route level).
    let json = r#"{"title":"t","description":"d","package_names":["p"],"bogus":1}"#;
    let result = serde_json::from_str::<fkst_control_plane::routes::goals::CreateGoalRequest>(json);
    assert!(result.is_err(), "unknown field must be rejected");
}

#[tokio::test]
async fn post_goal_with_repo_sets_repo() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    let body = json!({
        "title": "Goal with repo",
        "description": "desc",
        "package_names": ["repo-test-pkg"],
        "repo": {"owner": "acme", "name": "billing-repo"},
    });

    let (status, _, response_body) = post_goal(&app.router, &body).await;
    assert_eq!(status, StatusCode::CREATED);
    let goal: Value = serde_json::from_str(&response_body).expect("parse");
    assert_eq!(goal["repo"]["owner"], "acme");
    assert_eq!(goal["repo"]["name"], "billing-repo");
}

#[tokio::test]
async fn patch_clear_repo_works() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create with repo.
    let body = json!({
        "title": "Goal with repo",
        "description": "desc",
        "package_names": ["clear-pkg"],
        "repo": {"owner": "acme", "name": "my-repo"},
    });
    let (_, _, create_body) = post_goal(&app.router, &body).await;
    let goal: Value = serde_json::from_str(&create_body).expect("parse");
    let goal_id = goal["id"].as_str().expect("id");

    // Clear the repo.
    let patch = json!({"clear_repo": true});
    let (status, _, patch_body) =
        patch_path(&app.router, &format!("/api/v1/goals/{goal_id}"), &patch).await;
    assert_eq!(status, StatusCode::OK);
    let patched: Value = serde_json::from_str(&patch_body).expect("parse");
    assert!(patched["repo"].is_null(), "repo must be null after clear");
}

#[tokio::test]
async fn pagination_limit_and_offset() {
    let app = app().await;
    // Package names validated by format only; no store seeding required.

    // Create 3 goals.
    for i in 0..3 {
        let body = json!({
            "title": format!("Goal {i}"),
            "description": format!("Description {i}"),
            "package_names": ["page-pkg"],
        });
        post_goal(&app.router, &body).await;
    }

    // List with limit=2.
    let (status, _, list_body) = get_path(&app.router, "/api/v1/goals?limit=2").await;
    assert_eq!(status, StatusCode::OK);
    let goals: Vec<Value> = serde_json::from_str(&list_body).expect("parse");
    assert_eq!(goals.len(), 2);

    // List with limit=200 (max).
    let (status, _, list_body) = get_path(&app.router, "/api/v1/goals?limit=200").await;
    assert_eq!(status, StatusCode::OK);
    let goals: Vec<Value> = serde_json::from_str(&list_body).expect("parse");
    assert!(goals.len() >= 3);
}
// (Removed `goals_indexes_are_idempotent`: goals are GitHub-Issue + in-memory
// backed now (#137) — there is no goals collection / index step.)
