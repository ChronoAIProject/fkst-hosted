//! Relay-source tests: the mandatory constraint travels, delivery state is
//! carried through, and a source failure maps onto the documented split.

use std::sync::Arc;
use std::time::Duration;

use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use crate::operations::filters::{ActivityFilters, RecordKind, StatusClass};
use crate::operations::test_support::{all, authorized_session, mine, range};

use super::*;

fn source(server: &MockServer) -> RelayActivitySource {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::BestEffort,
        relay_url: Some(server.uri().trim_end_matches('/').to_string()),
        write_token: SecretString::from("write-secret".to_string()),
        read_token: SecretString::from("read-secret".to_string()),
        start_timeout_ms: 500,
        completion_timeout_ms: 500,
        incomplete_grace_secs: 60,
    };
    let client = Arc::new(
        AuditRelayClient::from_config(&config, RelayClientMetrics::new()).expect("client builds"),
    );
    RelayActivitySource::new(client, Duration::from_millis(500))
}

fn personal_query() -> SourceQuery {
    SourceQuery {
        constraint: mine(101, "alice", None),
        record_kind: RecordKind::ApiRequest,
        range: range(),
        filters: ActivityFilters::default(),
        cursor: None,
        fetch_limit: 51,
    }
}

fn relay_row(event_id: &str, delivery_state: &str) -> serde_json::Value {
    serde_json::json!({
        "event_id": event_id,
        "record_kind": "api_request",
        "state": "complete",
        "delivery_state": delivery_state,
        "sort_timestamp": "2026-07-31T11:59:00.000Z",
        "terminal": {
            "schema_version": 1,
            "event_id": event_id,
            "request_id": "req-1",
            "started_at": "2026-07-31T11:58:59.750Z",
            "completed_at": "2026-07-31T11:59:00.000Z",
            "method": "GET",
            "route_template": "/api/v1/overview",
            "operation_id": "canvas_overview",
            "arguments": {},
            "arguments_parse_status": "parsed",
            "actor_id": 101,
            "actor": {"kind": "github_user", "id": 101, "login": "alice", "authentication": "bearer"},
            "principal": {"kind": "github_user_token", "id": null},
            "status_code": 200,
            "outcome": "success",
            "error_code": null,
            "duration_ms": 250,
            "session_id": null,
            "correlation": {},
            "service_version": "0.2.3",
            "deployment_environment": "test"
        }
    })
}

#[test]
fn a_personal_query_carries_the_verified_actor_to_the_relay() {
    let wire = RelayActivitySource::build_query(&personal_query());
    assert_eq!(wire.scope, "mine");
    assert_eq!(wire.actor_id, Some(101));
    assert_eq!(wire.limit, 51);
    assert_eq!(wire.record_kind, "api_request");
}

#[test]
fn an_authorized_lifecycle_session_travels_with_the_constraint() {
    let mut query = personal_query();
    query.record_kind = RecordKind::All;
    query.constraint = mine(
        101,
        "alice",
        Some(authorized_session("sess-1", 101, "alice")),
    );
    let wire = RelayActivitySource::build_query(&query);
    assert_eq!(wire.lifecycle_session_id.as_deref(), Some("sess-1"));
    assert_eq!(wire.record_kind, "all");
}

#[test]
fn a_global_query_carries_no_narrowing_predicate() {
    let mut query = personal_query();
    query.constraint = all(7, "root");
    let wire = RelayActivitySource::build_query(&query);
    assert_eq!(wire.scope, "all");
    assert_eq!(wire.actor_id, None);
    assert_eq!(wire.lifecycle_session_id, None);
}

#[test]
fn every_normalized_filter_is_projected_onto_the_wire() {
    let mut query = personal_query();
    query.filters = ActivityFilters {
        actor_id: Some(101),
        actor_login: Some("alice".to_string()),
        operation_id: Some("canvas_overview".to_string()),
        method: Some("GET".to_string()),
        status_code: Some(200),
        status_class: Some(StatusClass::Success),
        outcome: Some(crate::audit::event::AuditOutcome::Success),
        session_id: Some("sess-1".to_string()),
        repo_full_name: Some("acme/site".to_string()),
        trigger_issue: Some(7),
        request_id: Some("req-1".to_string()),
    };
    let wire = RelayActivitySource::build_query(&query);
    assert_eq!(wire.filter_actor_id, Some(101));
    assert_eq!(wire.filter_actor_login.as_deref(), Some("alice"));
    assert_eq!(wire.filter_operation_id.as_deref(), Some("canvas_overview"));
    assert_eq!(wire.filter_method.as_deref(), Some("GET"));
    assert_eq!(wire.filter_status_code, Some(200));
    assert_eq!(wire.filter_status_low, Some(200));
    assert_eq!(wire.filter_status_high, Some(300));
    assert_eq!(wire.filter_outcome.as_deref(), Some("success"));
    assert_eq!(wire.filter_session_id.as_deref(), Some("sess-1"));
    assert_eq!(wire.filter_repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(wire.filter_trigger_issue, Some(7));
    assert_eq!(wire.filter_request_id.as_deref(), Some("req-1"));
}

#[tokio::test]
async fn the_relay_is_asked_with_the_viewer_predicate_in_the_query_string() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/internal/v1/audit/records"))
        .and(query_param("scope", "mine"))
        .and(query_param("actor_id", "101"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"rows": []})))
        .expect(1)
        .mount(&server)
        .await;
    let page = source(&server)
        .fetch(&personal_query())
        .await
        .expect("the relay answers");
    assert_eq!(page.raw_rows, 0);
    server.verify().await;
}

#[tokio::test]
async fn every_delivery_state_is_carried_through_to_the_merge() {
    for (wire, expected) in [
        ("queued", DeliveryState::Queued),
        (
            "accepted_pending_verification",
            DeliveryState::AcceptedPendingVerification,
        ),
        ("verified_in_posthog", DeliveryState::VerifiedInPosthog),
        ("incomplete", DeliveryState::Incomplete),
        ("dead_letter", DeliveryState::DeadLetter),
    ] {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/internal/v1/audit/records"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "rows": [relay_row("ev-1", wire)]
            })))
            .mount(&server)
            .await;
        let page = source(&server)
            .fetch(&personal_query())
            .await
            .expect("the relay answers");
        assert_eq!(page.records.len(), 1, "state `{wire}`");
        assert_eq!(page.records[0].delivery_state(), expected);
        assert_eq!(page.records[0].source(), ActivitySourceKind::Relay);
    }
}

#[tokio::test]
async fn an_unknown_delivery_state_drops_the_row_and_marks_the_page_partial() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/internal/v1/audit/records"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "rows": [relay_row("ev-1", "delivered")]
        })))
        .mount(&server)
        .await;
    let page = source(&server)
        .fetch(&personal_query())
        .await
        .expect("the relay answers");
    assert!(page.records.is_empty());
    assert_eq!(page.row_errors, 1);
    assert_eq!(page.raw_rows, 1, "an undecodable row still consumed a slot");
}

#[tokio::test]
async fn a_refused_credential_is_an_upstream_fault_and_an_outage_is_transient() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/internal/v1/audit/records"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;
    let error = source(&server)
        .fetch(&personal_query())
        .await
        .expect_err("refused");
    assert!(
        error.is_upstream_fault(),
        "an auth failure must not read as a transient blip"
    );

    let outage = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/internal/v1/audit/records"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&outage)
        .await;
    let error = source(&outage)
        .fetch(&personal_query())
        .await
        .expect_err("unavailable");
    assert!(!error.is_upstream_fault());
}
