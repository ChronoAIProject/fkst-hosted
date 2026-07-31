//! The stored row and its column mapping.
//!
//! ## Why `actor_id` and `session_id` are TEXT
//!
//! They are the two SCOPE columns: every personal read seeks on one of them
//! through `audit_records_actor_terminal` / `audit_records_session_terminal`.
//! Keeping them TEXT (the numeric actor id rendered in decimal) means the bound
//! parameter and the column have one storage class, so SQLite can never apply an
//! affinity conversion that quietly turns an indexed seek into a scan — which
//! would break the "predicate before `LIMIT`" guarantee in the one place it
//! matters most. The conversion is exact and total in both directions.
//!
//! ## Why the JSON is stored verbatim
//!
//! `start_json` and `terminal_json` hold the already-sanitized wire bodies
//! exactly as they were validated. Nothing is re-derived on read, so the read
//! surface cannot widen what the write surface accepted, and an idempotent replay
//! can be decided by comparing bytes rather than by re-deriving a canonical form
//! that might drift between builds.

use rusqlite::Row;

use super::super::record::{RecordState, RelayRecordKind};
use super::DbError;

/// The columns every read of a stored record selects, in one place so the
/// SELECT list and [`StoredRecord::from_row`] cannot drift apart.
pub const RECORD_COLUMNS: &str = "event_id, record_kind, state, actor_id, session_id, \
     started_at, terminal_at, completion_deadline_at, terminal_json, capture_attempts, \
     posthog_accepted_at, posthog_verified_at, last_delivery_code";

/// One stored record, as the delivery worker and the read API see it.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredRecord {
    pub event_id: String,
    pub record_kind: RelayRecordKind,
    pub state: RecordState,
    pub actor_id: Option<i64>,
    pub session_id: Option<String>,
    pub started_at: String,
    pub terminal_at: Option<String>,
    pub completion_deadline_at: Option<String>,
    /// The stored terminal wire body. `None` for a `started` row.
    pub terminal_json: Option<Vec<u8>>,
    pub capture_attempts: u32,
    pub posthog_accepted_at: Option<String>,
    pub posthog_verified_at: Option<String>,
    pub last_delivery_code: Option<String>,
}

impl StoredRecord {
    /// Decode a row selected with [`RECORD_COLUMNS`].
    pub fn from_row(row: &Row<'_>) -> rusqlite::Result<Self> {
        let record_kind: String = row.get(1)?;
        let state: String = row.get(2)?;
        let actor_id: Option<String> = row.get(3)?;
        Ok(Self {
            event_id: row.get(0)?,
            // A row whose closed-enum column is unreadable is a corrupted record,
            // not a differently-shaped one. Defaulting would silently reclassify
            // it; the sweep and the read both drop it loudly instead.
            record_kind: RelayRecordKind::parse(&record_kind)
                .ok_or_else(|| unknown_column("record_kind"))?,
            state: RecordState::parse(&state).ok_or_else(|| unknown_column("state"))?,
            actor_id: actor_id.and_then(|raw| raw.parse::<i64>().ok()),
            session_id: row.get(4)?,
            started_at: row.get(5)?,
            terminal_at: row.get(6)?,
            completion_deadline_at: row.get(7)?,
            terminal_json: row.get(8)?,
            capture_attempts: row.get::<_, i64>(9)?.clamp(0, i64::from(u32::MAX)) as u32,
            posthog_accepted_at: row.get(10)?,
            posthog_verified_at: row.get(11)?,
            last_delivery_code: row.get(12)?,
        })
    }

    /// The instant this row sorts on: its terminal instant. `None` for a row
    /// with no terminal projection, which the read API never returns.
    pub fn sort_timestamp(&self) -> Option<&str> {
        self.terminal_at.as_deref()
    }
}

/// The numeric actor id, as the scope column stores it.
pub fn actor_column(actor_id: Option<i64>) -> Option<String> {
    actor_id.map(|id| id.to_string())
}

fn unknown_column(column: &'static str) -> rusqlite::Error {
    rusqlite::Error::FromSqlConversionFailure(
        0,
        rusqlite::types::Type::Text,
        Box::new(UnknownEnum { column }),
    )
}

/// A stored closed-enum column carrying a value this build does not know.
#[derive(Debug, thiserror::Error)]
#[error("stored column `{column}` is not a value of its closed enum")]
struct UnknownEnum {
    column: &'static str,
}

/// Map a rusqlite failure raised while decoding rows.
pub fn decode_error(error: &rusqlite::Error) -> DbError {
    match error {
        rusqlite::Error::SqliteFailure(..) => super::classify(error),
        _ => DbError::Internal("decode"),
    }
}

#[cfg(test)]
#[path = "row_tests.rs"]
mod tests;
