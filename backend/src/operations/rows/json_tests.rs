//! Adapter tests: the stored, structured relay body decodes through the SAME
//! typed row contract as a flat PostHog result row.

use serde_json::json;

use super::super::{decode, RowCells};
use super::*;
use crate::operations::record::{ActivityRecord, ActivitySourceKind, DeliveryState};

fn api_body() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "event_id": "ev-1",
        "request_id": "req-1",
        "started_at": "2026-07-31T11:58:59.750Z",
        "completed_at": "2026-07-31T11:59:00.000Z",
        "method": "GET",
        "route_template": "/api/v1/overview",
        "operation_id": "canvas_overview",
        "arguments": {"broader_visibility_requested": false},
        "arguments_parse_status": "parsed",
        "actor_id": 101,
        "actor": {"kind": "github_user", "id": 101, "login": "alice", "authentication": "bearer"},
        "principal": {"kind": "github_user_token", "id": "github_user_token"},
        "status_code": 200,
        "outcome": "success",
        "error_code": null,
        "duration_ms": 250,
        "session_id": "sess-1",
        "correlation": {
            "session_id": "sess-1",
            "repo_full_name": "acme/site",
            "installation_id": 4242,
            "trigger_issue": 7,
            "webhook_delivery_id": "d-9f3a"
        },
        "service_version": "0.2.3",
        "deployment_environment": "test"
    })
}

fn lifecycle_body() -> serde_json::Value {
    json!({
        "schema_version": 1,
        "event_id": "ev-2",
        "occurred_at": "2026-07-31T11:00:00.000Z",
        "lifecycle_action": "created",
        "actor": {"kind": "system", "id": null, "login": null, "authentication": "internal"},
        "principal": {"kind": "reconciler", "id": "reconciler"},
        "session_id": "sess-1",
        "backend": "opensandbox",
        "runtime_id": "sbx-1",
        "runtime_created_at": "2026-07-31T10:59:00.000Z",
        "creator_id": 101,
        "creator_login": "alice",
        "trigger_author_id": 101,
        "trigger_author_login": "alice",
        "correlation": {
            "session_id": "sess-1",
            "repo_full_name": "acme/site",
            "installation_id": 4242,
            "trigger_issue": 7,
            "request_id": "req-9"
        },
        "reason_code": null,
        "service_version": "0.2.3",
        "deployment_environment": "test"
    })
}

#[test]
fn a_stored_api_body_decodes_into_the_source_neutral_record() {
    let body = api_body();
    let view = JsonRowView::new(&body, "api_request", "2026-07-31T11:59:00.000Z");
    let record = decode(&view, ActivitySourceKind::Relay, DeliveryState::Queued)
        .expect("the stored body decodes");
    let ActivityRecord::ApiRequest { record, .. } = record else {
        panic!("an api_request body must decode as an API record");
    };
    assert_eq!(record.event_id, "ev-1");
    assert_eq!(record.actor.id, Some(101));
    assert_eq!(record.actor.login.as_deref(), Some("alice"));
    assert_eq!(record.principal.id.as_deref(), Some("github_user_token"));
    assert_eq!(
        record.correlation.repo_full_name.as_deref(),
        Some("acme/site")
    );
    assert_eq!(record.correlation.installation_id, Some(4242));
    assert_eq!(
        record.correlation.webhook_delivery_id.as_deref(),
        Some("d-9f3a")
    );
    assert_eq!(record.status_code, Some(200));
    assert_eq!(record.duration_ms, Some(250));
    assert_eq!(record.arguments.len(), 1);
}

#[test]
fn a_stored_lifecycle_body_decodes_into_the_source_neutral_record() {
    let body = lifecycle_body();
    let view = JsonRowView::new(&body, "sandbox_lifecycle", "2026-07-31T11:00:00.000Z");
    let record = decode(&view, ActivitySourceKind::Relay, DeliveryState::Queued)
        .expect("the stored body decodes");
    let ActivityRecord::SandboxLifecycle { record, .. } = record else {
        panic!("a lifecycle body must decode as a lifecycle record");
    };
    assert_eq!(record.session_id, "sess-1");
    assert_eq!(record.lifecycle_action, "created");
    assert_eq!(record.backend.as_deref(), Some("opensandbox"));
    assert_eq!(record.creator_id, Some(101));
    assert!(
        record.created_at.is_some(),
        "runtime_created_at maps to created_at"
    );
    assert_eq!(record.correlation.request_id.as_deref(), Some("req-9"));
}

#[test]
fn an_incomplete_record_decodes_under_the_incomplete_event_name() {
    let mut body = api_body();
    body["outcome"] = json!("incomplete");
    body["status_code"] = serde_json::Value::Null;
    body["error_code"] = json!("request_incomplete");
    let view = JsonRowView::new(&body, "api_request", "2026-07-31T11:59:00.000Z");
    assert_eq!(
        view.cell("event").and_then(serde_json::Value::as_str),
        Some(crate::audit::event::INCOMPLETE_EVENT_NAME)
    );
    let record = decode(&view, ActivitySourceKind::Relay, DeliveryState::Incomplete)
        .expect("an incomplete record still decodes");
    let ActivityRecord::ApiRequest { record, .. } = record else {
        panic!("still an API record");
    };
    assert_eq!(record.status_code, None, "no fabricated status");
    assert_eq!(record.outcome, "incomplete");
}

#[test]
fn a_null_field_reads_as_absent() {
    let body = api_body();
    let view = JsonRowView::new(&body, "api_request", "2026-07-31T11:59:00.000Z");
    assert_eq!(view.cell("error_code"), None);
    assert_eq!(view.cell("this_field_does_not_exist"), None);
}

#[test]
fn a_missing_required_field_is_a_rejected_row_not_a_guess() {
    let mut body = api_body();
    body.as_object_mut().expect("object").remove("operation_id");
    let view = JsonRowView::new(&body, "api_request", "2026-07-31T11:59:00.000Z");
    assert!(decode(&view, ActivitySourceKind::Relay, DeliveryState::Queued).is_err());
}
