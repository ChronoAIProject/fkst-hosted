//! Integration tests for the control-plane observability surface (#144):
//! `GET /api/v1/admin/state` and `GET /metrics`.
//!
//! Driven via `tower::ServiceExt::oneshot` — no real TCP bind, no Docker, no
//! datastore. The control plane is API-only: there is no claim authority and no
//! worker registry, so both routes report the in-memory SESSION store only. The
//! redaction test seeds a session and asserts no token/secret bytes reach the
//! response.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
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

/// Build a router wired with the supplied session service so the observability
/// routes reflect the real in-memory session store.
fn router_with(sessions: SessionService) -> axum::Router {
    let config = Config::default();
    let goals = GoalIssueStore::new(None);
    let vault = support::test_vault();
    build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config,
        sessions,
        // Disabled => the dev AuthContext carries `fkst:admin`, so the admin
        // route is reachable (the production gate is `fkst:admin`).
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
        goals,
        vault,
        ornn: None,
    })
    .expect("router")
}

/// A minimal goal-style `SessionDoc` seeded into the in-memory store. `owner` is
/// stamped so the redacted view reports `owner_present: true`.
fn seed_doc(owner: &str, status: SessionStatus) -> SessionDoc {
    SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status,
        pod_id: Some("worker-a".to_string()),
        fencing_token: Some(1),
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: Some(owner.to_string()),
        org_id: None,
        package_names: vec!["demo".to_string()],
        goal_id: Some(bson::Uuid::new()),
        repo: None,
        env_scope: None,
        triggered_by: Some("goal-trigger".to_string()),
        nyxid_key_id: None,
        nyxid_key_prefix: None,
        ornn_skills: None,
        terminal_cause: None,
        created_at: bson::DateTime::from_millis(1_700_000_000_000),
        started_at: None,
        stopped_at: None,
    }
}

async fn get(router: axum::Router, path: &str) -> (StatusCode, Vec<u8>) {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        router.oneshot(Request::get(path).body(Body::empty()).expect("request")),
    )
    .await
    .expect("route must answer within 2s")
    .expect("router responds");
    let status = response.status();
    let body = response.into_body().collect().await.unwrap().to_bytes();
    (status, body.to_vec())
}

#[tokio::test]
async fn admin_state_returns_json_shape() {
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let router = router_with(sessions);

    let (status, body) = get(router, "/api/v1/admin/state").await;
    assert_eq!(status, StatusCode::OK);
    let json: Value = serde_json::from_slice(&body).expect("admin state is JSON");
    // The API-only view carries a single `sessions` object (no claims/workers).
    assert!(json.get("sessions").is_some(), "missing `sessions` key");
    assert!(json["sessions"].is_object());
    assert!(json.get("claims").is_none(), "claims must be gone");
    assert!(json.get("workers").is_none(), "workers must be gone");
}

#[tokio::test]
async fn admin_state_never_serializes_secret_values() {
    // The admin view must report the session's presence (owner_present) WITHOUT
    // serializing any token/secret field.
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());

    let doc = seed_doc("owner-user-42", SessionStatus::Running);
    let session_id = doc.id;
    sessions.repo().insert(&doc).await.expect("seed session");

    let router = router_with(sessions);
    let (status, body) = get(router, "/api/v1/admin/state").await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body.clone()).expect("utf8 body");

    // (1) The presence boolean IS reported for the seeded session.
    let json: Value = serde_json::from_slice(&body).expect("JSON");
    let view = &json["sessions"][session_id.to_string()];
    assert_eq!(
        view["owner_present"], true,
        "owner presence must be reported as a boolean"
    );

    // (2) Presence-only redaction means no token/secret field is serialized.
    assert!(
        !text.contains("user_access_token") && !text.contains("token"),
        "no token field may appear in the admin view: {text}"
    );
}

#[tokio::test]
async fn metrics_renders_prometheus_text() {
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    let router = router_with(sessions);

    let (status, body) = get(router, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf8");
    assert!(
        text.contains("# TYPE fkst_sessions_total gauge"),
        "missing TYPE line in:\n{text}"
    );
    assert!(
        text.contains("fkst_sessions_total 0"),
        "sessions-total gauge must render a value in:\n{text}"
    );
    assert!(text.contains("# TYPE fkst_sessions_pending gauge"));
}

#[tokio::test]
async fn metrics_reflects_pending_count() {
    // Seed N pending sessions; the gauge must read exactly N.
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());
    const N: usize = 3;
    for _ in 0..N {
        sessions
            .repo()
            .insert(&seed_doc("owner-user-42", SessionStatus::Pending))
            .await
            .expect("seed pending session");
    }
    let router = router_with(sessions);

    let (status, body) = get(router, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    let text = String::from_utf8(body).expect("utf8");
    assert!(
        text.contains(&format!("fkst_sessions_pending {N}")),
        "pending gauge must equal {N} in:\n{text}"
    );
}

#[tokio::test]
async fn both_routes_answer_and_api_health_still_works() {
    // Nest-coexistence: the two routes answer 200 AND /api/v1/health (the
    // literal route alongside the /api/v1 nest) keeps answering.
    let sessions = SessionService::new(SessionRepo::new(), EngineConfig::default());

    for path in ["/metrics", "/api/v1/admin/state", "/api/v1/health"] {
        let router = router_with(sessions.clone());
        let (status, _) = get(router, path).await;
        assert_eq!(status, StatusCode::OK, "{path} must answer 200");
    }
}
