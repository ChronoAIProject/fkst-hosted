//! `required` delivery-mode tests: what the middleware refuses to do when the
//! durable relay cannot — or provably did not — record a request.
//!
//! Split from the middleware's own contract tests because it is a second,
//! self-contained matrix (two phases x outage/conflict/success x two modes) with
//! its own fixtures, and because one file holding both crossed the repository's
//! size limit.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use axum::http::StatusCode;
use axum::routing::get;
use axum::Router;

use crate::audit::event::ServiceIdentity;
use crate::audit::relay::{
    AuditDelivery, AuditDeliveryConfig, AuditDeliveryMode, RelayClientMetrics, RequiredRejection,
};
use crate::audit::request::catalog::OperationCatalog;
use crate::audit::request::id::REQUEST_ID_HEADER;
use crate::audit::request::middleware::{audit_requests, AuditMiddleware};
use crate::audit::request::response::{codes, AuditErrorCode};
use crate::audit::AuditHandle;

use super::{call, document};

/// A router whose one audited handler counts its invocations, so a test can
/// prove the handler did or did not run.
fn counting_app(delivery: AuditDelivery) -> (Router, Arc<AtomicUsize>) {
    let invocations = Arc::new(AtomicUsize::new(0));
    let handler_invocations = invocations.clone();
    let middleware = AuditMiddleware::new(
        Arc::new(OperationCatalog::from_openapi(&document()).expect("catalog builds")),
        AuditHandle::disabled(),
        ServiceIdentity {
            version: "9.9.9".to_string(),
            environment: "test".to_string(),
        },
    )
    .with_delivery(delivery);
    let router = Router::new()
        .route(
            "/api/v1/overview",
            get(move || {
                let invocations = handler_invocations.clone();
                async move {
                    invocations.fetch_add(1, Ordering::SeqCst);
                    "ok"
                }
            }),
        )
        .route("/health", get(|| async { "ok" }))
        .layer(axum::middleware::from_fn_with_state(
            middleware,
            audit_requests,
        ));
    (router, invocations)
}

/// A delivery policy plus the metrics handle it counts through, so a test can
/// assert WHICH emergency series a refusal landed on.
fn delivery_with_metrics(
    mode: AuditDeliveryMode,
    relay_url: &str,
) -> (AuditDelivery, RelayClientMetrics) {
    let metrics = RelayClientMetrics::new();
    let delivery = AuditDelivery::from_config(
        &AuditDeliveryConfig {
            mode,
            relay_url: Some(relay_url.to_string()),
            write_token: secrecy::SecretString::from("write-secret".to_string()),
            read_token: secrecy::SecretString::from("read-secret".to_string()),
            start_timeout_ms: 200,
            completion_timeout_ms: 200,
            incomplete_grace_secs: 60,
        },
        metrics.clone(),
    )
    .expect("the delivery policy builds");
    (delivery, metrics)
}

/// A delivery policy pointed at `relay_url`.
fn delivery_to(mode: AuditDeliveryMode, relay_url: &str) -> AuditDelivery {
    delivery_with_metrics(mode, relay_url).0
}

fn overview_request() -> axum::http::Request<axum::body::Body> {
    axum::http::Request::builder()
        .uri("/api/v1/overview")
        .body(axum::body::Body::empty())
        .expect("request builds")
}

/// A relay that acknowledges every start it is sent.
async fn acknowledging_starts(server: &wiremock::MockServer) {
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::path(
            "/internal/v1/audit/request-starts",
        ))
        .respond_with(
            wiremock::ResponseTemplate::new(201).set_body_json(serde_json::json!({
                "event_id": "11111111-1111-4111-8111-111111111111",
                "durable_at": "2026-07-31T12:00:00.000Z",
                "state": "started",
            })),
        )
        .mount(server)
        .await;
}

/// A relay that answers `409 event_id_conflict` on `method`.
async fn conflicting_relay(server: &wiremock::MockServer, method: &str) {
    wiremock::Mock::given(wiremock::matchers::method(method))
        .respond_with(
            wiremock::ResponseTemplate::new(409).set_body_json(serde_json::json!({
                "error": "event_id_conflict",
                "message": "already durable",
            })),
        )
        .mount(server)
        .await;
}

#[tokio::test]
async fn required_mode_does_not_invoke_the_handler_when_the_start_is_not_durable() {
    // Nothing listens on this port, so the start can never be acknowledged.
    let (router, invocations) = counting_app(delivery_to(
        AuditDeliveryMode::Required,
        "http://127.0.0.1:1",
    ));
    let response = call(&router, overview_request()).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_INGRESS_UNAVAILABLE))
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        0,
        "the product handler must not run when the invocation cannot be recorded"
    );
    // The normalized request id is still echoed, so the refusal is correlatable.
    assert!(response.headers().get(REQUEST_ID_HEADER).is_some());
}

#[tokio::test]
async fn best_effort_mode_runs_the_handler_despite_a_relay_outage() {
    let (router, invocations) = counting_app(delivery_to(
        AuditDeliveryMode::BestEffort,
        "http://127.0.0.1:1",
    ));
    let response = call(&router, overview_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn required_mode_refuses_to_report_a_status_it_could_not_record() {
    // The start is acknowledged but the completion is not, so the handler DID
    // run and the deployment must not claim its status was durably recorded.
    let server = wiremock::MockServer::start().await;
    acknowledging_starts(&server).await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(wiremock::ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let (router, invocations) = counting_app(delivery_to(
        AuditDeliveryMode::Required,
        server.uri().trim_end_matches('/'),
    ));
    let response = call(&router, overview_request()).await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_COMPLETION_UNCONFIRMED))
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the handler ran; only its durable record is in doubt"
    );
}

#[tokio::test]
async fn required_mode_releases_the_response_once_the_completion_is_durable() {
    let server = wiremock::MockServer::start().await;
    acknowledging_starts(&server).await;
    wiremock::Mock::given(wiremock::matchers::method("PUT"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "event_id": "11111111-1111-4111-8111-111111111111",
                "durable_at": "2026-07-31T12:00:00.100Z",
                "state": "complete",
            })),
        )
        .mount(&server)
        .await;

    let (router, invocations) = counting_app(delivery_to(
        AuditDeliveryMode::Required,
        server.uri().trim_end_matches('/'),
    ));
    let response = call(&router, overview_request()).await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn excluded_traffic_never_touches_the_relay_at_all() {
    // `/health` is excluded from audit, so a `required` deployment with a dead
    // relay must still answer its probes — otherwise the whole Pod would go
    // unready the moment the relay blinked.
    let (router, _) = counting_app(delivery_to(
        AuditDeliveryMode::Required,
        "http://127.0.0.1:1",
    ));
    let response = call(
        &router,
        axum::http::Request::builder()
            .uri("/health")
            .body(axum::body::Body::empty())
            .expect("request builds"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn required_mode_does_not_invoke_the_handler_when_a_different_start_is_durable() {
    // A `409` on the start path means the relay holds a DIFFERENT start under
    // this event id — an id collision across replicas, say. This invocation
    // therefore has no durable start, so its handler must not run.
    let server = wiremock::MockServer::start().await;
    conflicting_relay(&server, "POST").await;
    let (delivery, metrics) = delivery_with_metrics(
        AuditDeliveryMode::Required,
        server.uri().trim_end_matches('/'),
    );
    let (router, invocations) = counting_app(delivery);

    let response = call(&router, overview_request()).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_INGRESS_UNAVAILABLE))
    );
    assert_eq!(invocations.load(Ordering::SeqCst), 0);
    let snapshot = metrics.snapshot();
    assert_eq!(
        snapshot.rejections(RequiredRejection::IngressConflict),
        1,
        "a conflict must land on its own emergency series, not the outage one"
    );
    assert_eq!(
        snapshot.rejections(RequiredRejection::IngressUnavailable),
        0
    );
}

#[tokio::test]
async fn required_mode_refuses_a_status_the_durable_trail_contradicts() {
    // The start is durable, the handler runs, and the relay then reports that a
    // DIFFERENT terminal projection already exists for this id — in production,
    // the `incomplete` row it synthesized once the deadline passed. The handler's
    // `200` must not be released: the durable trail says something else.
    let server = wiremock::MockServer::start().await;
    acknowledging_starts(&server).await;
    conflicting_relay(&server, "PUT").await;
    let (delivery, metrics) = delivery_with_metrics(
        AuditDeliveryMode::Required,
        server.uri().trim_end_matches('/'),
    );
    let (router, invocations) = counting_app(delivery);

    let response = call(&router, overview_request()).await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(
        response.extensions().get::<AuditErrorCode>(),
        Some(&AuditErrorCode(codes::AUDIT_COMPLETION_UNCONFIRMED))
    );
    assert_eq!(
        invocations.load(Ordering::SeqCst),
        1,
        "the handler ran; what is refused is the CLAIM that its status was recorded"
    );
    assert_eq!(
        metrics
            .snapshot()
            .rejections(RequiredRejection::CompletionConflict),
        1
    );
}

#[tokio::test]
async fn best_effort_mode_releases_the_response_through_a_conflict() {
    // The same conflict must never change a response in `best_effort`: that mode
    // promises nothing about durability, so it may not fail a completed request.
    let server = wiremock::MockServer::start().await;
    conflicting_relay(&server, "POST").await;
    let (delivery, _) = delivery_with_metrics(
        AuditDeliveryMode::BestEffort,
        server.uri().trim_end_matches('/'),
    );
    let (router, invocations) = counting_app(delivery);

    let response = call(&router, overview_request()).await;

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(invocations.load(Ordering::SeqCst), 1);
}
