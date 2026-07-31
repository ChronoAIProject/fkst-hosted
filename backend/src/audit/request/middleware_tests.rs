//! Unit tests for the audit middleware, driven through a miniature router.
//!
//! The full cross-layer matrix (timeouts, the leader gate, `AppError`
//! conversion, exclusions, redaction canaries) lives in the router-level
//! integration suites; these cover the middleware's own contract.

use std::sync::Arc;

use axum::routing::get;
use axum::Router;
use tower::ServiceExt;
use utoipa::openapi::path::{HttpMethod, Operation, Paths};
use utoipa::openapi::{Info, OpenApi};

use super::*;
use crate::audit::request::id::is_acceptable;
use crate::audit::sink::RecordingSink;
use crate::audit::{validate, ApiRequestCompletedV1, AuditHandle, AuditMetrics};
use axum::http::{Method, StatusCode};

/// A document declaring exactly one audited operation and one excluded one.
fn document() -> OpenApi {
    let mut paths = Paths::new();
    for (method, path, operation_id) in [
        (HttpMethod::Get, "/api/v1/overview", "canvas_overview"),
        (HttpMethod::Get, "/health", "health"),
    ] {
        let mut operation = Operation::new();
        operation.operation_id = Some(operation_id.to_string());
        paths.add_path_operation(path, vec![method], operation);
    }
    let mut document = OpenApi::new(Info::new("test", "0"), Paths::new());
    document.paths = paths;
    document
}

fn app(capacity: usize) -> (Router, RecordingSink) {
    let sink = RecordingSink::new(capacity);
    let handle = AuditHandle::new(Arc::new(sink.clone()), AuditMetrics::new());
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::from_openapi(&document()).expect("catalog builds")),
        handle,
        crate::audit::event::ServiceIdentity {
            version: "9.9.9".to_string(),
            environment: "test".to_string(),
        },
    );
    let router = Router::new()
        .route("/api/v1/overview", get(|| async { "ok" }))
        .route("/health", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            middleware,
            audit_requests,
        ));
    (router, sink)
}

async fn call(router: &Router, request: axum::http::Request<axum::body::Body>) -> Response {
    router
        .clone()
        .oneshot(request)
        .await
        .expect("router responds")
}

fn get_request(path: &str) -> axum::http::Request<axum::body::Body> {
    axum::http::Request::get(path)
        .body(axum::body::Body::empty())
        .expect("request builds")
}

fn only_event(sink: &RecordingSink) -> ApiRequestCompletedV1 {
    let events = sink.events();
    assert_eq!(events.len(), 1, "exactly one event per request");
    events.into_iter().next().expect("one event")
}

#[tokio::test]
async fn an_audited_request_produces_one_valid_terminal_record() {
    let (router, sink) = app(8);
    let response = call(&router, get_request("/api/v1/overview")).await;
    assert_eq!(response.status(), StatusCode::OK);

    let event = only_event(&sink);
    validate::validate(&event).expect("the middleware must build a contract-valid record");
    assert_eq!(event.operation_id, "canvas_overview");
    assert_eq!(event.route_template, "/api/v1/overview");
    assert_eq!(event.method, "GET");
    assert_eq!(event.status_code, Some(200));
    assert_eq!(event.outcome, crate::audit::AuditOutcome::Success);
    assert_eq!(event.error_code, None);
    assert_eq!(event.actor.kind, crate::audit::ActorKind::Anonymous);
    assert_eq!(event.service.version, "9.9.9");
    assert!(event.completed_at >= event.started_at, "monotonic duration");
}

#[tokio::test]
async fn an_excluded_operation_produces_no_record_but_still_answers() {
    let (router, sink) = app(8);
    let response = call(&router, get_request("/health")).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(sink.is_empty(), "probe traffic must never be recorded");
}

#[tokio::test]
async fn an_unmatched_path_is_recorded_without_its_raw_path_or_query() {
    let (router, sink) = app(8);
    let response = call(
        &router,
        get_request("/api/v1/auth/github/callback?code=secret-code&state=secret-state"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let event = only_event(&sink);
    validate::validate(&event).expect("valid record");
    assert_eq!(event.operation_id, "<unmatched>");
    assert_eq!(event.route_template, "<unmatched>");
    assert_eq!(event.error_code.as_deref(), Some("route_not_found"));
    let rendered = format!("{event:?}");
    for canary in ["secret-code", "secret-state", "github/callback"] {
        assert!(
            !rendered.contains(canary),
            "{canary} leaked into {rendered}"
        );
    }
}

#[tokio::test]
async fn a_well_formed_client_request_id_is_accepted_and_echoed() {
    let (router, sink) = app(8);
    let request = axum::http::Request::get("/api/v1/overview")
        .header("x-request-id", "client-req-1")
        .body(axum::body::Body::empty())
        .expect("request builds");
    let response = call(&router, request).await;
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("client-req-1")
    );
    assert_eq!(only_event(&sink).request_id, "client-req-1");
}

#[tokio::test]
async fn a_hostile_client_request_id_is_replaced_everywhere() {
    let (router, sink) = app(8);
    let request = axum::http::Request::get("/api/v1/overview")
        .header("x-request-id", "spoof me\tplease")
        .body(axum::body::Body::empty())
        .expect("request builds");
    let response = call(&router, request).await;
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("a normalized id is always echoed")
        .to_string();
    assert_ne!(echoed, "spoof me\tplease");
    assert!(is_acceptable(&echoed));
    assert_eq!(only_event(&sink).request_id, echoed);
}

/// A request id is client-controlled and reusable; the delivery/dedupe id is not.
#[tokio::test]
async fn two_requests_sharing_a_request_id_still_get_distinct_event_ids() {
    let (router, sink) = app(8);
    for _ in 0..2 {
        let request = axum::http::Request::get("/api/v1/overview")
            .header("x-request-id", "reused")
            .body(axum::body::Body::empty())
            .expect("request builds");
        let response = call(&router, request).await;
        assert_eq!(response.status(), StatusCode::OK);
    }
    let events = sink.events();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].request_id, events[1].request_id);
    assert_ne!(
        events[0].event_id, events[1].event_id,
        "a reused request id must not collapse two records into one"
    );
}

/// Audit backpressure is never allowed to become a product failure.
#[tokio::test]
async fn a_full_sink_never_rewrites_a_completed_response() {
    let (router, sink) = app(1);
    for _ in 0..3 {
        let response = call(&router, get_request("/api/v1/overview")).await;
        assert_eq!(
            response.status(),
            StatusCode::OK,
            "a dropped audit event must not change the business response"
        );
    }
    assert_eq!(sink.len(), 1, "the bounded sink kept only what it could");
}

#[tokio::test]
async fn a_cors_preflight_is_excluded() {
    let (router, sink) = app(8);
    let request = axum::http::Request::builder()
        .method(Method::OPTIONS)
        .uri("/api/v1/overview")
        .body(axum::body::Body::empty())
        .expect("request builds");
    let _response = call(&router, request).await;
    assert!(sink.is_empty(), "preflights must never be recorded");
}
