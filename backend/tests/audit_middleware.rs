//! Router-level tests for the outer audit middleware's request lifecycle,
//! against the REAL [`fkst_control_plane::router::build_router`].
//!
//! This suite covers what the middleware does with a request *as a request*:
//! which traffic is in scope, how a route resolves to an operation, and how the
//! `X-Request-Id` is accepted, replaced, and propagated. How a request's
//! identity is attributed and classified lives in the sibling `audit_identity`
//! suite; outcome derivation across the full status surface lives in
//! `audit_outcomes`.

mod audit_router;

use audit_router::Harness;
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use fkst_control_plane::audit::AuditOutcome;

/// Probe, scrape, contract, and preflight traffic must never reach the sink —
/// and must still be answered exactly as before.
#[tokio::test]
async fn excluded_traffic_produces_no_records() {
    let harness = Harness::new();
    for path in ["/health", "/ready", "/metrics", "/openapi.json"] {
        let response = harness.get(path).await;
        assert!(
            response.status().is_success(),
            "{path} must still answer: {}",
            response.status()
        );
    }
    let preflight = harness
        .call(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/v1/overview")
                .header("origin", "https://app.example")
                .header("access-control-request-method", "GET")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert!(preflight.status().is_success());
    assert!(
        harness.sink.is_empty(),
        "probe/scrape/contract/preflight traffic must be excluded, got {:#?}",
        harness.sink.events()
    );
}

/// axum answers HEAD from the GET handler, so a HEAD uptime probe or
/// load-balancer check reaches the very same operations — and must inherit the
/// very same exclusions. Without that, every HEAD-based monitor would pump
/// probe/scrape noise into the trail.
#[tokio::test]
async fn head_probes_are_excluded_exactly_like_their_get_counterparts() {
    let harness = Harness::new();
    for path in ["/health", "/ready", "/metrics", "/openapi.json"] {
        let response = harness.head(path).await;
        assert!(
            response.status().is_success(),
            "HEAD {path} must still answer: {}",
            response.status()
        );
    }
    assert!(
        harness.sink.is_empty(),
        "HEAD probe/scrape/contract traffic must be excluded, got {:#?}",
        harness.sink.events()
    );
}

/// An unrouted `/api/v1` path may carry OAuth material in its query, so neither
/// the path nor the query may survive into the record.
#[tokio::test]
async fn an_unmatched_api_path_records_sentinels_only() {
    let harness = Harness::new();
    let response = harness
        .get("/api/v1/not-a-real-route?code=canary-code&state=canary-state")
        .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "<unmatched>");
    assert_eq!(event.route_template, "<unmatched>");
    assert_eq!(event.outcome, AuditOutcome::ClientError);
    assert_eq!(event.error_code.as_deref(), Some("route_not_found"));
    let rendered = format!("{event:#?}");
    for canary in ["canary-code", "canary-state", "not-a-real-route"] {
        assert!(
            !rendered.contains(canary),
            "{canary} leaked into {rendered}"
        );
    }
}

/// A matched path with an unserved method keeps its documented template (a
/// public constant) but never invents an operation id.
#[tokio::test]
async fn a_method_not_allowed_answer_is_recorded_under_the_matched_template() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::post("/api/v1/overview")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "<unmatched>");
    assert_eq!(event.route_template, "/api/v1/overview");
    assert_eq!(event.error_code.as_deref(), Some("method_not_allowed"));
}

/// The leader gate answers before any handler; the record must say so.
#[tokio::test]
async fn a_leader_gate_rejection_is_recorded_as_rejected_with_its_real_status() {
    let harness = Harness::follower();
    let response = harness.get("/api/v1/overview").await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    let event = harness.only_event();
    assert_eq!(event.operation_id, "canvas_overview");
    assert_eq!(event.status_code, Some(503));
    assert_eq!(event.outcome, AuditOutcome::Rejected);
    assert_eq!(event.error_code.as_deref(), Some("leader_not_ready"));
    assert_eq!(
        event.actor_id, None,
        "a gated request has no verified actor"
    );
}

#[tokio::test]
async fn a_well_formed_request_id_is_accepted_and_echoed_and_recorded() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::get("/api/v1/overview")
                .header("x-request-id", "client-supplied-1")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    assert_eq!(
        response
            .headers()
            .get("x-request-id")
            .and_then(|value| value.to_str().ok()),
        Some("client-supplied-1")
    );
    assert_eq!(harness.only_event().request_id, "client-supplied-1");
}

#[tokio::test]
async fn a_malformed_request_id_is_replaced_in_the_response_and_the_record() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::get("/api/v1/overview")
                .header("x-request-id", "forged id; with separators")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("a normalized id is always echoed")
        .to_string();
    assert_ne!(echoed, "forged id; with separators");
    assert_eq!(harness.only_event().request_id, echoed);
}

#[tokio::test]
async fn a_request_without_a_request_id_gets_a_generated_one() {
    let harness = Harness::new();
    let response = harness.get("/api/v1/overview").await;
    let echoed = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .expect("a request id is always generated")
        .to_string();
    assert_eq!(echoed.len(), 36, "a generated id is a hyphenated UUID");
    assert_eq!(harness.only_event().request_id, echoed);
}

/// Echoing `X-Request-Id` is only half the contract: the frontend and the API
/// are served from different origins in the deployment topology this repository
/// prescribes, and a browser hides every response header that is neither
/// CORS-safelisted nor explicitly exposed. Without this the operations UI's
/// "quote this request id to support" line silently never renders.
#[tokio::test]
async fn the_request_id_header_is_exposed_to_cross_origin_callers() {
    let harness = Harness::new();
    let response = harness
        .call(
            Request::get("/api/v1/overview")
                .header("origin", "https://app.example")
                .body(Body::empty())
                .expect("request builds"),
        )
        .await;
    let exposed = response
        .headers()
        .get("access-control-expose-headers")
        .and_then(|value| value.to_str().ok())
        .expect("a cross-origin response states which headers it exposes")
        .to_ascii_lowercase();
    assert!(
        exposed.split(',').any(|name| name.trim() == "x-request-id"),
        "x-request-id must be readable cross-origin, got {exposed:?}"
    );
}

/// Different requests must never share a delivery id, or PostHog's UUID
/// deduplication would silently collapse them into one row.
#[tokio::test]
async fn every_request_gets_its_own_event_id() {
    let harness = Harness::new();
    for path in [
        "/api/v1/overview",
        "/api/v1/users/me/environment-profiles",
        "/api/v1/nope",
    ] {
        let _ = harness.get(path).await;
    }
    let events = harness.sink.events();
    assert_eq!(events.len(), 3);
    let mut ids: Vec<_> = events.iter().map(|event| event.event_id).collect();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), 3, "every request needs a distinct event id");
}
