//! Integration tests for the built router, driven via `tower::ServiceExt::oneshot`
//! (no real TCP bind).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_hosted_api::config::Config;
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn test_router() -> axum::Router {
    build_router(AppState {
        config: Config::default(),
    })
}

#[tokio::test]
async fn health_returns_ok_envelope_with_request_id() {
    let response = test_router()
        .oneshot(Request::get("/health").body(Body::empty()).unwrap())
        .await
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

    // Exact wire contract, including field order: status, version, checks.
    let expected = format!(
        r#"{{"status":"ok","version":"{}","checks":{{"mongo":"unknown"}}}}"#,
        env!("CARGO_PKG_VERSION")
    );
    assert_eq!(std::str::from_utf8(&body).unwrap(), expected);

    let json: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(json["status"], "ok");
    assert_eq!(json["version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(json["checks"]["mongo"], "unknown");
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let response = test_router()
        .oneshot(Request::get("/does-not-exist").body(Body::empty()).unwrap())
        .await
        .expect("router must respond");

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}
