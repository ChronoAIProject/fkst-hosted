//! Idempotent, conflict-safe ingress: the three write paths, each committed in
//! one transaction before its endpoint answers.
//!
//! ## One rule governs all three
//!
//! **An event id names one immutable fact.** A replay carrying byte-identical
//! content is an acknowledgement ([`Ingested::Replayed`]); a replay carrying
//! anything else is [`super::DbError::Conflict`] and changes nothing. Audit
//! history is append-only: "last writer wins" would let a retry, a rolled-back
//! deploy, or a second replica quietly rewrite what a record says happened.
//!
//! ## Why the comparison is on bytes
//!
//! Both sides serialize the same `serde` structs with `serde_json`, whose default
//! map is key-ordered, so one logical record has exactly one encoding. Comparing
//! the stored bytes therefore decides "same fact" without re-deriving a canonical
//! form that could drift between builds — and it cannot be fooled by a field this
//! build does not know about.

use k8s_openapi::chrono::{DateTime, Utc};
use rusqlite::{params, OptionalExtension, Transaction};

use super::super::protocol::{
    format_instant, LifecycleEventV1, RequestCompletionV1, RequestStartV1, StartIdentity,
};
use super::super::record::{RecordState, RelayRecordKind};
use super::row::actor_column;
use super::DbError;

/// What one ingress call did.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Ingested {
    /// A new durable record was created (`201`).
    Created(RecordState),
    /// An exact replay of an already-durable record (`200`).
    Replayed(RecordState),
}

impl Ingested {
    /// The record's state after the call.
    pub fn state(self) -> RecordState {
        match self {
            Ingested::Created(state) | Ingested::Replayed(state) => state,
        }
    }

    /// Whether this call created the record.
    pub fn created(self) -> bool {
        matches!(self, Ingested::Created(_))
    }
}

/// Register a request start.
pub fn register_start(
    transaction: &Transaction<'_>,
    start: &RequestStartV1,
    identity: &StartIdentity,
    now: DateTime<Utc>,
) -> Result<Ingested, DbError> {
    let event_id = identity.event_id.to_string();
    let encoded = encode(start)?;
    let existing: Option<(String, Vec<u8>)> = transaction
        .query_row(
            "SELECT state, start_json FROM audit_records WHERE event_id = ?1",
            params![event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| super::classify(&error))?;

    if let Some((state, stored)) = existing {
        let state = RecordState::parse(&state).ok_or(DbError::Internal("decode"))?;
        if stored != encoded {
            return Err(DbError::Conflict);
        }
        return Ok(Ingested::Replayed(state));
    }

    let now_text = format_instant(now);
    transaction
        .execute(
            "INSERT INTO audit_records (
                event_id, schema_version, kind, request_id, operation_id, actor_id,
                session_id, record_kind, started_at, terminal_at, completion_deadline_at,
                state, start_json, terminal_json, capture_attempts, next_attempt_at,
                posthog_accepted_at, posthog_verified_at, last_delivery_code,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, ?5, NULL,
                NULL, ?6, ?7, NULL, ?8,
                ?9, ?10, NULL, 0, NULL,
                NULL, NULL, NULL,
                ?11, ?11
            )",
            params![
                event_id,
                i64::from(start.schema_version),
                RelayRecordKind::ApiRequest.ingress(),
                identity.request_id,
                identity.operation_id,
                RelayRecordKind::ApiRequest.as_str(),
                format_instant(identity.started_at),
                // The PARSED deadline, re-rendered: the overdue sweep compares
                // this column as text, so an alternative rendering of the same
                // instant would sort wrong and close the record early or late.
                format_instant(identity.completion_deadline_at),
                RecordState::Started.as_str(),
                encoded,
                now_text,
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(Ingested::Created(RecordState::Started))
}

/// Commit the terminal projection of an already-registered request.
///
/// `terminal_at` is the caller's ALREADY-PARSED completion instant. It is passed
/// in rather than read off the wire body because this column is the read API's
/// range and sort key and is compared as text: storing the submitted rendering
/// would let `…T00:00:00+00:00` and `…T00:00:00.000Z` — the same instant — order
/// differently, silently dropping rows from a page.
pub fn commit_completion(
    transaction: &Transaction<'_>,
    completion: &RequestCompletionV1,
    terminal_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Ingested, DbError> {
    let event_id = completion.event_id.clone();
    let encoded = encode(completion)?;
    let existing: Option<(String, Vec<u8>, Option<Vec<u8>>)> = transaction
        .query_row(
            "SELECT state, start_json, terminal_json FROM audit_records WHERE event_id = ?1",
            params![event_id],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(|error| super::classify(&error))?;

    // A completion with no registered start is refused, never auto-registered:
    // the whole value of `required` mode is that a start exists BEFORE a handler
    // runs, and inventing one here would erase the evidence that it did not.
    let Some((state, start_json, terminal_json)) = existing else {
        return Err(DbError::NoStart);
    };
    let state = RecordState::parse(&state).ok_or(DbError::Internal("decode"))?;
    let start: RequestStartV1 =
        serde_json::from_slice(&start_json).map_err(|_| DbError::Internal("decode"))?;
    if !start_matches(&start, completion) {
        return Err(DbError::Conflict);
    }
    if let Some(stored) = terminal_json {
        // Already terminal: an identical body is an acknowledgement, anything
        // else is a conflict. The stored history is never rewritten.
        return if stored == encoded {
            Ok(Ingested::Replayed(state))
        } else {
            Err(DbError::Conflict)
        };
    }

    let now_text = format_instant(now);
    transaction
        .execute(
            "UPDATE audit_records
                SET state = ?2,
                    terminal_json = ?3,
                    terminal_at = ?4,
                    actor_id = ?5,
                    session_id = ?6,
                    next_attempt_at = ?7,
                    updated_at = ?7
              WHERE event_id = ?1",
            params![
                event_id,
                RecordState::Complete.as_str(),
                encoded,
                format_instant(terminal_at),
                actor_column(completion.actor_id),
                completion.session_id,
                now_text,
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(Ingested::Created(RecordState::Complete))
}

/// Commit one lifecycle transition. It is terminal on arrival: there is no
/// "before" for a background effect, so start and terminal are the same body.
///
/// `occurred_at` is the already-parsed instant, normalized on write for the same
/// reason a completion's `terminal_at` is.
pub fn commit_lifecycle(
    transaction: &Transaction<'_>,
    event: &LifecycleEventV1,
    occurred_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<Ingested, DbError> {
    let event_id = event.event_id.clone();
    let encoded = encode(event)?;
    let existing: Option<(String, Option<Vec<u8>>)> = transaction
        .query_row(
            "SELECT state, terminal_json FROM audit_records WHERE event_id = ?1",
            params![event_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
        .map_err(|error| super::classify(&error))?;

    if let Some((state, terminal_json)) = existing {
        let state = RecordState::parse(&state).ok_or(DbError::Internal("decode"))?;
        return if terminal_json.as_deref() == Some(encoded.as_slice()) {
            Ok(Ingested::Replayed(state))
        } else {
            Err(DbError::Conflict)
        };
    }

    let now_text = format_instant(now);
    transaction
        .execute(
            "INSERT INTO audit_records (
                event_id, schema_version, kind, request_id, operation_id, actor_id,
                session_id, record_kind, started_at, terminal_at, completion_deadline_at,
                state, start_json, terminal_json, capture_attempts, next_attempt_at,
                posthog_accepted_at, posthog_verified_at, last_delivery_code,
                created_at, updated_at
            ) VALUES (
                ?1, ?2, ?3, ?4, NULL, ?5,
                ?6, ?7, ?8, ?8, NULL,
                ?9, ?10, ?10, 0, ?11,
                NULL, NULL, NULL,
                ?11, ?11
            )",
            params![
                event_id,
                i64::from(event.schema_version),
                RelayRecordKind::SandboxLifecycle.ingress(),
                event.correlation.request_id,
                // A system lifecycle row is visible through its SESSION, never
                // through an actor: storing the reconciler's actor id would make
                // it look like a person's own call.
                Option::<String>::None,
                event.session_id,
                RelayRecordKind::SandboxLifecycle.as_str(),
                format_instant(occurred_at),
                RecordState::Complete.as_str(),
                encoded,
                now_text,
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(Ingested::Created(RecordState::Complete))
}

/// How many records the outbox currently holds. Used by the capacity guard's
/// periodic sweep, never per request.
pub fn record_count(connection: &rusqlite::Connection) -> Result<u64, DbError> {
    connection
        .query_row("SELECT COUNT(*) FROM audit_records", [], |row| {
            row.get::<_, i64>(0)
        })
        .map(|count| u64::try_from(count).unwrap_or(0))
        .map_err(|error| super::classify(&error))
}

/// How many records sit in each state, for the bounded `{state}` gauges.
pub fn state_counts(connection: &rusqlite::Connection) -> Result<Vec<(String, u64)>, DbError> {
    let mut statement = connection
        .prepare("SELECT state, COUNT(*) FROM audit_records GROUP BY state")
        .map_err(|error| super::classify(&error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?))
        })
        .map_err(|error| super::classify(&error))?;
    let mut counts = Vec::new();
    for row in rows {
        let (state, count) = row.map_err(|error| super::classify(&error))?;
        counts.push((state, u64::try_from(count).unwrap_or(0)));
    }
    Ok(counts)
}

/// The oldest `created_at` in each state, for the staleness gauges.
pub fn oldest_per_state(
    connection: &rusqlite::Connection,
) -> Result<Vec<(String, String)>, DbError> {
    let mut statement = connection
        .prepare("SELECT state, MIN(created_at) FROM audit_records GROUP BY state")
        .map_err(|error| super::classify(&error))?;
    let rows = statement
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })
        .map_err(|error| super::classify(&error))?;
    let mut oldest = Vec::new();
    for row in rows {
        let (state, created_at) = row.map_err(|error| super::classify(&error))?;
        if let Some(created_at) = created_at {
            oldest.push((state, created_at));
        }
    }
    Ok(oldest)
}

/// Whether a completion agrees with its registered start on every immutable
/// field. A disagreement is a conflict, never a merge.
fn start_matches(start: &RequestStartV1, completion: &RequestCompletionV1) -> bool {
    start.event_id == completion.event_id
        && start.request_id == completion.request_id
        && start.started_at == completion.started_at
        && start.method == completion.method
        && start.route_template == completion.route_template
        && start.operation_id == completion.operation_id
}

/// Serialize a wire body for storage. A body that cannot be re-encoded is a
/// programming error on this side, not a caller's fault, so it is bounded and
/// named rather than propagated as a client error.
fn encode<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, DbError> {
    serde_json::to_vec(value).map_err(|_| DbError::Internal("encode"))
}

#[cfg(test)]
#[path = "ingest_tests.rs"]
mod tests;
