//! Row-decoding tests: name addressing, column reorder, lenient integer
//! rendering, unknown properties, and the rejections that must NOT be repaired.

use serde_json::json;

use super::{column_index, decode, RowError, RowView};
use crate::audit::event::EVENT_NAME;
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;
use crate::operations::record::{ActivityRecord, ActivitySourceKind, DeliveryState};

/// A complete API-request row, as `(columns, values)`.
fn api_row() -> (Vec<String>, Vec<serde_json::Value>) {
    let pairs: Vec<(&str, serde_json::Value)> = vec![
        ("event", json!(EVENT_NAME)),
        ("row_timestamp", json!("2026-07-31T11:59:00.000Z")),
        ("event_id", json!("ev-1")),
        ("request_id", json!("req-1")),
        ("started_at", json!("2026-07-31T11:58:59.750Z")),
        ("completed_at", json!("2026-07-31T11:59:00.000Z")),
        ("method", json!("GET")),
        ("route_template", json!("/api/v1/overview")),
        ("operation_id", json!("canvas_overview")),
        ("arguments", json!({"broader_visibility_requested": false})),
        ("arguments_parse_status", json!("parsed")),
        ("status_code", json!(200)),
        ("outcome", json!("success")),
        ("error_code", serde_json::Value::Null),
        ("duration_ms", json!(12)),
        ("actor_kind", json!("github_user")),
        ("actor_id", json!(101)),
        ("actor_login", json!("alice")),
        ("principal_kind", json!("github_user_token")),
        ("session_id", json!("sess-1")),
        ("repo_full_name", json!("acme/site")),
        ("installation_id", json!(4242)),
        ("trigger_issue", json!(7)),
    ];
    (
        pairs.iter().map(|(name, _)| (*name).to_string()).collect(),
        pairs.into_iter().map(|(_, value)| value).collect(),
    )
}

fn lifecycle_row() -> (Vec<String>, Vec<serde_json::Value>) {
    let pairs: Vec<(&str, serde_json::Value)> = vec![
        ("event", json!(LIFECYCLE_EVENT_NAME)),
        ("row_timestamp", json!("2026-07-31T11:00:00.000Z")),
        ("event_id", json!("ev-2")),
        ("occurred_at", json!("2026-07-31T11:00:00.000Z")),
        ("lifecycle_action", json!("created")),
        ("session_id", json!("sess-1")),
        ("backend", json!("kubernetes")),
        ("runtime_id", json!("fkst-sess-1")),
        ("creator_id", json!("101")),
        ("creator_login", json!("alice")),
        ("actor_kind", json!("system")),
        ("principal_kind", json!("reconciler")),
        ("principal_id", json!("reconciler")),
        ("reason_code", json!("idle")),
    ];
    (
        pairs.iter().map(|(name, _)| (*name).to_string()).collect(),
        pairs.into_iter().map(|(_, value)| value).collect(),
    )
}

#[test]
fn a_complete_api_row_decodes_into_the_neutral_record() {
    let (columns, values) = api_row();
    let index = column_index(&columns);
    let record = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes");
    let ActivityRecord::ApiRequest { record, .. } = record else {
        panic!("expected an api-request record");
    };
    assert_eq!(record.event_id, "ev-1");
    assert_eq!(record.method, "GET");
    assert_eq!(record.operation_id, "canvas_overview");
    assert_eq!(record.status_code, Some(200));
    assert_eq!(record.duration_ms, Some(12));
    assert_eq!(record.actor.id, Some(101));
    assert_eq!(record.correlation.session_id.as_deref(), Some("sess-1"));
    assert_eq!(record.correlation.installation_id, Some(4242));
    assert_eq!(
        record.arguments.get("broader_visibility_requested"),
        Some(&json!(false))
    );
}

#[test]
fn a_complete_lifecycle_row_decodes_into_the_neutral_record() {
    let (columns, values) = lifecycle_row();
    let index = column_index(&columns);
    let record = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes");
    let ActivityRecord::SandboxLifecycle { record, .. } = record else {
        panic!("expected a lifecycle record");
    };
    assert_eq!(record.session_id, "sess-1");
    assert_eq!(record.lifecycle_action, "created");
    assert_eq!(record.backend.as_deref(), Some("kubernetes"));
    assert_eq!(
        record.creator_id,
        Some(101),
        "an integer rendered as a JSON string is accepted; ClickHouse does that"
    );
    assert_eq!(record.reason_code.as_deref(), Some("idle"));
}

/// Columns are addressed by NAME: reordering them must change nothing.
#[test]
fn a_reordered_column_list_decodes_identically() {
    let (columns, values) = api_row();
    let index = column_index(&columns);
    let straight = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes");

    let mut reordered_columns = columns.clone();
    let mut reordered_values = values.clone();
    reordered_columns.reverse();
    reordered_values.reverse();
    let reordered_index = column_index(&reordered_columns);
    let reordered = decode(
        &RowView::new(&reordered_index, &reordered_values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes");
    assert_eq!(straight, reordered);
}

/// A forward-compatible writer can add properties; they contribute nothing.
#[test]
fn unknown_columns_are_ignored() {
    let (mut columns, mut values) = api_row();
    columns.push("some_future_property".to_string());
    values.push(json!("whatever it is"));
    let index = column_index(&columns);
    let record = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes");
    let rendered = format!("{record:?}");
    assert!(!rendered.contains("whatever it is"), "{rendered}");
}

#[test]
fn a_missing_required_column_rejects_the_row_rather_than_guessing() {
    for missing in [
        "event_id",
        "method",
        "route_template",
        "operation_id",
        "outcome",
    ] {
        let (columns, values) = api_row();
        let kept: Vec<usize> = columns
            .iter()
            .enumerate()
            .filter(|(_, name)| name.as_str() != missing)
            .map(|(index, _)| index)
            .collect();
        let columns: Vec<String> = kept.iter().map(|i| columns[*i].clone()).collect();
        let values: Vec<serde_json::Value> = kept.iter().map(|i| values[*i].clone()).collect();
        let index = column_index(&columns);
        let error = decode(
            &RowView::new(&index, &values),
            ActivitySourceKind::Posthog,
            DeliveryState::VerifiedInPosthog,
        )
        .expect_err(missing);
        assert_eq!(error, RowError::Missing { column: missing });
    }
}

#[test]
fn a_wrongly_typed_required_column_rejects_the_row() {
    let (columns, mut values) = api_row();
    let index = column_index(&columns);
    let method_at = index["method"];
    values[method_at] = json!(42);
    let error = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect_err("a numeric method is not a method");
    assert_eq!(error, RowError::WrongType { column: "method" });
}

#[test]
fn an_unknown_event_name_is_an_error_not_a_silent_skip() {
    let (columns, mut values) = api_row();
    let index = column_index(&columns);
    values[index["event"]] = json!("$pageview");
    let error = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect_err("only the fixed audit contracts decode");
    assert_eq!(error, RowError::UnknownEvent);
}

/// ClickHouse renders `DateTime64` without the `T`/`Z`; both forms must parse,
/// and neither may be invented when absent.
#[test]
fn clickhouse_timestamp_rendering_is_accepted_and_falls_back_to_the_event_timestamp() {
    let (columns, mut values) = api_row();
    let index = column_index(&columns);
    values[index["completed_at"]] = serde_json::Value::Null;
    values[index["row_timestamp"]] = json!("2026-07-31 11:59:00.000");
    let record = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect("decodes from the event timestamp");
    assert_eq!(
        record
            .sort_timestamp()
            .to_rfc3339_opts(k8s_openapi::chrono::SecondsFormat::Millis, true),
        "2026-07-31T11:59:00.000Z"
    );

    values[index["row_timestamp"]] = serde_json::Value::Null;
    let error = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect_err("with no instant at all the row is rejected, never zeroed");
    assert_eq!(
        error,
        RowError::Missing {
            column: "row_timestamp"
        }
    );
}

/// A short row (fewer values than columns) must not panic or mis-address.
#[test]
fn a_short_row_is_rejected_rather_than_indexed_out_of_bounds() {
    let (columns, _) = api_row();
    let index = column_index(&columns);
    let values = vec![json!(EVENT_NAME)];
    let error = decode(
        &RowView::new(&index, &values),
        ActivitySourceKind::Posthog,
        DeliveryState::VerifiedInPosthog,
    )
    .expect_err("a truncated row cannot decode");
    assert!(matches!(error, RowError::Missing { .. }), "{error:?}");
}
