//! Lifecycle-queue tests: admission, durable hand-off, and bounded overflow.

use std::sync::Arc;

use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit::lifecycle::{LifecycleAction, SandboxLifecycleV1};
use crate::audit::{AuditIdentity, ServiceIdentity};
use crate::runtime_identity::RuntimeBackendKind;

use super::*;
use crate::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};

fn event() -> SandboxLifecycleV1 {
    SandboxLifecycleV1::new(
        LifecycleAction::Created,
        RuntimeBackendKind::OpenSandbox,
        "sess-1",
        AuditIdentity::reconciler(None),
        ServiceIdentity {
            version: "0.2.3".to_string(),
            environment: "test".to_string(),
        },
    )
}

fn client_for(server: &MockServer) -> Arc<AuditRelayClient> {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(server.uri().trim_end_matches('/').to_string()),
        write_token: SecretString::from("write-secret".to_string()),
        read_token: SecretString::from("read-secret".to_string()),
        start_timeout_ms: 500,
        completion_timeout_ms: 500,
        incomplete_grace_secs: 60,
    };
    Arc::new(
        AuditRelayClient::from_config(&config, RelayClientMetrics::new()).expect("client builds"),
    )
}

#[tokio::test]
async fn a_lifecycle_event_reaches_the_relay_events_endpoint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/events"))
        .respond_with(ResponseTemplate::new(201).set_body_json(serde_json::json!({
            "event_id": "ev-1",
            "durable_at": "2026-07-31T12:00:00.000Z",
            "state": "complete",
        })))
        .expect(1)
        .mount(&server)
        .await;
    let queue = LifecycleRelayQueue::spawn(client_for(&server));
    assert!(queue.submit(&event()));
    // Give the drain task a moment to make the call before the server asserts.
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    drop(queue);
    server.verify().await;
}

#[tokio::test]
async fn a_relay_outage_does_not_block_the_producer() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/internal/v1/audit/events"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let queue = LifecycleRelayQueue::spawn(client_for(&server));
    // Admission is non-blocking: a reconciler must never wait on the relay.
    for _ in 0..10 {
        assert!(queue.submit(&event()));
    }
}
