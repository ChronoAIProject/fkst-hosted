//! `GET /internal/v1/audit/records`: the scoped read.
//!
//! The handler is deliberately thin and paranoid. It authenticates with the READ
//! token — never the write token — resolves the server-constructed scope through
//! [`ResolvedScope::resolve`] (which refuses a `mine` scope missing its actor
//! id), bounds the window and the page, and then hands a value that cannot
//! express "no predicate" to [`super::super::db::read::build`].
//!
//! There is no post-fetch filtering here and there must never be one: the SQL
//! applies the scope before its own `LIMIT`, so the page boundary is decided by
//! rows the caller may already see (epic `AUTH-06`). A `started` row is never
//! returned — it has no terminal projection, and the read surface does not invent
//! outcomes.

use axum::extract::{Query, State};
use axum::http::HeaderMap;
use axum::Json;
use k8s_openapi::chrono::{DateTime, Duration, Utc};

use super::super::auth::TokenRole;
use super::super::db::read as db_read;
use super::super::db::row::StoredRecord;
use super::super::metrics::{IngressKind, IngressResult};
use super::super::query::{RecordRowV1, RecordsPageV1, RecordsQueryV1, ResolvedScope};
use super::error::{RelayError, RelayResult};
use super::RelayState;

/// The record kinds the read accepts, mirroring the public activity query.
const ACCEPTED_KINDS: [&str; 3] = ["api_request", "sandbox_lifecycle", "all"];

pub async fn get_records(
    State(state): State<RelayState>,
    headers: HeaderMap,
    Query(query): Query<RecordsQueryV1>,
) -> RelayResult<Json<RecordsPageV1>> {
    let outcome = run(&state, &headers, query).await;
    let result = match &outcome {
        // `Served`, never `Created`: a read commits nothing, and counting it as a
        // creation would make `{kind="records_read",result="created"}` contradict
        // the exposition's own help text.
        Ok(_) => IngressResult::Served,
        Err(error) => error.ingress_result(),
    };
    state
        .metrics
        .record_ingress(IngressKind::RecordsRead, result);
    outcome
}

async fn run(
    state: &RelayState,
    headers: &HeaderMap,
    query: RecordsQueryV1,
) -> RelayResult<Json<RecordsPageV1>> {
    state
        .tokens
        .authorize(headers, TokenRole::Read)
        .map_err(|_| RelayError::Unauthorized)?;

    let scope = ResolvedScope::resolve(&query.scope_wire()).ok_or(RelayError::Invalid(
        "scope must be `mine` with an actor id, or `all`",
    ))?;
    if !ACCEPTED_KINDS.contains(&query.record_kind.as_str()) {
        return Err(RelayError::Invalid(
            "record_kind must be api_request, sandbox_lifecycle, or all",
        ));
    }
    let (from, to) = window(&query, state.config.max_range_days)?;
    if query.cursor_timestamp.is_some() != query.cursor_event_id.is_some() {
        return Err(RelayError::Invalid(
            "a cursor needs both its timestamp and its event id",
        ));
    }
    let cursor = match (&query.cursor_timestamp, &query.cursor_event_id) {
        (Some(timestamp), Some(event_id)) => Some((
            parse_instant(timestamp).ok_or(RelayError::Invalid(
                "cursor_timestamp must be an RFC3339 UTC timestamp",
            ))?,
            event_id.clone(),
        )),
        _ => None,
    };
    // Clamped, not rejected: the control plane already applies its own page
    // ceiling, and answering a slightly smaller page is strictly safer than
    // failing a read an operator is depending on.
    let limit = query.limit.clamp(1, state.config.max_read_rows);
    // The PARSED instants go to the SQL, never the caller's own bytes: the stored
    // column is text, so two equally valid renderings of the same instant do not
    // compare equal — see `db::read`.
    let read_window = db_read::ReadWindow::new(from, to, cursor);

    let read = db_read::build(&query, &scope, limit, &read_window);
    let records = state
        .db
        .read(move |connection| db_read::fetch(connection, &read))
        .await?;
    tracing::debug!(
        scope = scope.as_str(),
        record_kind = %query.record_kind,
        rows = records.len(),
        "audit relay: answered a scoped read"
    );
    Ok(Json(RecordsPageV1 {
        rows: records.iter().filter_map(project).collect(),
    }))
}

/// Validate the window against the configured maximum span.
fn window(
    query: &RecordsQueryV1,
    max_range_days: u64,
) -> RelayResult<(DateTime<Utc>, DateTime<Utc>)> {
    let from = parse_instant(&query.from)
        .ok_or(RelayError::Invalid("from must be an RFC3339 UTC timestamp"))?;
    let to = parse_instant(&query.to)
        .ok_or(RelayError::Invalid("to must be an RFC3339 UTC timestamp"))?;
    if from >= to {
        return Err(RelayError::Invalid("from must be strictly before to"));
    }
    let max = Duration::days(i64::try_from(max_range_days).unwrap_or(i64::MAX));
    if to - from > max {
        return Err(RelayError::Invalid(
            "the requested range exceeds the relay's maximum span",
        ));
    }
    Ok((from, to))
}

fn parse_instant(raw: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(raw.trim())
        .ok()
        .map(|parsed| parsed.with_timezone(&Utc))
}

/// Project one stored row onto the wire.
///
/// A row without a terminal projection or with an unreadable body contributes
/// nothing: the read surface echoes what was committed and never reconstructs it.
fn project(record: &StoredRecord) -> Option<RecordRowV1> {
    let terminal_json = record.terminal_json.as_deref()?;
    let sort_timestamp = record.sort_timestamp()?.to_string();
    let terminal = match serde_json::from_slice::<serde_json::Value>(terminal_json) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(
                state = record.state.as_str(),
                "audit relay: dropping a stored record whose body is not readable JSON"
            );
            return None;
        }
    };
    Some(RecordRowV1 {
        event_id: record.event_id.clone(),
        record_kind: record.record_kind.as_str().to_string(),
        state: record.state.as_str().to_string(),
        delivery_state: record.state.delivery_state().to_string(),
        sort_timestamp,
        terminal,
    })
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
