//! Terminal-outcome coverage for the audit middleware across the whole status
//! surface, plus the two behaviours the real router cannot easily exercise:
//! a route-scoped timeout firing, and a streaming response.
//!
//! These run against a purpose-built router rather than the product one because
//! the product surface cannot be made to return a `201`/`409`/`429`/`502` without
//! a live GitHub, while the middleware's contract is exactly that it classifies
//! *whatever* the inner service returned. The layer SHAPE mirrors
//! `build_router`: a route-scoped `TimeoutLayer` inside, the audit middleware
//! outermost.

use std::sync::Arc;
use std::time::Duration;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use fkst_control_plane::audit::sink::RecordingSink;
use fkst_control_plane::audit::{
    audit_requests, ApiRequestCompletedV1, AuditHandle, AuditMiddleware, AuditOutcome,
    OperationCatalog, ServiceIdentity,
};
use fkst_control_plane::error::AppError;
use tower::ServiceExt;
use tower_http::timeout::TimeoutLayer;

/// Deliberately shorter than the handler that must trip it.
const SHORT_TIMEOUT: Duration = Duration::from_millis(40);
/// Comfortably longer than every handler here.
const LONG_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(serde::Deserialize)]
struct Payload {
    #[allow(dead_code)]
    name: String,
}

fn harness() -> (Router, RecordingSink) {
    let (handle, sink) = AuditHandle::recording();
    // An EMPTY catalog: every route resolves as `<unmatched>` under its real
    // template, which is precisely the shape this suite wants — it isolates
    // outcome derivation from operation lookup (covered elsewhere).
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::default()),
        handle,
        ServiceIdentity {
            version: "9.9.9".to_string(),
            environment: "test".to_string(),
        },
    );

    let slow = Router::new()
        .route(
            "/slow",
            get(|| async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                "never observed"
            }),
        )
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            SHORT_TIMEOUT,
        ));

    let router = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .route(
            "/created",
            get(|| async { (StatusCode::CREATED, "made").into_response() }),
        )
        .route(
            "/empty",
            get(|| async { StatusCode::NO_CONTENT.into_response() }),
        )
        .route(
            "/redirect",
            get(|| async { axum::response::Redirect::to("/ok").into_response() }),
        )
        .route("/stream", get(stream_handler))
        .route("/json", post(|_: axum::Json<Payload>| async { "parsed" }))
        .route("/error/:code", get(error_handler))
        .merge(slow)
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            LONG_TIMEOUT,
        ))
        .layer(axum::middleware::from_fn_with_state(
            middleware,
            audit_requests,
        ));
    (router, sink)
}

/// A response whose body is produced lazily; the middleware must classify it
/// from the status alone, never by draining it.
async fn stream_handler() -> Response {
    let chunks = futures::stream::iter(
        (0..64).map(|index| Ok::<_, std::io::Error>(format!("chunk-{index}\n"))),
    );
    (StatusCode::OK, Body::from_stream(chunks)).into_response()
}

/// Map a path segment onto the representative `AppError` for that status.
async fn error_handler(axum::extract::Path(code): axum::extract::Path<u16>) -> Response {
    match code {
        400 => AppError::Validation("bad".to_string()),
        401 => AppError::Unauthorized("nope".to_string()),
        403 => AppError::Forbidden("nope".to_string()),
        404 => AppError::NotFound("gone".to_string()),
        409 => AppError::Conflict("clash".to_string()),
        422 => AppError::Unprocessable("semantics".to_string()),
        429 => AppError::RateLimited {
            message: "slow down".to_string(),
            retry_after_secs: 3,
        },
        502 => AppError::Upstream("upstream said no".to_string()),
        503 => AppError::Unavailable("dependency down".to_string()),
        _ => AppError::Internal(anyhow::anyhow!("boom")),
    }
    .into_response()
}

async fn call(router: &Router, request: Request<Body>) -> Response {
    router
        .clone()
        .oneshot(request)
        .await
        .expect("router responds")
}

fn only_event(sink: &RecordingSink) -> ApiRequestCompletedV1 {
    let events = sink.events();
    assert_eq!(events.len(), 1, "exactly one record per request");
    events.into_iter().next().expect("one event")
}

async fn record(path: &str) -> (StatusCode, ApiRequestCompletedV1) {
    let (router, sink) = harness();
    let response = call(
        &router,
        Request::get(path).body(Body::empty()).expect("request"),
    )
    .await;
    let status = response.status();
    (status, only_event(&sink))
}

#[tokio::test]
async fn successful_and_redirect_responses_are_classified_by_class() {
    for (path, status, outcome) in [
        ("/ok", StatusCode::OK, AuditOutcome::Success),
        ("/created", StatusCode::CREATED, AuditOutcome::Success),
        ("/empty", StatusCode::NO_CONTENT, AuditOutcome::Success),
        ("/redirect", StatusCode::SEE_OTHER, AuditOutcome::Redirect),
    ] {
        let (actual, event) = record(path).await;
        assert_eq!(actual, status, "{path}");
        assert_eq!(event.status_code, Some(status.as_u16()), "{path}");
        assert_eq!(event.outcome, outcome, "{path}");
        assert_eq!(event.error_code, None, "{path} must carry no error code");
        assert_eq!(event.route_template, path);
    }
}

#[tokio::test]
async fn every_app_error_status_is_classified_with_its_stable_code() {
    for (code, outcome, error_code) in [
        (400u16, AuditOutcome::ClientError, "invalid_request"),
        (401, AuditOutcome::Rejected, "unauthorized"),
        (403, AuditOutcome::Rejected, "forbidden"),
        (404, AuditOutcome::ClientError, "not_found"),
        (409, AuditOutcome::ClientError, "conflict"),
        (422, AuditOutcome::ClientError, "unprocessable"),
        (429, AuditOutcome::ClientError, "rate_limited"),
        (500, AuditOutcome::ServerError, "internal"),
        (502, AuditOutcome::ServerError, "upstream_error"),
        (503, AuditOutcome::ServerError, "unavailable"),
    ] {
        let (status, event) = record(&format!("/error/{code}")).await;
        assert_eq!(status.as_u16(), code);
        assert_eq!(event.status_code, Some(code));
        assert_eq!(event.outcome, outcome, "status {code}");
        assert_eq!(
            event.error_code.as_deref(),
            Some(error_code),
            "status {code}"
        );
        // The matched TEMPLATE, never the concrete path segment.
        assert_eq!(event.route_template, "/error/{code}");
    }
}

/// The record must never contain the human message, only the stable code.
#[tokio::test]
async fn an_error_message_never_reaches_the_record() {
    let (_, event) = record("/error/502").await;
    let rendered = format!("{event:#?}");
    assert!(!rendered.contains("upstream said no"), "{rendered}");
}

#[tokio::test]
async fn a_route_scoped_timeout_is_recorded_as_a_timeout() {
    let (status, event) = record("/slow").await;
    assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
    assert_eq!(event.status_code, Some(408));
    assert_eq!(event.outcome, AuditOutcome::Timeout);
    assert_eq!(event.error_code.as_deref(), Some("request_timeout"));
    assert_eq!(event.route_template, "/slow");
}

/// The long route-scoped timeout must not fire for a fast handler, and the
/// record must reflect the handler's answer, not the deadline.
#[tokio::test]
async fn a_long_timeout_leaves_a_fast_response_untouched() {
    let (status, event) = record("/ok").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(event.outcome, AuditOutcome::Success);
}

/// A body rejected by an extractor never reaches the handler; the record must
/// still exist, and must not contain the offending bytes.
#[tokio::test]
async fn malformed_json_rejected_before_the_handler_is_still_recorded() {
    let (router, sink) = harness();
    let response = call(
        &router,
        Request::post("/json")
            .header("content-type", "application/json")
            .body(Body::from("{not json at all: canary-body"))
            .expect("request"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);

    let event = only_event(&sink);
    assert_eq!(event.status_code, Some(400));
    assert_eq!(event.outcome, AuditOutcome::ClientError);
    assert_eq!(event.method, "POST");
    assert_eq!(event.route_template, "/json");
    assert!(!format!("{event:#?}").contains("canary-body"));
}

/// The middleware classifies a streaming response from its status and returns it
/// untouched — buffering it to inspect it would break log/blob downloads and the
/// chat SSE stream.
#[tokio::test]
async fn a_streaming_response_is_recorded_without_being_buffered() {
    let (router, sink) = harness();
    let response = call(
        &router,
        Request::get("/stream")
            .body(Body::empty())
            .expect("request"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // The record exists BEFORE the body is consumed: the middleware returned
    // while the stream was still unread.
    let event = only_event(&sink);
    assert_eq!(event.outcome, AuditOutcome::Success);

    let bytes = axum::body::to_bytes(response.into_body(), 1 << 20)
        .await
        .expect("the stream is still fully readable");
    let body = String::from_utf8(bytes.to_vec()).expect("utf8 body");
    assert!(body.starts_with("chunk-0\n"), "{body}");
    assert!(body.ends_with("chunk-63\n"), "{body}");
}

/// Every terminal record the middleware can build must satisfy the event
/// contract, or the sink would drop it with a bare `invalid` metric.
#[tokio::test]
async fn every_recorded_outcome_satisfies_the_event_contract() {
    let (router, sink) = harness();
    for path in [
        "/ok",
        "/created",
        "/empty",
        "/redirect",
        "/stream",
        "/slow",
        "/error/400",
        "/error/401",
        "/error/403",
        "/error/404",
        "/error/409",
        "/error/422",
        "/error/429",
        "/error/500",
        "/error/502",
        "/error/503",
        "/no-such-route",
    ] {
        let _ = call(
            &router,
            Request::get(path).body(Body::empty()).expect("request"),
        )
        .await;
    }
    let events = sink.events();
    assert_eq!(events.len(), 17, "one record per request, no more, no less");
    for event in &events {
        fkst_control_plane::audit::validate::validate(event)
            .unwrap_or_else(|error| panic!("{} {}: {error}", event.method, event.route_template));
        assert!(event.completed_at >= event.started_at);
        assert_eq!(
            event.duration_ms,
            u64::try_from((event.completed_at - event.started_at).num_milliseconds())
                .expect("non-negative duration")
        );
    }
}
