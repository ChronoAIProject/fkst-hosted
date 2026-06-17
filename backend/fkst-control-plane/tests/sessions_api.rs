//! Session HTTP API integration tests driven via `tower::ServiceExt::oneshot`
//! against the REAL `build_router(AppState)`.
//!
//! Since #143 the controller is datastore-free: sessions are backed by the
//! in-memory `SessionRepo`, so these tests need no Docker and no Mongo
//! container — they run unconditionally.
//!
//! Since #115 sessions are created ONLY via a goal trigger (a session loads its
//! packages from its goal repo's `.fkst/packages/`), so the classic
//! `POST /api/v1/sessions` create endpoint is gone. These tests cover the
//! surviving session surface that does not need the engine/clone path: the
//! `GET`/`stop` id-parsing + not-found edges, and the orphan sweep (which seeds
//! documents directly via the repository). The full goal→session lifecycle is
//! covered by the goal-trigger and runner suites.

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::engine::EngineConfig;
use fkst_control_plane::goals::GoalIssueStore;
use fkst_control_plane::models::{SessionDoc, SessionStatus};
use fkst_control_plane::router::build_router;
use fkst_control_plane::sessions::{SessionRepo, SessionService};
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

mod support;

/// Everything a test needs.
struct TestApp {
    router: axum::Router,
    /// The same in-memory session store the router's `SessionService` holds, so
    /// a test can seed/inspect the documents the API serves (the orphan sweep).
    /// With the db-free store two `SessionRepo::new()` calls are DISTINCT maps,
    /// so the handle must be cloned from the one the service was built with.
    repo: SessionRepo,
}

/// Build the real application router over the in-memory session store. No
/// engine is wired: these tests never start an engine process.
async fn app() -> TestApp {
    let config = Config::default();

    let goals = GoalIssueStore::new(None);
    // One in-memory store shared between the router's service and the test's
    // seed/inspect handle (cloning shares the Arc-backed map).
    let repo = SessionRepo::new();
    let sessions = SessionService::new(repo.clone(), EngineConfig::default());
    let vault = support::test_vault();
    let router = build_router(AppState {
        config,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router");
    TestApp { router, repo }
}

/// Drain a response into (status, headers, parsed JSON body or Null).
async fn drain(response: axum::response::Response) -> (StatusCode, HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, headers, body)
}

/// POST with an empty body (the stop endpoint).
async fn post_empty(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// GET a path.
async fn get_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
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

// ---- id parsing / lookup -----------------------------------------------------

#[tokio::test]
async fn malformed_and_unknown_ids_map_to_400_and_404() {
    let app = app().await;

    for path in [
        "/api/v1/sessions/not-a-uuid",
        "/api/v1/sessions/not-a-uuid/stop",
    ] {
        let (status, _headers, body) = if path.ends_with("/stop") {
            post_empty(&app.router, path).await
        } else {
            get_path(&app.router, path).await
        };
        assert_eq!(status, StatusCode::BAD_REQUEST, "path {path}");
        assert_eq!(body["error"], "invalid_request", "path {path}");
        assert_eq!(body["message"], "invalid session id: must be a UUID");
    }

    let ghost = "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12";
    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{ghost}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");

    let (status, _headers, body) =
        post_empty(&app.router, &format!("/api/v1/sessions/{ghost}/stop")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

// ---- classic create endpoint is gone (#115) ----------------------------------

#[tokio::test]
async fn classic_session_create_endpoint_is_removed_404() {
    let app = app().await;

    // `POST /api/v1/sessions` was removed: sessions are created via goal
    // trigger now. The route is absent, so the method is not allowed / not
    // found — never a 201.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/v1/sessions")
                .header("content-type", "application/json")
                .body(Body::from(r#"{"package_name":"demo"}"#))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    let status = response.status();
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::METHOD_NOT_ALLOWED,
        "classic create must not exist (got {status})"
    );
}

// ---- orphan sweep ------------------------------------------------------------

#[tokio::test]
async fn orphan_sweep_fails_only_pre_terminal_sessions_and_is_idempotent() {
    let app = app().await;
    let repo = app.repo.clone();

    let mk = |status: SessionStatus| SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status,
        pod_id: Some("dead-pod".to_string()),
        fencing_token: None,
        pid: Some(4242),
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: None,
        org_id: None,
        package_names: vec![],
        goal_id: None,
        repo: None,
        env_scope: None,
        triggered_by: None,
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        terminal_cause: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    };
    // Every pre-terminal status must be swept. The `stopping` row is the
    // durable backstop for a stop request that was acknowledged but whose
    // driver died (with the pod) before completing it.
    let pre_terminal = [
        mk(SessionStatus::Pending),
        mk(SessionStatus::Validating),
        mk(SessionStatus::Running),
        mk(SessionStatus::Stopping),
    ];
    let stopped = mk(SessionStatus::Stopped);
    let failed = mk(SessionStatus::Failed);
    for doc in &pre_terminal {
        repo.insert(doc).await.expect("insert pre-terminal");
    }
    repo.insert(&stopped).await.expect("insert stopped");
    repo.insert(&failed).await.expect("insert failed");

    let swept = repo.fail_orphans().await.expect("sweep");
    assert_eq!(swept, 4, "exactly the pre-terminal sessions are swept");

    for doc in &pre_terminal {
        let (status, _headers, body) =
            get_path(&app.router, &format!("/api/v1/sessions/{}", doc.id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "failed", "seeded {:?}: {body}", doc.status);
        assert_eq!(body["error"], "orphaned by pod restart");
        assert!(body["stopped_at"].as_str().is_some());
    }

    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{}", stopped.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "stopped", "terminal sessions untouched");

    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{}", failed.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "failed", "already-failed left alone");
    assert!(
        body["error"].is_null(),
        "an already-failed session must not be re-stamped: {body}"
    );

    let again = repo.fail_orphans().await.expect("second sweep");
    assert_eq!(again, 0, "sweep is idempotent");
}
