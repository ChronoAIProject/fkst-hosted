//! Delivery-policy tests: what each mode promises, and the one thing a conflict
//! must not be mistaken for.

use std::sync::Arc;

use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit::event::RequestIdentity;

use super::*;

const EVENT: uuid::Uuid = uuid::Uuid::from_u128(0x1111_1111_1111_4111_8111_1111_1111_1111);

fn identity() -> RequestIdentity {
    RequestIdentity {
        request_id: "req-1".to_string(),
        method: "GET".to_string(),
        route_template: "/api/v1/overview".to_string(),
        operation_id: "canvas_overview".to_string(),
    }
}

fn config_for(server: &MockServer, mode: AuditDeliveryMode) -> AuditDeliveryConfig {
    AuditDeliveryConfig {
        mode,
        relay_url: Some(server.uri().trim_end_matches('/').to_string()),
        write_token: SecretString::from("write-secret".to_string()),
        read_token: SecretString::from("read-secret".to_string()),
        start_timeout_ms: 300,
        completion_timeout_ms: 300,
        incomplete_grace_secs: 90,
    }
}

async fn register(delivery: &AuditDelivery) -> Result<(), RelayClientError> {
    delivery
        .register_start(
            &identity(),
            EVENT,
            k8s_openapi::chrono::Utc::now(),
            "0.2.3",
            "test",
        )
        .await
}

#[test]
fn the_disabled_policy_makes_no_call_and_keeps_the_local_sink() {
    let delivery = AuditDelivery::disabled();
    assert_eq!(delivery.mode(), AuditDeliveryMode::Disabled);
    assert!(delivery.client().is_none());
    assert!(delivery.use_local_sink());
}

#[test]
fn the_required_policy_does_not_double_send_to_the_local_sink() {
    let delivery = AuditDelivery::with_client(
        AuditDeliveryMode::Required,
        Arc::new(
            AuditRelayClient::from_config(
                &AuditDeliveryConfig {
                    mode: AuditDeliveryMode::Required,
                    relay_url: Some("http://127.0.0.1:1".to_string()),
                    write_token: SecretString::from("write-secret".to_string()),
                    ..AuditDeliveryConfig::default()
                },
                RelayClientMetrics::new(),
            )
            .expect("client builds"),
        ),
        60,
        RelayClientMetrics::new(),
    );
    assert!(!delivery.use_local_sink());
}

#[tokio::test]
async fn best_effort_swallows_a_relay_outage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::BestEffort),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    assert!(
        register(&delivery).await.is_ok(),
        "best effort never fails a request"
    );
    assert!(delivery.use_local_sink(), "the local sink is the fallback");
}

#[tokio::test]
async fn required_surfaces_a_relay_outage() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    assert!(register(&delivery).await.is_err());
}

/// A `409` on either write path, mounted on `path`.
async fn conflicting(server: &MockServer, method_name: &str, request_path: &str) {
    Mock::given(method(method_name))
        .and(path(request_path))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": "event_id_conflict",
            "message": "already durable",
        })))
        .mount(server)
        .await;
}

fn terminal() -> crate::audit::event::ApiRequestCompletedV1 {
    use crate::audit::event::{
        ApiRequestCompletedV1, AuditOutcome, RequestResult, RequestTiming, ServiceIdentity,
    };
    let started_at = k8s_openapi::chrono::Utc::now();
    ApiRequestCompletedV1::new(
        identity(),
        RequestTiming {
            started_at,
            completed_at: started_at,
        },
        crate::audit::event::Actor::anonymous(),
        crate::audit::event::Principal::none(),
        RequestResult {
            status_code: Some(200),
            outcome: AuditOutcome::Success,
            error_code: None,
        },
        ServiceIdentity {
            version: "0.2.3".to_string(),
            environment: "test".to_string(),
        },
    )
}

#[tokio::test]
async fn a_conflicting_start_is_a_failure_because_a_different_start_is_the_durable_one() {
    // The relay answers an EXACT replay with `200`, so a `409` can only mean the
    // stored start describes a different invocation. Running the handler anyway
    // would leave this invocation with no durable start at all — the one thing
    // `required` mode exists to prevent.
    let server = MockServer::start().await;
    conflicting(&server, "POST", "/internal/v1/audit/request-starts").await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    assert_eq!(
        register(&delivery).await,
        Err(RelayClientError::Conflict),
        "a conflicting start must not be reported as durable"
    );
}

#[tokio::test]
async fn best_effort_still_swallows_a_conflicting_start() {
    let server = MockServer::start().await;
    conflicting(&server, "POST", "/internal/v1/audit/request-starts").await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::BestEffort),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    assert!(
        register(&delivery).await.is_ok(),
        "best effort never changes a response, whatever the relay says"
    );
}

#[tokio::test]
async fn a_conflicting_completion_is_a_failure_because_history_says_otherwise() {
    // The relay holding a different terminal projection is POSITIVE evidence
    // that this process's status was not recorded — in practice the `incomplete`
    // row it synthesized after the deadline. Releasing the handler's status here
    // would assert a durable record that provably does not exist.
    let server = MockServer::start().await;
    conflicting(
        &server,
        "PUT",
        &format!("/internal/v1/audit/requests/{EVENT}/completion"),
    )
    .await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    let mut event = terminal();
    event.event_id = EVENT;
    assert_eq!(
        delivery.complete(&event).await,
        Err(RelayClientError::Conflict)
    );
}

#[tokio::test]
async fn best_effort_still_swallows_a_conflicting_completion() {
    let server = MockServer::start().await;
    conflicting(
        &server,
        "PUT",
        &format!("/internal/v1/audit/requests/{EVENT}/completion"),
    )
    .await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::BestEffort),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    let mut event = terminal();
    event.event_id = EVENT;
    assert!(delivery.complete(&event).await.is_ok());
}

#[test]
fn a_relay_mode_without_a_usable_write_half_is_a_startup_failure() {
    let error = AuditDelivery::from_config(
        &AuditDeliveryConfig {
            mode: AuditDeliveryMode::Required,
            relay_url: None,
            ..AuditDeliveryConfig::default()
        },
        RelayClientMetrics::new(),
    )
    .expect_err("a required policy needs a relay");
    assert!(error.to_string().contains("FKST_AUDIT_RELAY_URL"));
}

#[tokio::test]
async fn the_start_deadline_is_the_start_instant_plus_the_shared_grace() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "event_id": EVENT.to_string(),
            "durable_at": "2026-07-31T12:00:00.000Z",
            "state": "started",
        })))
        .mount(&server)
        .await;
    let delivery = AuditDelivery::from_config(
        &config_for(&server, AuditDeliveryMode::Required),
        RelayClientMetrics::new(),
    )
    .expect("policy builds");
    register(&delivery).await.expect("acknowledged");

    let requests = server.received_requests().await.expect("recorded requests");
    let body: crate::audit_relay::protocol::RequestStartV1 =
        serde_json::from_slice(&requests[0].body).expect("a start body");
    let started =
        k8s_openapi::chrono::DateTime::parse_from_rfc3339(&body.started_at).expect("rfc3339");
    let deadline = k8s_openapi::chrono::DateTime::parse_from_rfc3339(&body.completion_deadline_at)
        .expect("rfc3339");
    assert_eq!((deadline - started).num_seconds(), 90);
}
