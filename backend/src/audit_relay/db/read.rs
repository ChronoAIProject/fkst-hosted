//! The scoped read: fixed SQL whose visibility predicate is applied BEFORE
//! `LIMIT`, on an index built for it.
//!
//! **A request cannot contribute one character to the query text.** Every
//! predicate here is a `&'static str` fragment selected by the PRESENCE of an
//! already-validated field; every value travels as a bound parameter. This is the
//! same argument [`crate::operations::hogql`] makes for PostHog, and it has to be
//! true in both sources or the read surface has two different security models.
//!
//! ## The parentheses in the `all` branch are load-bearing
//!
//! ```text
//! (   ( record_kind = 'api_request'      AND actor_id = ?viewer   )
//!  OR ( record_kind = 'sandbox_lifecycle' AND session_id = ?session ) )
//! ```
//!
//! Without the outer pair, `AND … OR …` would bind so that the lifecycle branch
//! escaped the time window and the actor predicate — turning "my calls plus this
//! session's system events" into "everybody's calls". It is asserted by a test.
//!
//! ## A personal lifecycle query with no authorized session matches nothing
//!
//! The control plane refuses that shape before it ever gets here, but the SQL
//! still emits an explicitly false predicate rather than omitting the session
//! clause: a missing predicate would silently widen the query, which is the exact
//! failure mode this layer exists to make impossible.
//!
//! ## Every bound is NORMALIZED before it is compared
//!
//! `terminal_at` is stored as [`super::super::protocol::format_instant`] text and
//! compared with SQLite's lexicographic TEXT ordering, which is only equivalent
//! to instant ordering when both sides use the same rendering. A caller sending
//! the equally valid `2026-01-01T00:00:00+00:00` would otherwise sort BELOW the
//! stored `2026-01-01T00:00:00.000Z` (`.` < `Z`) and silently lose a page of
//! rows. [`ReadWindow`] therefore carries instants the handler already parsed,
//! re-rendered in the one canonical form — the caller's own bytes never reach the
//! comparison.
//!
//! ## `started` rows are never returned
//!
//! A registered-but-unfinished request has no terminal projection, so there is no
//! honest status, outcome, or completion instant to show. It becomes visible as
//! `incomplete` once its deadline plus grace elapses — which is what the durable
//! start guarantees. `terminal_at IS NOT NULL` is therefore part of the fixed
//! predicate, not a filter a caller can switch off.

use k8s_openapi::chrono::{DateTime, Utc};
use rusqlite::types::Value as SqlValue;
use rusqlite::Connection;

use super::super::protocol::format_instant;
use super::super::query::{RecordsQueryV1, ResolvedScope};
use super::row::{StoredRecord, RECORD_COLUMNS};
use super::DbError;

/// The total order every page is cut from. `event_id` breaks ties so rows
/// sharing a millisecond still page deterministically — the same order the
/// PostHog source applies.
const ORDER_BY: &str = "ORDER BY terminal_at DESC, event_id DESC";

/// Record kinds a query may ask for, mirroring
/// [`crate::operations::filters::RecordKind`].
const KIND_API_REQUEST: &str = "api_request";
const KIND_SANDBOX_LIFECYCLE: &str = "sandbox_lifecycle";
const KIND_ALL: &str = "all";

/// A rendered read: fixed text plus the ordered bound values.
#[derive(Debug)]
pub struct ScopedRead {
    pub sql: String,
    pub values: Vec<SqlValue>,
}

/// The already-parsed bounds of one read, re-rendered in the storage form.
///
/// It exists so the SQL cannot be handed a caller's raw timestamp bytes; see the
/// module docs for the page a mismatched rendering silently drops.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReadWindow {
    from: String,
    to: String,
    cursor: Option<(String, String)>,
}

impl ReadWindow {
    /// Normalize a validated window and its optional keyset cursor.
    pub fn new(
        from: DateTime<Utc>,
        to: DateTime<Utc>,
        cursor: Option<(DateTime<Utc>, String)>,
    ) -> Self {
        Self {
            from: format_instant(from),
            to: format_instant(to),
            cursor: cursor.map(|(timestamp, event_id)| (format_instant(timestamp), event_id)),
        }
    }
}

/// Build the fixed SQL for one already-authorized scope.
///
/// `limit` is the caller's fetch size, already clamped by the handler against
/// `FKST_AUDIT_RELAY_MAX_READ_ROWS`; `window` carries the normalized bounds.
pub fn build(
    query: &RecordsQueryV1,
    scope: &ResolvedScope,
    limit: u32,
    window: &ReadWindow,
) -> ScopedRead {
    let mut values: Vec<SqlValue> = Vec::new();
    let mut predicates: Vec<String> = vec![
        // Terminal-only: see the module docs.
        "terminal_at IS NOT NULL".to_string(),
        format!("terminal_at >= {}", bind_text(&mut values, &window.from)),
        format!("terminal_at < {}", bind_text(&mut values, &window.to)),
        visibility(query, scope, &mut values),
    ];
    predicates.extend(filters(query, &mut values));
    if let Some((timestamp, event_id)) = &window.cursor {
        let timestamp = bind_text(&mut values, timestamp);
        let event_id_a = bind_text(&mut values, event_id);
        // Strictly after the cursor in the descending total order, so pages tile
        // with no overlap and no gap.
        predicates.push(format!(
            "(terminal_at < {timestamp} OR (terminal_at = {timestamp} AND event_id < {event_id_a}))"
        ));
    }
    let limit_placeholder = bind_integer(&mut values, i64::from(limit));

    ScopedRead {
        sql: format!(
            "SELECT {RECORD_COLUMNS} FROM audit_records WHERE {} {ORDER_BY} LIMIT {limit_placeholder}",
            predicates.join(" AND ")
        ),
        values,
    }
}

/// Execute a built read.
pub fn fetch(connection: &Connection, read: &ScopedRead) -> Result<Vec<StoredRecord>, DbError> {
    let mut statement = connection
        .prepare(&read.sql)
        .map_err(|error| super::classify(&error))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(read.values.iter()), |row| {
            StoredRecord::from_row(row)
        })
        .map_err(|error| super::classify(&error))?;
    let mut records = Vec::new();
    for row in rows {
        match row {
            Ok(record) => records.push(record),
            // One undecodable stored row must not hide every well-formed one; it
            // is dropped with a bounded reason and the page continues.
            Err(error) => tracing::warn!(
                reason = super::row::decode_error(&error).as_str(),
                "audit relay: dropping an undecodable stored record"
            ),
        }
    }
    Ok(records)
}

/// The engine's plan for a built read. Used by the query-plan test that proves
/// the scope predicate is served by an index rather than by post-fetch filtering.
pub fn explain(connection: &Connection, read: &ScopedRead) -> Result<Vec<String>, DbError> {
    let mut statement = connection
        .prepare(&format!("EXPLAIN QUERY PLAN {}", read.sql))
        .map_err(|error| super::classify(&error))?;
    let rows = statement
        .query_map(rusqlite::params_from_iter(read.values.iter()), |row| {
            row.get::<_, String>(3)
        })
        .map_err(|error| super::classify(&error))?;
    let mut plan = Vec::new();
    for row in rows {
        plan.push(row.map_err(|error| super::classify(&error))?);
    }
    Ok(plan)
}

/// The mandatory row-visibility predicate.
fn visibility(query: &RecordsQueryV1, scope: &ResolvedScope, values: &mut Vec<SqlValue>) -> String {
    match query.record_kind.as_str() {
        KIND_SANDBOX_LIFECYCLE => lifecycle_branch(scope, values),
        KIND_ALL => format!(
            // The outer parentheses are load-bearing; see the module docs.
            "({} OR {})",
            api_request_branch(scope, values),
            lifecycle_branch(scope, values)
        ),
        // `api_request`, and the narrowest branch for anything the handler has
        // already rejected — defaulting to the widest would be the wrong way to
        // fail.
        _ => api_request_branch(scope, values),
    }
}

/// API-request rows: the caller's own, in personal scope.
fn api_request_branch(scope: &ResolvedScope, values: &mut Vec<SqlValue>) -> String {
    let mut parts = vec![format!(
        "record_kind = {}",
        bind_text(values, KIND_API_REQUEST)
    )];
    if let ResolvedScope::Mine {
        actor_id,
        lifecycle_session_id,
    } = scope
    {
        // The scope column is TEXT (see `super::row`), so the viewer id binds as
        // its decimal rendering and the indexed seek is exact.
        parts.push(format!(
            "actor_id = {}",
            bind_text(values, &actor_id.to_string())
        ));
        if let Some(session_id) = lifecycle_session_id {
            parts.push(format!("session_id = {}", bind_text(values, session_id)));
        }
    }
    format!("({})", parts.join(" AND "))
}

/// System lifecycle rows: only for the one separately authorized session.
fn lifecycle_branch(scope: &ResolvedScope, values: &mut Vec<SqlValue>) -> String {
    let mut parts = vec![format!(
        "record_kind = {}",
        bind_text(values, KIND_SANDBOX_LIFECYCLE)
    )];
    if let ResolvedScope::Mine {
        lifecycle_session_id,
        ..
    } = scope
    {
        match lifecycle_session_id {
            Some(session_id) => {
                parts.push(format!("session_id = {}", bind_text(values, session_id)))
            }
            // Explicitly false rather than omitted: see the module docs.
            None => parts.push("1 = 0".to_string()),
        }
    }
    format!("({})", parts.join(" AND "))
}

/// The optional narrowing predicates, each selected by the PRESENCE of an
/// already-validated filter.
///
/// Four of them read an indexed column directly; the rest read the stored,
/// already-sanitized terminal body through `json_extract`. Both forms run inside
/// the same `WHERE`, so every filter is applied before `LIMIT` — a filter
/// evaluated after the page was cut would let the page boundary depend on rows
/// the caller did not ask for.
fn filters(query: &RecordsQueryV1, values: &mut Vec<SqlValue>) -> Vec<String> {
    let mut predicates = Vec::new();
    if let Some(actor_id) = query.filter_actor_id {
        predicates.push(format!(
            "actor_id = {}",
            bind_text(values, &actor_id.to_string())
        ));
    }
    if let Some(login) = &query.filter_actor_login {
        predicates.push(format!(
            "{} = {}",
            json_path("$.actor.login"),
            bind_text(values, login)
        ));
    }
    if let Some(operation_id) = &query.filter_operation_id {
        predicates.push(format!(
            "operation_id = {}",
            bind_text(values, operation_id)
        ));
    }
    if let Some(method) = &query.filter_method {
        predicates.push(format!(
            "{} = {}",
            json_path("$.method"),
            bind_text(values, method)
        ));
    }
    if let Some(status_code) = query.filter_status_code {
        predicates.push(format!(
            "{} = {}",
            json_path("$.status_code"),
            bind_integer(values, i64::from(status_code))
        ));
    }
    if let (Some(low), Some(high)) = (query.filter_status_low, query.filter_status_high) {
        predicates.push(format!(
            "({field} >= {} AND {field} < {})",
            bind_integer(values, i64::from(low)),
            bind_integer(values, i64::from(high)),
            field = json_path("$.status_code"),
        ));
    }
    if let Some(outcome) = &query.filter_outcome {
        predicates.push(format!(
            "{} = {}",
            json_path("$.outcome"),
            bind_text(values, outcome)
        ));
    }
    if let Some(session_id) = &query.filter_session_id {
        predicates.push(format!("session_id = {}", bind_text(values, session_id)));
    }
    if let Some(repo) = &query.filter_repo_full_name {
        predicates.push(format!(
            "{} = {}",
            json_path("$.correlation.repo_full_name"),
            bind_text(values, repo)
        ));
    }
    if let Some(trigger_issue) = query.filter_trigger_issue {
        predicates.push(format!(
            "{} = {}",
            json_path("$.correlation.trigger_issue"),
            bind_integer(values, trigger_issue)
        ));
    }
    if let Some(request_id) = &query.filter_request_id {
        predicates.push(format!("request_id = {}", bind_text(values, request_id)));
    }
    predicates
}

/// A fixed `json_extract` over the stored terminal body. The PATH is always a
/// compile-time constant; nothing a caller sends reaches it.
fn json_path(path: &'static str) -> String {
    format!("json_extract(CAST(terminal_json AS TEXT), '{path}')")
}

fn bind_text(values: &mut Vec<SqlValue>, value: &str) -> String {
    values.push(SqlValue::Text(value.to_string()));
    format!("?{}", values.len())
}

fn bind_integer(values: &mut Vec<SqlValue>, value: i64) -> String {
    values.push(SqlValue::Integer(value));
    format!("?{}", values.len())
}

#[cfg(test)]
#[path = "read_tests.rs"]
mod tests;
