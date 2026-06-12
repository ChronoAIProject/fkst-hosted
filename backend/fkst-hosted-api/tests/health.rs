//! Integration tests for the built router, driven via `tower::ServiceExt::oneshot`
//! (no real TCP bind, no Docker).
//!
//! The Mongo handle points at an unreachable address with a short
//! server-selection timeout, so both health paths must answer `503 degraded`
//! quickly instead of hanging.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_hosted_api::auth::AuthMode;
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::packages::PackageRepository;
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Nothing listens on port 1; selection fails after ~200ms.
const UNREACHABLE_URI: &str = "mongodb://127.0.0.1:1";

async fn test_router() -> axum::Router {
    let config = Config {
        mongodb_uri: UNREACHABLE_URI.to_string(),
        mongodb_server_selection_timeout_ms: 200,
        ..Config::default()
    };
    let db = Db::from_config(&config)
        .await
        .expect("lazy handle must build without I/O");
    let packages = PackageRepository::new(&db.database);
    let sessions = SessionService::new(
        SessionRepo::new(&db),
        packages.clone(),
        EngineConfig::default(),
    );
    build_router(AppState {
        config,
        db,
        packages,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
    })
    .expect("router")
}

async fn assert_degraded(path: &str) {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        test_router().await.oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        ),
    )
    .await
    .expect("health must answer within 2s, not hang")
    .expect("router must respond");

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let content_type = response
        .headers()
        .get("content-type")
        .expect("content-type header present")
        .to_str()
        .unwrap();
    assert!(
        content_type.starts_with("application/json"),
        "unexpected content-type: {content_type}"
    );

    let request_id = response
        .headers()
        .get("x-request-id")
        .expect("x-request-id header present")
        .to_str()
        .unwrap();
    assert!(!request_id.is_empty(), "x-request-id must be non-empty");

    let body = response.into_body().collect().await.unwrap().to_bytes();

    // Exact wire contract, including field order: status, mongo, version.
    let expected = format!(
        r#"{{"status":"degraded","mongo":"down","version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(std::str::from_utf8(&body).unwrap(), expected);
}

#[tokio::test]
async fn health_returns_503_degraded_when_mongo_is_down() {
    assert_degraded("/health").await;
}

#[tokio::test]
async fn api_v1_health_returns_503_degraded_when_mongo_is_down() {
    assert_degraded("/api/v1/health").await;
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        test_router().await.oneshot(
            Request::get("/does-not-exist")
                .body(Body::empty())
                .expect("request builds"),
        ),
    )
    .await
    .expect("must answer within 2s")
    .expect("router must respond");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
