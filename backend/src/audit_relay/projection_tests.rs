//! Projection tests: a stored row replays through the SHARED capture format, and
//! an incomplete record ships under the incomplete event name.

use super::*;
use crate::audit::event::{EVENT_NAME, INCOMPLETE_EVENT_NAME};
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;
use crate::audit::projection::EventLimits;
use crate::audit_relay::db::row::StoredRecord;
use crate::audit_relay::record::{RecordState, RelayRecordKind};
use crate::audit_relay::test_support::{completion, lifecycle, now, start};

const EVENT: &str = "11111111-1111-4111-8111-111111111111";

fn stored(kind: RelayRecordKind, state: RecordState, terminal: Option<Vec<u8>>) -> StoredRecord {
    StoredRecord {
        event_id: EVENT.to_string(),
        record_kind: kind,
        state,
        actor_id: Some(101),
        session_id: None,
        started_at: crate::audit_relay::protocol::format_instant(now()),
        terminal_at: Some(crate::audit_relay::protocol::format_instant(now())),
        completion_deadline_at: None,
        terminal_json: terminal,
        capture_attempts: 0,
        posthog_accepted_at: None,
        posthog_verified_at: None,
        last_delivery_code: None,
    }
}

fn limits() -> EventLimits {
    EventLimits::new(65_536)
}

#[test]
fn a_completed_request_projects_under_the_completed_event_name() {
    let body = serde_json::to_vec(&completion(EVENT, Some(101))).expect("encodes");
    let record = stored(
        RelayRecordKind::ApiRequest,
        RecordState::Complete,
        Some(body),
    );
    let event = capture_event(&record, limits()).expect("projects");
    assert_eq!(event.event, EVENT_NAME);
    assert_eq!(event.uuid, EVENT);
    assert_eq!(event.distinct_id, "github:101");
    assert_eq!(event.properties["status_code"], serde_json::json!(200));
}

#[test]
fn a_synthesized_incomplete_projects_under_the_incomplete_event_name() {
    let (terminal, _) =
        crate::audit_relay::incomplete::synthesize(&start(EVENT)).expect("synthesizes");
    let body = serde_json::to_vec(&terminal).expect("encodes");
    let record = stored(
        RelayRecordKind::ApiRequest,
        RecordState::Incomplete,
        Some(body),
    );
    let event = capture_event(&record, limits()).expect("projects");
    assert_eq!(event.event, INCOMPLETE_EVENT_NAME);
    assert_eq!(event.uuid, EVENT, "the SAME event id as the start");
    assert_eq!(event.properties["status_code"], serde_json::Value::Null);
    assert_eq!(
        event.properties["error_code"],
        serde_json::json!("request_incomplete")
    );
}

#[test]
fn a_lifecycle_row_projects_under_the_lifecycle_event_name() {
    let body = serde_json::to_vec(&lifecycle(EVENT, "sess-1")).expect("encodes");
    let record = stored(
        RelayRecordKind::SandboxLifecycle,
        RecordState::Complete,
        Some(body),
    );
    let event = capture_event(&record, limits()).expect("projects");
    assert_eq!(event.event, LIFECYCLE_EVENT_NAME);
    assert_eq!(event.properties["session_id"], serde_json::json!("sess-1"));
}

#[test]
fn a_row_without_a_terminal_projection_cannot_be_captured() {
    let record = stored(RelayRecordKind::ApiRequest, RecordState::Started, None);
    assert_eq!(
        capture_event(&record, limits()).expect_err("no terminal"),
        ProjectionError::NoTerminal
    );
}

#[test]
fn an_undecodable_body_is_named_not_guessed() {
    let record = stored(
        RelayRecordKind::ApiRequest,
        RecordState::Complete,
        Some(b"{not json".to_vec()),
    );
    assert_eq!(
        capture_event(&record, limits()).expect_err("undecodable"),
        ProjectionError::Undecodable
    );
}

#[test]
fn an_oversized_projection_is_refused_rather_than_truncated() {
    let mut oversized = completion(EVENT, Some(101));
    oversized.arguments.insert(
        "environment_name".to_string(),
        serde_json::json!("x".repeat(8_192)),
    );
    let body = serde_json::to_vec(&oversized).expect("encodes");
    let record = stored(
        RelayRecordKind::ApiRequest,
        RecordState::Complete,
        Some(body),
    );
    let error = capture_event(&record, EventLimits::new(4_096)).expect_err("too large");
    assert_eq!(error.as_str(), "contract");
}
