//! Integration tests for the built router, driven via `tower::ServiceExt::oneshot`
//! (no real TCP bind, no Docker, no datastore).
//!
//! `/health` is process liveness and preserves its exact wire contract. `/ready`
//! separately projects startup/full-resync recovery so dependency failures do not
//! cause Kubernetes to restart an otherwise-serving process.

use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use fkst_control_plane::config::Config;
use fkst_control_plane::recovery::{RecoveryMonitor, ResyncResult};
use fkst_control_plane::router::build_router;
use fkst_control_plane::state::AppState;
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

fn test_router(recovery: RecoveryMonitor) -> axum::Router {
    build_router(AppState {
        config: Config::default(),
        recovery,
        github_app: None,
        github_app_webhook_secret: None,
        reconciler: None,
        session_backend: None,
        storage: None,
        log_registry: Default::default(),
        log_bundle_cache: Default::default(),
    })
    .expect("router")
}

async fn get(recovery: RecoveryMonitor, path: &str) -> axum::response::Response {
    tokio::time::timeout(
        Duration::from_secs(2),
        test_router(recovery).oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        ),
    )
    .await
    .expect("system route must answer within 2s, not hang")
    .expect("router must respond")
}

async fn json_body(response: axum::response::Response) -> Value {
    let body = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&body).expect("JSON response")
}

#[tokio::test]
async fn health_preserves_the_exact_200_wire_contract() {
    // Enabled dispatch starts unready, but liveness is intentionally independent
    // of GitHub/recovery state.
    let recovery = RecoveryMonitor::new(true);
    recovery.record_attempt(ResyncResult::Failure, Duration::from_secs(1), 0);
    let response = get(recovery, "/health").await;

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
async fn readiness_is_immediate_when_dispatch_is_disabled() {
    let response = get(RecoveryMonitor::new(false), "/ready").await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        json_body(response).await,
        serde_json::json!({
            "status": "ready",
            "version": env!("CARGO_PKG_VERSION"),
            "startup_resync_complete": true,
        })
    );
}

#[tokio::test]
async fn readiness_transitions_from_recovering_to_ready_and_back_to_degraded() {
    let recovery = RecoveryMonitor::new(true);

    let initial = get(recovery.clone(), "/ready").await;
    assert_eq!(initial.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(json_body(initial).await["status"], "recovering");

    recovery.record_attempt(ResyncResult::Partial, Duration::from_millis(250), 3);
    let partial = get(recovery.clone(), "/ready").await;
    assert_eq!(partial.status(), StatusCode::SERVICE_UNAVAILABLE);
    let partial_body = json_body(partial).await;
    assert_eq!(partial_body["status"], "degraded");
    assert_eq!(partial_body["startup_resync_complete"], false);

    recovery.record_attempt(ResyncResult::Success, Duration::from_millis(125), 5);
    let complete = get(recovery.clone(), "/ready").await;
    assert_eq!(complete.status(), StatusCode::OK);
    let complete_body = json_body(complete).await;
    assert_eq!(complete_body["status"], "ready");
    assert_eq!(complete_body["startup_resync_complete"], true);

    recovery.record_attempt(ResyncResult::Failure, Duration::from_secs(1), 0);
    let later_failure = get(recovery, "/ready").await;
    assert_eq!(later_failure.status(), StatusCode::SERVICE_UNAVAILABLE);
    let failure_body = json_body(later_failure).await;
    assert_eq!(failure_body["status"], "degraded");
    assert_eq!(failure_body["startup_resync_complete"], true);
}

#[tokio::test]
async fn recovery_metrics_track_bounded_attempt_results() {
    let recovery = RecoveryMonitor::new(true);
    recovery.record_attempt(ResyncResult::Partial, Duration::from_millis(250), 3);
    recovery.record_attempt(ResyncResult::Success, Duration::from_millis(125), 5);

    let response = get(recovery, "/metrics").await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response.into_body().collect().await.unwrap().to_bytes();
    let body = std::str::from_utf8(&body).unwrap();
    for expected in [
        "fkst_startup_resync_attempts_total{result=\"success\"} 1",
        "fkst_startup_resync_attempts_total{result=\"partial\"} 1",
        "fkst_startup_resync_attempts_total{result=\"failure\"} 0",
        "fkst_startup_resync_complete 1",
        "fkst_recovery_ready 1",
        "fkst_startup_resync_last_repositories_enqueued 5",
    ] {
        assert!(
            body.contains(expected),
            "missing metric: {expected}\n{body}"
        );
    }
}

#[tokio::test]
async fn unknown_route_returns_404() {
    let response = tokio::time::timeout(
        Duration::from_secs(2),
        test_router(RecoveryMonitor::new(false)).oneshot(
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
