//! Integration tests for the built router, driven via `tower::ServiceExt::oneshot`
//! (no real TCP bind, no Docker, no datastore).
//!
//! The controller is datastore-free (#143): `/health` and `/api/v1/health`
//! report ready unconditionally, since a process that can answer the route is
//! healthy. These tests assert the exact `200 ok` wire contract and that the
//! routes are public.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::auth::AuthMode;
use fkst_control_plane::authz::Authorizer;
use fkst_control_plane::config::Config;
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn test_router() -> axum::Router {
    build_router(AppState {
        binding_store: fkst_control_plane::nyxid_connect::BrokerBindingStore::new(),
        config: Config::default(),
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        github_app_webhook_secret: None,
    })
    .expect("router")
}

async fn assert_ready(path: &str) {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        test_router().oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        ),
    )
    .await
    .expect("health must answer within 2s, not hang")
    .expect("router must respond");

    assert_eq!(response.status(), StatusCode::OK);

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

    // Exact wire contract, including field order: status, version. The
    // datastore-free controller dropped the `mongo` field (#143).
    let expected = format!(
        r#"{{"status":"ok","version":"{}"}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(std::str::from_utf8(&body).unwrap(), expected);
}

#[tokio::test]
async fn health_returns_200_ok_with_no_datastore() {
    assert_ready("/health").await;
}

#[tokio::test]
async fn api_v1_health_returns_200_ok_with_no_datastore() {
    assert_ready("/api/v1/health").await;
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        test_router().oneshot(
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
