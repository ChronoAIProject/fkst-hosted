//! Decoding one PostHog result row into a source-neutral [`ActivityRecord`].
//!
//! Three rules, each of which exists because the alternative silently corrupts
//! the timeline:
//!
//! - **Columns are addressed by NAME, never by position.** A source that reorders
//!   or adds a column must not re-map every field onto the wrong one.
//! - **Unknown properties are ignored.** The projection is an allowlist: a row
//!   carrying something this build has never heard of contributes nothing, so a
//!   forward-compatible writer can never leak an unexpected value into a
//!   response.
//! - **A malformed row is rejected, never repaired.** A missing or wrong-typed
//!   REQUIRED field yields a [`RowError`]; the caller drops that row, counts it,
//!   and marks the page partial. Guessing a status, a timestamp, or an operation
//!   id would produce a record that reads as fact.
//!
//! Type coercion is deliberately lenient in ONE direction only: an integer that
//! arrives as a JSON string is accepted, because ClickHouse renders JSON property
//! values as text. Nothing else is coerced — a string where a timestamp belongs
//! is a rejected row, not a zero instant.

use std::collections::HashMap;

use k8s_openapi::chrono::{DateTime, Utc};
use serde_json::{Map, Value};

use crate::audit::event::{EVENT_NAME, INCOMPLETE_EVENT_NAME};
use crate::audit::lifecycle::LIFECYCLE_EVENT_NAME;

use super::record::{
    ActivityRecord, ActivitySourceKind, ApiRequestRecord, DeliveryState, RecordActor,
    RecordCorrelation, RecordPrincipal, SandboxLifecycleRecord,
};

/// Why one already-authorized row could not be decoded. Bounded: it names the
/// COLUMN, which is a compile-time constant, and never the value.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum RowError {
    #[error("row is missing the required column {column}")]
    Missing { column: &'static str },
    #[error("row column {column} has the wrong type")]
    WrongType { column: &'static str },
    #[error("row carries an event name outside the fixed audit contract")]
    UnknownEvent,
}

/// A name-addressed view over one result row.
pub struct RowView<'a> {
    columns: &'a HashMap<String, usize>,
    values: &'a [Value],
}

impl<'a> RowView<'a> {
    /// Bind a row to its column index.
    pub fn new(columns: &'a HashMap<String, usize>, values: &'a [Value]) -> Self {
        Self { columns, values }
    }

    /// The raw cell, or `None` when the column is absent or the row is short.
    fn cell(&self, column: &str) -> Option<&'a Value> {
        let index = *self.columns.get(column)?;
        self.values.get(index).filter(|value| !value.is_null())
    }

    fn string(&self, column: &str) -> Option<String> {
        match self.cell(column)? {
            Value::String(text) => Some(text.clone()),
            _ => None,
        }
    }

    fn required_string(&self, column: &'static str) -> Result<String, RowError> {
        match self.cell(column) {
            Some(Value::String(text)) if !text.is_empty() => Ok(text.clone()),
            Some(Value::String(_)) | None => Err(RowError::Missing { column }),
            Some(_) => Err(RowError::WrongType { column }),
        }
    }

    /// An integer, accepting the JSON-string rendering ClickHouse produces for
    /// property values.
    fn integer(&self, column: &str) -> Option<i64> {
        match self.cell(column)? {
            Value::Number(number) => number.as_i64(),
            Value::String(text) => text.trim().parse::<i64>().ok(),
            _ => None,
        }
    }

    fn unsigned(&self, column: &str) -> Option<u64> {
        self.integer(column)
            .and_then(|value| u64::try_from(value).ok())
    }

    fn status_code(&self, column: &str) -> Option<u16> {
        self.integer(column)
            .and_then(|value| u16::try_from(value).ok())
    }

    fn timestamp(&self, column: &str) -> Option<DateTime<Utc>> {
        let text = self.string(column)?;
        parse_timestamp(&text)
    }

    fn required_timestamp(&self, column: &'static str) -> Result<DateTime<Utc>, RowError> {
        match self.cell(column) {
            None => Err(RowError::Missing { column }),
            Some(Value::String(text)) => {
                parse_timestamp(text).ok_or(RowError::WrongType { column })
            }
            Some(_) => Err(RowError::WrongType { column }),
        }
    }

    fn object(&self, column: &str) -> Map<String, Value> {
        match self.cell(column) {
            Some(Value::Object(map)) => map.clone(),
            // ClickHouse renders a nested JSON property as text; parse it back
            // rather than dropping the operation's safe arguments entirely.
            Some(Value::String(text)) => match serde_json::from_str::<Value>(text) {
                Ok(Value::Object(map)) => map,
                _ => Map::new(),
            },
            _ => Map::new(),
        }
    }

    fn actor(&self) -> RecordActor {
        RecordActor {
            kind: self.string("actor_kind"),
            id: self.integer("actor_id"),
            login: self.string("actor_login"),
        }
    }

    fn principal(&self) -> RecordPrincipal {
        RecordPrincipal {
            kind: self.string("principal_kind"),
            id: self.string("principal_id"),
        }
    }

    fn correlation(&self) -> RecordCorrelation {
        RecordCorrelation {
            session_id: self.string("session_id"),
            repo_full_name: self.string("repo_full_name"),
            installation_id: self.integer("installation_id"),
            trigger_issue: self.integer("trigger_issue"),
            request_id: self.string("request_id"),
            webhook_delivery_id: self.string("webhook_delivery_id"),
        }
    }
}

/// Accept both the millisecond RFC3339 form the audit contract writes and the
/// `YYYY-MM-DD HH:MM:SS(.fff)` form ClickHouse renders a `DateTime64` in.
fn parse_timestamp(text: &str) -> Option<DateTime<Utc>> {
    let text = text.trim();
    if let Ok(parsed) = DateTime::parse_from_rfc3339(text) {
        return Some(parsed.with_timezone(&Utc));
    }
    let rfc3339ish = text.replacen(' ', "T", 1);
    let candidate = if rfc3339ish.ends_with('Z') || rfc3339ish.contains('+') {
        rfc3339ish
    } else {
        format!("{rfc3339ish}Z")
    };
    DateTime::parse_from_rfc3339(&candidate)
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// Decode one row from `source`.
///
/// `delivery_state` is supplied by the adapter: a row read back OUT of PostHog is
/// by definition query-visible, while a relay row carries its own outbox state.
pub fn decode(
    row: &RowView<'_>,
    source: ActivitySourceKind,
    delivery_state: DeliveryState,
) -> Result<ActivityRecord, RowError> {
    let event = row.required_string("event")?;
    match event.as_str() {
        EVENT_NAME | INCOMPLETE_EVENT_NAME => {
            decode_api_request(row, source, delivery_state).map(|record| {
                ActivityRecord::ApiRequest {
                    record: Box::new(record),
                    delivery_state,
                    source,
                }
            })
        }
        LIFECYCLE_EVENT_NAME => {
            decode_lifecycle(row).map(|record| ActivityRecord::SandboxLifecycle {
                record: Box::new(record),
                delivery_state,
                source,
            })
        }
        // The query allowlists three event names, so this is unreachable through
        // the fixed HogQL. It is still an error rather than a silent skip: a row
        // arriving here means the source answered something other than what was
        // asked, which an operator needs to see.
        _ => Err(RowError::UnknownEvent),
    }
}

fn decode_api_request(
    row: &RowView<'_>,
    _source: ActivitySourceKind,
    _delivery_state: DeliveryState,
) -> Result<ApiRequestRecord, RowError> {
    // The completion instant is the sort key. `row_timestamp` is the event's own
    // PostHog timestamp, which the capture projection sets to exactly this value,
    // so it is the correct fallback rather than a guess.
    let completed_at = row
        .timestamp("completed_at")
        .map(Ok)
        .unwrap_or_else(|| row.required_timestamp("row_timestamp"))?;
    Ok(ApiRequestRecord {
        event_id: row.required_string("event_id")?,
        request_id: row.string("request_id"),
        started_at: row.timestamp("started_at"),
        completed_at,
        method: row.required_string("method")?,
        route_template: row.required_string("route_template")?,
        operation_id: row.required_string("operation_id")?,
        actor: row.actor(),
        principal: row.principal(),
        arguments: row.object("arguments"),
        arguments_parse_status: row.string("arguments_parse_status"),
        status_code: row.status_code("status_code"),
        outcome: row.required_string("outcome")?,
        error_code: row.string("error_code"),
        duration_ms: row.unsigned("duration_ms"),
        correlation: row.correlation(),
    })
}

fn decode_lifecycle(row: &RowView<'_>) -> Result<SandboxLifecycleRecord, RowError> {
    let occurred_at = row
        .timestamp("occurred_at")
        .map(Ok)
        .unwrap_or_else(|| row.required_timestamp("row_timestamp"))?;
    Ok(SandboxLifecycleRecord {
        event_id: row.required_string("event_id")?,
        occurred_at,
        lifecycle_action: row.required_string("lifecycle_action")?,
        actor: row.actor(),
        principal: row.principal(),
        session_id: row.required_string("session_id")?,
        backend: row.string("backend"),
        runtime_id: row.string("runtime_id"),
        creator_id: row.integer("creator_id"),
        creator_login: row.string("creator_login"),
        trigger_author_id: row.integer("trigger_author_id"),
        trigger_author_login: row.string("trigger_author_login"),
        created_at: row.timestamp("created_at"),
        reason_code: row.string("reason_code"),
        correlation: row.correlation(),
    })
}

/// Build the `column name -> index` map from a response's `columns` array.
pub fn column_index(columns: &[String]) -> HashMap<String, usize> {
    columns
        .iter()
        .enumerate()
        .map(|(index, name)| (name.clone(), index))
        .collect()
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
