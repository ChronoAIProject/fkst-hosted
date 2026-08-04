//! `/health`, `/ready`, and `/metrics`: liveness never depends on a dependency,
//! readiness is durable ingress, and the exposition carries no identifier.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::audit_relay::http::tests::{call, get_request};
use crate::audit_relay::test_support::{relay, READ_TOKEN, WRITE_TOKEN};

async fn text(router: &axum::Router, uri: &str) -> (StatusCode, String) {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .uri(uri)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body collects")
        .to_bytes();
    (status, String::from_utf8_lossy(&bytes).to_string())
}

#[tokio::test]
async fn health_and_the_ops_endpoints_need_no_credentials() {
    let (_dir, _state, router) = relay();
    let (status, body) = call(&router, get_request("/health", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "ok");
}

#[tokio::test]
async fn readiness_is_true_while_durable_ingress_is_possible() {
    let (_dir, _state, router) = relay();
    let (status, body) = call(&router, get_request("/ready", None)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["ready"], true);
    assert_eq!(body["storage_ready"], true);
}

#[tokio::test]
async fn readiness_is_false_at_capacity() {
    let (_dir, state, router) = relay();
    state
        .at_capacity
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (status, body) = call(&router, get_request("/ready", None)).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["ready"], false);
    assert_eq!(body["at_capacity"], true);
}

#[tokio::test]
async fn the_exposition_is_closed_label_and_credential_free() {
    let (_dir, _state, router) = relay();
    let (status, body) = text(&router, "/metrics").await;
    assert_eq!(status, StatusCode::OK);
    for family in [
        "fkst_audit_relay_records",
        "fkst_audit_relay_oldest_record_age_seconds",
        "fkst_audit_relay_db_bytes",
        "fkst_audit_relay_ingress_total",
        "fkst_audit_relay_capture_total",
        "fkst_audit_relay_verification_total",
        "fkst_audit_relay_dead_letters_total",
        "fkst_audit_relay_incomplete_total",
    ] {
        assert!(body.contains(family), "`{family}` must be exposed");
    }
    for canary in [WRITE_TOKEN, READ_TOKEN] {
        assert!(
            !body.contains(canary),
            "`{canary}` must never reach the exposition"
        );
    }
    // No high-cardinality label may exist at all.
    for forbidden in ["event_id=", "actor_id=", "session_id=", "request_id="] {
        assert!(
            !body.contains(forbidden),
            "`{forbidden}` must never be a metric label"
        );
    }
}
