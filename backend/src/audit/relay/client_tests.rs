//! Relay-client tests over a real HTTP server: acknowledgement, conflict,
//! refusal, retry inside the budget, and credential hygiene.

use std::time::Duration;

use secrecy::SecretString;
use wiremock::matchers::{header, method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit_relay::protocol::{format_instant, RequestStartV1, PROTOCOL_SCHEMA_VERSION};

use super::*;
use crate::audit::relay::{AuditDeliveryConfig, AuditDeliveryMode};

const WRITE_TOKEN: &str = "canary-write-token-4f2a";
const READ_TOKEN: &str = "canary-read-token-9b71";
const EVENT: &str = "11111111-1111-4111-8111-111111111111";

fn client_for(server: &MockServer) -> AuditRelayClient {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(server.uri().trim_end_matches('/').to_string()),
        write_token: SecretString::from(WRITE_TOKEN.to_string()),
        read_token: SecretString::from(READ_TOKEN.to_string()),
        start_timeout_ms: 500,
        completion_timeout_ms: 800,
        incomplete_grace_secs: 60,
    };
    AuditRelayClient::from_config(&config, RelayClientMetrics::new()).expect("client builds")
}

fn start() -> RequestStartV1 {
    let now = k8s_openapi::chrono::Utc::now();
    RequestStartV1 {
        schema_version: PROTOCOL_SCHEMA_VERSION,
        event_id: EVENT.to_string(),
        request_id: "req-1".to_string(),
        started_at: format_instant(now),
        method: "GET".to_string(),
        route_template: "/api/v1/overview".to_string(),
        operation_id: "canvas_overview".to_string(),
        service_version: "0.2.3".to_string(),
        deployment_environment: "test".to_string(),
        completion_deadline_at: format_instant(now + k8s_openapi::chrono::Duration::seconds(60)),
    }
}

fn ack_body() -> serde_json::Value {
    serde_json::json!({
        "event_id": EVENT,
        "durable_at": "2026-07-31T12:00:00.000Z",
        "state": "started",
    })
}

#[tokio::test]
async fn a_start_is_acknowledged_and_carries_only_the_write_token() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .and(header(
            "authorization",
            format!("Bearer {WRITE_TOKEN}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(201).set_body_json(ack_body()))
        .mount(&server)
        .await;
    let ack = client_for(&server)
        .register_start(&start())
        .await
        .expect("the relay acknowledges");
    assert_eq!(ack.event_id, EVENT);
    assert_eq!(ack.state, "started");
}

#[tokio::test]
async fn a_conflict_is_its_own_outcome_not_a_generic_refusal() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": "event_id_conflict",
            "message": "already durable",
        })))
        .mount(&server)
        .await;
    let error = client_for(&server)
        .register_start(&start())
        .await
        .expect_err("a conflict is surfaced");
    assert_eq!(error, RelayClientError::Conflict);
}

#[tokio::test]
async fn a_completion_before_its_start_is_a_refusal_not_a_conflict() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(409).set_body_json(serde_json::json!({
            "error": "no_registered_start",
            "message": "no start",
        })))
        .mount(&server)
        .await;
    let error = client_for(&server)
        .register_start(&start())
        .await
        .expect_err("refused");
    assert_eq!(error, RelayClientError::Rejected { kind: "no_start" });
}

#[tokio::test]
async fn a_refused_credential_is_permanent_and_a_5xx_is_transient() {
    for (status, expected) in [
        (401u16, RelayClientError::Rejected { kind: "auth" }),
        (400, RelayClientError::Rejected { kind: "invalid" }),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/internal/v1/audit/request-starts"))
            .respond_with(ResponseTemplate::new(status))
            .mount(&server)
            .await;
        assert_eq!(
            client_for(&server)
                .register_start(&start())
                .await
                .expect_err("refused"),
            expected,
            "status {status}"
        );
    }

    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let error = client_for(&server)
        .register_start(&start())
        .await
        .expect_err("unavailable");
    assert!(matches!(error, RelayClientError::Unavailable { .. }));
}

#[tokio::test]
async fn an_unreachable_relay_is_unavailable_within_the_budget() {
    // A port nothing listens on: the client must give up inside its budget
    // rather than retrying forever.
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some("http://127.0.0.1:1".to_string()),
        write_token: SecretString::from(WRITE_TOKEN.to_string()),
        read_token: SecretString::from(READ_TOKEN.to_string()),
        start_timeout_ms: 200,
        completion_timeout_ms: 200,
        incomplete_grace_secs: 60,
    };
    let client =
        AuditRelayClient::from_config(&config, RelayClientMetrics::new()).expect("client builds");
    let started = std::time::Instant::now();
    let error = client
        .register_start(&start())
        .await
        .expect_err("unreachable");
    assert!(matches!(error, RelayClientError::Unavailable { .. }));
    assert!(
        started.elapsed() < Duration::from_secs(3),
        "the budget must bound the retry loop"
    );
}

#[tokio::test]
async fn the_read_endpoint_uses_the_read_token() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/internal/v1/audit/records"))
        .and(header(
            "authorization",
            format!("Bearer {READ_TOKEN}").as_str(),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"rows": []})))
        .mount(&server)
        .await;
    let query = crate::audit_relay::query::RecordsQueryV1 {
        scope: "all".to_string(),
        record_kind: "api_request".to_string(),
        from: "2026-07-30T12:00:00.000Z".to_string(),
        to: "2026-07-31T12:00:00.000Z".to_string(),
        limit: 10,
        ..Default::default()
    };
    let page = client_for(&server)
        .read_records(&query, Duration::from_millis(500))
        .await
        .expect("the relay answers");
    assert!(page.rows.is_empty());
}

#[tokio::test]
async fn the_client_never_renders_a_credential_in_debug_output() {
    let server = MockServer::start().await;
    let rendered = format!("{:?}", client_for(&server));
    assert!(!rendered.contains(WRITE_TOKEN));
    assert!(!rendered.contains(READ_TOKEN));
    assert!(rendered.contains("<redacted>"));
}

#[tokio::test]
async fn calls_are_counted_under_bounded_labels() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/request-starts"))
        .respond_with(ResponseTemplate::new(201).set_body_json(ack_body()))
        .mount(&server)
        .await;
    let metrics = RelayClientMetrics::new();
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(server.uri().trim_end_matches('/').to_string()),
        write_token: SecretString::from(WRITE_TOKEN.to_string()),
        read_token: SecretString::from(READ_TOKEN.to_string()),
        start_timeout_ms: 500,
        completion_timeout_ms: 800,
        incomplete_grace_secs: 60,
    };
    let client = AuditRelayClient::from_config(&config, metrics.clone()).expect("client builds");
    client.register_start(&start()).await.expect("acknowledged");
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.calls(RelayPhase::Start, RelayCallResult::Ack), 1);
    assert_eq!(
        snapshot.calls(RelayPhase::Start, RelayCallResult::Unavailable),
        0
    );
}
