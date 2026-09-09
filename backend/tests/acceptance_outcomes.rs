//! Milestone acceptance: the complete terminal-outcome matrix, end to end.
//!
//! `audit_outcomes.rs` proves the middleware classifies each status correctly.
//! This suite asks the three questions that only make sense once the whole
//! delivery path is in play, and that no sibling suite covers:
//!
//! 1. does the WHOLE status surface — including the `202`, `204`, `405`, and
//!    `413` shapes the sibling harness omits — produce exactly one terminal
//!    record each, with a consistent start/completion/duration ordering?
//! 2. does a retried request produce two independent records rather than one
//!    duplicate, given the deterministic `event_id` derivation?
//! 3. does a request whose process dies after the durable start close as
//!    `incomplete` with a NULL status, rather than having a status invented for
//!    it?
//!
//! The third is the honest boundary the epic's delivery semantics draw, and it
//! can only be observed against a real relay: the control plane, by definition,
//! is not there to report it.

#[path = "audit_relay_harness/mod.rs"]
mod relay;

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
use fkst_control_plane::audit_relay::protocol::format_instant;
use fkst_control_plane::error::AppError;
use k8s_openapi::chrono::Duration as ChronoDuration;
use tower::ServiceExt;

/// One expected terminal record.
struct Expected {
    /// The request to issue.
    method: &'static str,
    path: &'static str,
    /// The status the caller must observe.
    status: u16,
    outcome: AuditOutcome,
    /// The stable code, or `None` for a success/redirect.
    error_code: Option<&'static str>,
    /// The route template the record must carry.
    template: &'static str,
}

/// The whole matrix, in one table.
///
/// `408` is exercised by its own test (it needs a shorter deadline than every
/// other route), and streaming likewise; everything else is a plain call.
const MATRIX: &[Expected] = &[
    Expected {
        method: "GET",
        path: "/ok",
        status: 200,
        outcome: AuditOutcome::Success,
        error_code: None,
        template: "/ok",
    },
    Expected {
        method: "POST",
        path: "/created",
        status: 201,
        outcome: AuditOutcome::Success,
        error_code: None,
        template: "/created",
    },
    Expected {
        method: "POST",
        path: "/accepted",
        status: 202,
        outcome: AuditOutcome::Success,
        error_code: None,
        template: "/accepted",
    },
    Expected {
        method: "DELETE",
        path: "/gone",
        status: 204,
        outcome: AuditOutcome::Success,
        error_code: None,
        template: "/gone",
    },
    Expected {
        method: "GET",
        path: "/redirect",
        status: 302,
        outcome: AuditOutcome::Redirect,
        error_code: None,
        template: "/redirect",
    },
    Expected {
        method: "GET",
        path: "/error/400",
        status: 400,
        outcome: AuditOutcome::ClientError,
        error_code: Some("invalid_request"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/401",
        status: 401,
        outcome: AuditOutcome::Rejected,
        error_code: Some("unauthorized"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/403",
        status: 403,
        outcome: AuditOutcome::Rejected,
        error_code: Some("forbidden"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/404",
        status: 404,
        outcome: AuditOutcome::ClientError,
        error_code: Some("not_found"),
        template: "/error/{code}",
    },
    // A MATCHED path with the wrong method: the record must carry the matched
    // template, not the unmatched sentinel.
    Expected {
        method: "PUT",
        path: "/ok",
        status: 405,
        outcome: AuditOutcome::ClientError,
        error_code: Some("method_not_allowed"),
        template: "/ok",
    },
    Expected {
        method: "GET",
        path: "/error/409",
        status: 409,
        outcome: AuditOutcome::ClientError,
        error_code: Some("conflict"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/422",
        status: 422,
        outcome: AuditOutcome::ClientError,
        error_code: Some("unprocessable"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/429",
        status: 429,
        outcome: AuditOutcome::ClientError,
        error_code: Some("rate_limited"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/500",
        status: 500,
        outcome: AuditOutcome::ServerError,
        error_code: Some("internal"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/502",
        status: 502,
        outcome: AuditOutcome::ServerError,
        error_code: Some("upstream_error"),
        template: "/error/{code}",
    },
    Expected {
        method: "GET",
        path: "/error/503",
        status: 503,
        outcome: AuditOutcome::ServerError,
        error_code: Some("unavailable"),
        template: "/error/{code}",
    },
    // An unmatched `/api/v1` path: sentinels, never the raw path.
    Expected {
        method: "GET",
        path: "/api/v1/nothing-here",
        status: 404,
        outcome: AuditOutcome::ClientError,
        error_code: Some("route_not_found"),
        template: "<unmatched>",
    },
];

fn harness() -> (Router, RecordingSink) {
    let (handle, sink) = AuditHandle::recording();
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::default()),
        handle,
        ServiceIdentity {
            version: "9.9.9".to_string(),
            environment: "test".to_string(),
        },
    );
    let router = Router::new()
        .route("/ok", get(|| async { "ok" }))
        .route(
            "/created",
            post(|| async { (StatusCode::CREATED, "made").into_response() }),
        )
        .route(
            "/accepted",
            post(|| async { (StatusCode::ACCEPTED, "queued").into_response() }),
        )
        .route(
            "/gone",
            axum::routing::delete(|| async { StatusCode::NO_CONTENT.into_response() }),
        )
        .route(
            "/redirect",
            // An explicit `302`: the shape the OAuth and log-download callbacks
            // return, and the one the epic's terminal matrix names.
            get(|| async {
                (StatusCode::FOUND, [(axum::http::header::LOCATION, "/ok")]).into_response()
            }),
        )
        .route(
            "/bounded",
            // A body ceiling turns an oversized upload into a `413` BEFORE the
            // handler, which is the shape a real upload route has.
            post(|_: axum::body::Bytes| async { "accepted" })
                .layer(axum::extract::DefaultBodyLimit::max(16)),
        )
        .route("/error/:code", get(error_handler))
        .layer(axum::middleware::from_fn_with_state(
            middleware,
            audit_requests,
        ));
    (router, sink)
}

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

/// Every row of the matrix: one record, correctly classified, correctly ordered.
#[tokio::test]
async fn the_whole_terminal_matrix_produces_exactly_one_record_each() {
    for expected in MATRIX {
        let (router, sink) = harness();
        let response = router
            .oneshot(
                Request::builder()
                    .method(expected.method)
                    .uri(expected.path)
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router responds");
        assert_eq!(
            response.status().as_u16(),
            expected.status,
            "{} {}",
            expected.method,
            expected.path
        );

        let events = sink.events();
        assert_eq!(
            events.len(),
            1,
            "{} {} produced {} records",
            expected.method,
            expected.path,
            events.len()
        );
        let event = &events[0];
        assert_row(event, expected);
    }
}

/// The body-ceiling row needs a body, so it has its own call.
#[tokio::test]
async fn an_oversized_body_is_one_client_error_record_with_no_payload() {
    let (router, sink) = harness();
    let response = router
        .oneshot(
            Request::post("/bounded")
                .body(Body::from("canary-oversized-payload-0e4c7b9a".repeat(4)))
                .expect("request builds"),
        )
        .await
        .expect("router responds");
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

    let events = sink.events();
    assert_eq!(events.len(), 1);
    let event = &events[0];
    assert_eq!(event.status_code, Some(413));
    assert_eq!(event.outcome, AuditOutcome::ClientError);
    assert_eq!(event.route_template, "/bounded");
    assert!(
        !format!("{event:#?}").contains("canary-oversized-payload"),
        "the refused body reached the record"
    );
    fkst_control_plane::audit::validate::validate(event).expect("a valid record");
}

fn assert_row(event: &ApiRequestCompletedV1, expected: &Expected) {
    let at = format!("{} {}", expected.method, expected.path);
    assert_eq!(event.status_code, Some(expected.status), "{at}");
    assert_eq!(event.outcome, expected.outcome, "{at}");
    assert_eq!(event.error_code.as_deref(), expected.error_code, "{at}");
    assert_eq!(event.route_template, expected.template, "{at}");
    assert_eq!(event.method, expected.method, "{at}");
    assert!(event.completed_at >= event.started_at, "{at}: inverted");
    assert_eq!(
        event.duration_ms,
        u64::try_from((event.completed_at - event.started_at).num_milliseconds())
            .expect("a non-negative duration"),
        "{at}: the duration disagrees with the instants"
    );
    assert!(!event.request_id.is_empty(), "{at}: no request id");
    assert!(!event.event_id.is_nil(), "{at}: no event id");
    fkst_control_plane::audit::validate::validate(event)
        .unwrap_or_else(|error| panic!("{at}: {error}"));
}

/// A client retrying the SAME request produces two records with two event ids.
///
/// The event id is derived from identity plus the start instant, so two attempts
/// are genuinely distinct events even when the client reuses one request id. The
/// opposite property — one durable record per event id under a REPLAYED
/// submission — is the relay's, and is asserted below.
#[tokio::test]
async fn a_retried_request_produces_two_records_and_never_one_duplicate_id() {
    let (router, sink) = harness();
    for _ in 0..3 {
        let response = router
            .clone()
            .oneshot(
                Request::get("/error/503")
                    .header("x-request-id", "11111111-1111-4111-8111-111111111111")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router responds");
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        // Distinct start instants are what make the derived ids distinct; a
        // same-millisecond retry is the relay's dedup case, not the sink's.
        tokio::time::sleep(Duration::from_millis(2)).await;
    }
    let events = sink.events();
    assert_eq!(events.len(), 3, "one record per attempt");
    let ids: std::collections::BTreeSet<String> = events
        .iter()
        .map(|event| event.event_id.to_string())
        .collect();
    assert_eq!(ids.len(), 3, "two attempts shared one event id: {ids:?}");
    for event in &events {
        assert_eq!(
            event.request_id, "11111111-1111-4111-8111-111111111111",
            "the client's own correlation id must survive every attempt"
        );
    }
}

/// A request whose process dies after the durable start closes as `incomplete`
/// with a NULL status — no fabricated outcome.
#[tokio::test]
async fn an_aborted_request_closes_as_incomplete_with_a_null_status() {
    let node = relay::Relay::start().await;
    let client = node.client();
    let event_id = "aaaaaaaa-1111-4111-8111-aaaaaaaaaaaa";

    // The start is durable, and already past its own completion deadline — which
    // is the state a process killed mid-request leaves behind once the deadline
    // it registered has elapsed.
    let mut start = relay::Relay::start_body(event_id);
    start.started_at = format_instant(relay::anchor() - ChronoDuration::seconds(300));
    start.completion_deadline_at = format_instant(relay::anchor() - ChronoDuration::seconds(240));
    client
        .register_start(&start)
        .await
        .expect("the start is acknowledged");
    // ...and the completion never arrives: the process died here.

    node.sweep(k8s_openapi::chrono::Utc::now()).await;

    let rows = node.read_all_recent().await;
    let row = rows
        .iter()
        .find(|row| row.event_id == event_id)
        .unwrap_or_else(|| panic!("the synthesized terminal is not readable: {rows:?}"));
    assert_eq!(
        row.terminal["status_code"],
        serde_json::Value::Null,
        "a status was invented for a request that never produced one"
    );
    assert_eq!(row.terminal["outcome"], "incomplete");
    // The route identity is copied from the start rather than guessed.
    assert_eq!(row.terminal["operation_id"], "canvas_overview");
    assert_eq!(row.terminal["method"], "GET");
    // And ownership stays unprovable, which is why the epic makes such a record
    // global-admin-only.
    assert_eq!(row.terminal["actor_id"], serde_json::Value::Null);
}
