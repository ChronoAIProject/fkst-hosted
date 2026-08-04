//! Write-endpoint tests: the `201`/`200`/`409` protocol, server-side
//! re-validation, and capacity refusal.

use axum::http::StatusCode;

use crate::audit_relay::http::tests::{call, json_request};
use crate::audit_relay::metrics::{IngressKind, IngressResult};
use crate::audit_relay::test_support::{completion, lifecycle, relay, start, WRITE_TOKEN};

const EVENT: &str = "11111111-1111-4111-8111-111111111111";
const STARTS: &str = "/internal/v1/audit/request-starts";

fn completion_uri(event_id: &str) -> String {
    format!("/internal/v1/audit/requests/{event_id}/completion")
}

#[tokio::test]
async fn a_first_start_is_201_and_an_exact_replay_is_200() {
    let (_dir, state, router) = relay();
    let body = start(EVENT);
    let (status, ack) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &body),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(ack["event_id"], EVENT);
    assert_eq!(ack["state"], "started");
    assert!(ack["durable_at"].is_string());

    let (status, ack) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &body),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack["state"], "started");

    assert_eq!(
        state
            .metrics
            .ingress_count(IngressKind::RequestStart, IngressResult::Created),
        1
    );
    assert_eq!(
        state
            .metrics
            .ingress_count(IngressKind::RequestStart, IngressResult::Replayed),
        1
    );
}

#[tokio::test]
async fn a_divergent_start_replay_is_409_event_id_conflict() {
    let (_dir, state, router) = relay();
    call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &start(EVENT)),
    )
    .await;
    let mut divergent = start(EVENT);
    divergent.route_template = "/api/v1/logs/{session_id}".to_string();
    let (status, body) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &divergent),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "event_id_conflict");
    assert_eq!(
        state
            .metrics
            .ingress_count(IngressKind::RequestStart, IngressResult::Conflict),
        1
    );
}

#[tokio::test]
async fn a_completion_before_its_start_is_refused() {
    let (_dir, _state, router) = relay();
    let (status, body) = call(
        &router,
        json_request(
            "PUT",
            &completion_uri(EVENT),
            Some(WRITE_TOKEN),
            &completion(EVENT, Some(101)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "no_registered_start");
}

#[tokio::test]
async fn a_completion_commits_and_replays_exactly() {
    let (_dir, _state, router) = relay();
    call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &start(EVENT)),
    )
    .await;
    let terminal = completion(EVENT, Some(101));
    let (status, ack) = call(
        &router,
        json_request("PUT", &completion_uri(EVENT), Some(WRITE_TOKEN), &terminal),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack["state"], "complete");

    let (status, ack) = call(
        &router,
        json_request("PUT", &completion_uri(EVENT), Some(WRITE_TOKEN), &terminal),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(ack["state"], "complete");

    let mut different = terminal.clone();
    different.status_code = Some(403);
    different.outcome = "client_error".to_string();
    let (status, body) = call(
        &router,
        json_request("PUT", &completion_uri(EVENT), Some(WRITE_TOKEN), &different),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "event_id_conflict");
}

#[tokio::test]
async fn the_path_event_id_must_equal_the_body_event_id() {
    let (_dir, _state, router) = relay();
    let (status, body) = call(
        &router,
        json_request(
            "PUT",
            &completion_uri("22222222-2222-4222-8222-222222222222"),
            Some(WRITE_TOKEN),
            &completion(EVENT, Some(101)),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
}

#[tokio::test]
async fn the_audit_event_contract_is_re_run_on_the_relay() {
    // The control plane already validated; the relay is a separate trust
    // boundary and must refuse the same records on its own.
    let (_dir, _state, router) = relay();
    call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &start(EVENT)),
    )
    .await;

    // A raw, query-bearing URI in place of the matched template.
    let mut raw_uri = completion(EVENT, Some(101));
    raw_uri.route_template = "/api/v1/auth/github/callback?code=secret".to_string();
    let (status, body) = call(
        &router,
        json_request("PUT", &completion_uri(EVENT), Some(WRITE_TOKEN), &raw_uri),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
    assert!(
        !body.to_string().contains("code=secret"),
        "a rejection must never echo the offending value"
    );

    // A non-human actor claiming a person's id.
    let mut impersonating = completion(EVENT, None);
    impersonating.actor_id = Some(101);
    let (status, _) = call(
        &router,
        json_request(
            "PUT",
            &completion_uri(EVENT),
            Some(WRITE_TOKEN),
            &impersonating,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_lifecycle_event_is_201_then_200_then_409() {
    let (_dir, _state, router) = relay();
    let event = lifecycle("33333333-3333-4333-8333-333333333333", "sess-1");
    let (status, ack) = call(
        &router,
        json_request(
            "POST",
            "/internal/v1/audit/events",
            Some(WRITE_TOKEN),
            &event,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(ack["state"], "complete");

    let (status, _) = call(
        &router,
        json_request(
            "POST",
            "/internal/v1/audit/events",
            Some(WRITE_TOKEN),
            &event,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK);

    let mut divergent = event.clone();
    divergent.lifecycle_action = "deleted".to_string();
    let (status, body) = call(
        &router,
        json_request(
            "POST",
            "/internal/v1/audit/events",
            Some(WRITE_TOKEN),
            &divergent,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT);
    assert_eq!(body["error"], "event_id_conflict");
}

#[tokio::test]
async fn ingress_is_refused_at_the_configured_capacity() {
    let (_dir, state, router) = relay();
    state
        .at_capacity
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let (status, body) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &start(EVENT)),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(body["error"], "relay_at_capacity");
}

#[tokio::test]
async fn a_malformed_start_is_rejected_without_echoing_its_content() {
    let (_dir, _state, router) = relay();
    let mut malformed = start(EVENT);
    malformed.event_id = "not-a-uuid-at-all".to_string();
    let (status, body) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &malformed),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(!body.to_string().contains("not-a-uuid-at-all"));
}

#[tokio::test]
async fn a_start_carrying_a_raw_query_bearing_uri_is_refused() {
    // The start path is the one that MUST re-validate: a start is stored
    // verbatim and copied into the synthesized `incomplete` projection, so an
    // accepted raw URI would reach durable storage AND the read API, carrying
    // whatever the query string held.
    let (_dir, state, router) = relay();
    let mut smuggled = start(EVENT);
    smuggled.route_template = "/api/v1/auth/github/callback?code=secret".to_string();
    let (status, body) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &smuggled),
    )
    .await;

    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(body["error"], "invalid_request");
    assert!(
        !body.to_string().contains("code=secret"),
        "a rejection must never echo the offending value"
    );
    assert_eq!(
        state
            .metrics
            .ingress_count(IngressKind::RequestStart, IngressResult::Rejected),
        1
    );
    // Nothing was committed, so nothing can later be projected from it.
    assert_eq!(
        state
            .metrics
            .ingress_count(IngressKind::RequestStart, IngressResult::Created),
        0
    );
}

#[tokio::test]
async fn a_start_with_an_unbounded_or_control_bearing_field_is_refused() {
    let (_dir, _state, router) = relay();
    let mut oversized = start(EVENT);
    oversized.operation_id = "x".repeat(4_096);
    let (status, _) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &oversized),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);

    let mut newline = start(EVENT);
    newline.method = "GET\nX-Injected: 1".to_string();
    let (status, _) = call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &newline),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn a_first_completion_counts_as_created_and_its_replay_as_replayed() {
    // Both answer `200`, so the HTTP status cannot distinguish them; the metric
    // must, or an operator cannot see a retry storm.
    let (_dir, state, router) = relay();
    call(
        &router,
        json_request("POST", STARTS, Some(WRITE_TOKEN), &start(EVENT)),
    )
    .await;
    let terminal = completion(EVENT, Some(101));
    for _ in 0..2 {
        let (status, _) = call(
            &router,
            json_request("PUT", &completion_uri(EVENT), Some(WRITE_TOKEN), &terminal),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    let metrics = &state.metrics;
    assert_eq!(
        metrics.ingress_count(IngressKind::RequestCompletion, IngressResult::Created),
        1,
        "the first commit is a creation even though it answers 200"
    );
    assert_eq!(
        metrics.ingress_count(IngressKind::RequestCompletion, IngressResult::Replayed),
        1
    );
}
