//! Row-mapping tests: the scope columns are exact text, and a stored closed-enum
//! value this build does not know is a rejected row rather than a defaulted one.

use rusqlite::Connection;

use super::*;
use crate::audit_relay::db::DbError;
use crate::audit_relay::record::{RecordState, RelayRecordKind};

fn table() -> Connection {
    let mut connection = Connection::open_in_memory().expect("opens");
    crate::audit_relay::db::schema::migrate(&mut connection).expect("migrates");
    connection
}

fn insert(connection: &Connection, record_kind: &str, state: &str) {
    connection
        .execute(
            "INSERT INTO audit_records (
                event_id, schema_version, kind, request_id, operation_id, actor_id,
                session_id, record_kind, started_at, terminal_at, completion_deadline_at,
                state, start_json, terminal_json, capture_attempts, next_attempt_at,
                posthog_accepted_at, posthog_verified_at, last_delivery_code,
                created_at, updated_at
            ) VALUES (
                'ev-1', 1, 'request', 'req-1', 'canvas_overview', '101',
                'sess-1', ?1, '2026-07-31T12:00:00.000Z', '2026-07-31T12:00:01.000Z', NULL,
                ?2, X'7b7d', X'7b7d', 0, NULL,
                NULL, NULL, NULL,
                '2026-07-31T12:00:00.000Z', '2026-07-31T12:00:00.000Z'
            )",
            rusqlite::params![record_kind, state],
        )
        .expect("inserts");
}

#[test]
fn a_well_formed_row_decodes_with_its_scope_columns() {
    let connection = table();
    insert(&connection, "api_request", "complete");
    let record: StoredRecord = connection
        .query_row(
            &format!("SELECT {RECORD_COLUMNS} FROM audit_records"),
            [],
            StoredRecord::from_row,
        )
        .expect("decodes");
    assert_eq!(record.record_kind, RelayRecordKind::ApiRequest);
    assert_eq!(record.state, RecordState::Complete);
    assert_eq!(record.actor_id, Some(101));
    assert_eq!(record.session_id.as_deref(), Some("sess-1"));
    assert_eq!(record.sort_timestamp(), Some("2026-07-31T12:00:01.000Z"));
}

#[test]
fn an_unknown_stored_state_is_rejected_not_defaulted() {
    let connection = table();
    insert(&connection, "api_request", "delivered");
    let error = connection
        .query_row(
            &format!("SELECT {RECORD_COLUMNS} FROM audit_records"),
            [],
            StoredRecord::from_row,
        )
        .expect_err("an unknown state is a decode failure");
    assert_eq!(decode_error(&error), DbError::Internal("decode"));
}

#[test]
fn an_unknown_stored_record_kind_is_rejected() {
    let connection = table();
    insert(&connection, "telemetry", "complete");
    assert!(connection
        .query_row(
            &format!("SELECT {RECORD_COLUMNS} FROM audit_records"),
            [],
            StoredRecord::from_row,
        )
        .is_err());
}

#[test]
fn the_actor_column_is_the_decimal_rendering_of_the_id() {
    assert_eq!(actor_column(Some(101)).as_deref(), Some("101"));
    assert_eq!(actor_column(Some(-1)).as_deref(), Some("-1"));
    assert_eq!(actor_column(None), None);
}
