//! Unit tests for the audited extractors.
//!
//! Each case drives a real `axum::Router`, because the properties under test are
//! the extractor's REJECTION behaviour and the extension it wrote — neither of
//! which a direct `from_request` call reproduces faithfully.

use super::*;
use crate::audit::event::ArgumentsParseStatus;
use crate::audit::request::context::FrozenRequestContext;
use crate::audit::request::response::error_code_of;
use crate::audit::request::AuditRequestContext;
use axum::body::Body;
use axum::extract::DefaultBodyLimit;
use axum::http::Request;
use axum::routing::{get, post};
use axum::Router;
use serde::Deserialize;
use std::sync::{Arc, Mutex};
use tower::ServiceExt;

#[derive(Debug, Deserialize)]
struct Payload {
    value: u32,
}

#[derive(Debug, Deserialize)]
struct Selector {
    limit: u32,
}

/// A router that installs an audit context, runs the extractor under test, and
/// hands the frozen context back to the test.
type Captured = Arc<Mutex<Option<FrozenRequestContext>>>;

fn capture(captured: &Captured, context: &AuditRequestContext) {
    *captured.lock().unwrap_or_else(|e| e.into_inner()) = Some(context.freeze());
}

/// Install the context, dispatch, and freeze — the same order the real outer
/// middleware uses.
fn app(captured: Captured, inner: Router) -> Router {
    inner.layer(axum::middleware::from_fn(
        move |mut request: Request<Body>, next: axum::middleware::Next| {
            let captured = captured.clone();
            async move {
                let context = AuditRequestContext::new();
                context.install(request.extensions_mut());
                let response = next.run(request).await;
                capture(&captured, &context);
                response
            }
        },
    ))
}

fn frozen(captured: &Captured) -> FrozenRequestContext {
    captured
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
        .expect("the middleware froze a context")
}

async fn json_call(request: Request<Body>) -> (axum::response::Response, FrozenRequestContext) {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new().route(
            "/json",
            post(|AuditedJson(_): AuditedJson<Payload>| async { StatusCode::OK }),
        ),
    );
    let response = router.oneshot(request).await.expect("router responds");
    let frozen = frozen(&captured);
    (response, frozen)
}

fn json_request(content_type: &str, body: &'static str) -> Request<Body> {
    Request::post("/json")
        .header("content-type", content_type)
        .header("content-length", body.len().to_string())
        .body(Body::from(body))
        .expect("request builds")
}

#[tokio::test]
async fn a_valid_body_is_extracted_and_records_nothing_itself() {
    let (response, frozen) = json_call(json_request("application/json", r#"{"value":7}"#)).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(
        frozen.arguments.is_empty(),
        "a successful parse leaves the arguments to the handler's own DTO"
    );
}

/// The malformed-body contract: the normalized content type, the declared
/// length, the observed bounded size, and nothing else.
#[tokio::test]
async fn a_syntax_error_records_only_bounded_transport_metadata() {
    let body = r#"{"value": canary-not-json}"#;
    let (response, frozen) = json_call(json_request("application/json; charset=utf-8", body)).await;

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code_of(&response), Some(codes::INVALID_REQUEST));
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    assert_eq!(
        frozen
            .arguments
            .get("content_type")
            .and_then(|v| v.as_str()),
        Some("application/json")
    );
    assert_eq!(
        frozen
            .arguments
            .get("content_length_declared")
            .and_then(serde_json::Value::as_u64),
        Some(body.len() as u64)
    );
    assert_eq!(
        frozen
            .arguments
            .get("body_bytes_observed")
            .and_then(serde_json::Value::as_u64),
        Some(body.len() as u64)
    );
    let rendered = serde_json::to_string(&frozen.arguments).expect("serializes");
    assert!(!rendered.contains("canary-not-json"), "{rendered}");
    assert!(!rendered.contains("value"), "{rendered}");
}

/// A well-formed body that does not fit the schema is a DIFFERENT failure class
/// from a syntax error, and the stable code says so.
#[tokio::test]
async fn a_schema_mismatch_records_invalid_with_the_unprocessable_code() {
    let (response, frozen) =
        json_call(json_request("application/json", r#"{"value":"canary"}"#)).await;
    assert_eq!(response.status(), StatusCode::UNPROCESSABLE_ENTITY);
    assert_eq!(error_code_of(&response), Some(codes::UNPROCESSABLE));
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
}

#[tokio::test]
async fn a_missing_json_content_type_is_recorded_and_coded() {
    let (response, frozen) = json_call(json_request("text/plain", r#"{"value":1}"#)).await;
    assert_eq!(response.status(), StatusCode::UNSUPPORTED_MEDIA_TYPE);
    assert_eq!(
        error_code_of(&response),
        Some(codes::UNSUPPORTED_MEDIA_TYPE)
    );
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    assert_eq!(
        frozen
            .arguments
            .get("content_type")
            .and_then(|v| v.as_str()),
        Some("text/plain")
    );
}

/// An over-limit body is rejected before anything is buffered, so only the
/// DECLARED metadata survives — never a prefix of the bytes.
#[tokio::test]
async fn an_over_limit_body_records_only_its_declared_metadata() {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new()
            .route(
                "/json",
                post(|AuditedJson(_): AuditedJson<Payload>| async { StatusCode::OK }),
            )
            .layer(DefaultBodyLimit::max(8)),
    );
    let body = r#"{"value":123456789}"#;
    let response = router
        .oneshot(json_request("application/json", body))
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(error_code_of(&response), Some(codes::PAYLOAD_TOO_LARGE));
    let frozen = frozen(&captured);
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    assert_eq!(
        frozen
            .arguments
            .get("content_length_declared")
            .and_then(serde_json::Value::as_u64),
        Some(body.len() as u64)
    );
    assert!(
        !frozen.arguments.contains_key("body_bytes_observed"),
        "nothing was buffered, so nothing was observed"
    );
}

/// A query rejection knows only the query string, which is the one thing that
/// may never be recorded — so the record carries the status and nothing else.
#[tokio::test]
async fn a_query_rejection_records_invalid_without_the_query() {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new().route(
            "/thing",
            get(|AuditedQuery(_): AuditedQuery<Selector>| async { StatusCode::OK }),
        ),
    );
    let response = router
        .oneshot(
            Request::get("/thing?limit=canary-not-a-number")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code_of(&response), Some(codes::INVALID_REQUEST));
    let frozen = frozen(&captured);
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    let rendered = serde_json::to_string(&frozen.arguments).expect("serializes");
    assert!(!rendered.contains("canary-not-a-number"), "{rendered}");
    assert!(!rendered.contains("limit"), "{rendered}");
}

#[tokio::test]
async fn a_valid_query_is_extracted_unchanged() {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new().route(
            "/thing",
            get(
                |AuditedQuery(selector): AuditedQuery<Selector>| async move {
                    assert_eq!(selector.limit, 9);
                    StatusCode::OK
                },
            ),
        ),
    );
    let response = router
        .oneshot(
            Request::get("/thing?limit=9")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
}

/// A path segment that does not fit its type is `invalid`, and the raw segment
/// is never copied into the record.
#[tokio::test]
async fn a_path_rejection_records_invalid_without_the_segment() {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new().route(
            "/thing/:number",
            get(|AuditedPath(_): AuditedPath<u64>| async { StatusCode::OK }),
        ),
    );
    let response = router
        .oneshot(
            Request::get("/thing/canary-not-a-number")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(error_code_of(&response), Some(codes::INVALID_REQUEST));
    let frozen = frozen(&captured);
    assert_eq!(frozen.arguments_parse_status, ArgumentsParseStatus::Invalid);
    let rendered = serde_json::to_string(&frozen.arguments).expect("serializes");
    assert!(!rendered.contains("canary-not-a-number"), "{rendered}");
}

#[tokio::test]
async fn a_valid_path_is_extracted_unchanged() {
    let captured: Captured = Arc::default();
    let router = app(
        captured.clone(),
        Router::new().route(
            "/thing/:number",
            get(|AuditedPath(number): AuditedPath<u64>| async move {
                assert_eq!(number, 42);
                StatusCode::OK
            }),
        ),
    );
    let response = router
        .oneshot(
            Request::get("/thing/42")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
}

/// Without an installed context the extractors still behave exactly like the
/// axum ones — recording is best-effort, never a precondition.
#[tokio::test]
async fn the_extractors_work_without_an_audit_context() {
    let router = Router::new().route(
        "/json",
        post(|AuditedJson(payload): AuditedJson<Payload>| async move {
            assert_eq!(payload.value, 3);
            StatusCode::OK
        }),
    );
    let response = router
        .oneshot(json_request("application/json", r#"{"value":3}"#))
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::OK);
}

#[test]
fn every_body_rejection_status_maps_to_a_stable_bounded_code() {
    for (status, expected) in [
        (StatusCode::BAD_REQUEST, codes::INVALID_REQUEST),
        (StatusCode::UNPROCESSABLE_ENTITY, codes::UNPROCESSABLE),
        (StatusCode::PAYLOAD_TOO_LARGE, codes::PAYLOAD_TOO_LARGE),
        (
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            codes::UNSUPPORTED_MEDIA_TYPE,
        ),
        (StatusCode::IM_A_TEAPOT, codes::INVALID_REQUEST),
    ] {
        assert_eq!(body_rejection_code(status), expected);
    }
}
