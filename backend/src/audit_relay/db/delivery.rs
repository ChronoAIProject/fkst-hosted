//! The delivery state machine's storage half: claiming, accepting, failing,
//! verifying, closing, and purging.
//!
//! ```text
//! complete | incomplete ──capture 2xx──> posthog_accepted ──query read──> posthog_verified
//!        ^        │                                │                              │
//!        └retry───┘                        (absent past max age)              (purge after
//!                 └──attempts exhausted──> dead_letter                     verified retention)
//! ```
//!
//! Three invariants this module enforces rather than assumes:
//!
//! - **Acceptance is not verification.** [`mark_accepted`] sets
//!   `posthog_accepted_at` and moves the row to `posthog_accepted`; only
//!   [`mark_verified`] — driven by a fixed query that read the event id back —
//!   sets `posthog_verified_at`. Nothing collapses the two.
//! - **FIFO fairness, with an escape hatch for a poison record.**
//!   [`claim_due`] orders by `terminal_at, event_id`, so the oldest record is
//!   always attempted first; per-record `capture_attempts` and `next_attempt_at`
//!   mean one permanently-rejected record backs off and eventually dead-letters
//!   on its own instead of parking everything behind it.
//! - **Unverified, incomplete, and dead-letter rows are never auto-deleted.**
//!   [`purge_verified`] touches `posthog_verified` rows only. The rows whose
//!   delivery could not be PROVEN are exactly the rows an audit trail may not
//!   discard on a timer; removing them is an explicit operator action.

use k8s_openapi::chrono::{DateTime, Duration, Utc};
use rusqlite::{params, params_from_iter, Connection, Transaction};

use super::super::protocol::format_instant;
use super::super::record::RecordState;
use super::row::{StoredRecord, RECORD_COLUMNS};
use super::DbError;

/// Records due for a capture attempt, oldest terminal first.
pub fn claim_due(
    connection: &Connection,
    now: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<StoredRecord>, DbError> {
    let now_text = format_instant(now);
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM audit_records
              WHERE state IN (?1, ?2)
                AND (next_attempt_at IS NULL OR next_attempt_at <= ?3)
              ORDER BY terminal_at ASC, event_id ASC
              LIMIT ?4"
        ))
        .map_err(|error| super::classify(&error))?;
    collect(statement.query_map(
        params![
            RecordState::Complete.as_str(),
            RecordState::Incomplete.as_str(),
            now_text,
            i64::try_from(limit).unwrap_or(i64::MAX),
        ],
        StoredRecord::from_row,
    ))
}

/// Records accepted long enough ago to be worth verifying.
pub fn claim_unverified(
    connection: &Connection,
    accepted_before: DateTime<Utc>,
    limit: usize,
) -> Result<Vec<StoredRecord>, DbError> {
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM audit_records
              WHERE state = ?1
                AND posthog_accepted_at IS NOT NULL
                AND posthog_accepted_at <= ?2
              ORDER BY posthog_accepted_at ASC, event_id ASC
              LIMIT ?3"
        ))
        .map_err(|error| super::classify(&error))?;
    collect(statement.query_map(
        params![
            RecordState::PosthogAccepted.as_str(),
            format_instant(accepted_before),
            i64::try_from(limit).unwrap_or(i64::MAX),
        ],
        StoredRecord::from_row,
    ))
}

/// One start whose completion never arrived, with the body needed to close it.
///
/// It carries `start_json` — which [`StoredRecord`] deliberately does not, since
/// that blob is dead weight on every read-API page — because synthesizing the
/// incomplete projection must derive every field from what was registered and
/// invent nothing.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OverdueStart {
    pub event_id: String,
    pub start_json: Vec<u8>,
}

/// Started records whose deadline plus grace has elapsed.
pub fn claim_overdue_starts(
    connection: &Connection,
    now: DateTime<Utc>,
    grace_secs: u64,
    limit: usize,
) -> Result<Vec<OverdueStart>, DbError> {
    let cutoff = now - Duration::seconds(i64::try_from(grace_secs).unwrap_or(i64::MAX));
    let mut statement = connection
        .prepare(
            "SELECT event_id, start_json FROM audit_records
              WHERE state = ?1
                AND completion_deadline_at IS NOT NULL
                AND completion_deadline_at <= ?2
              ORDER BY completion_deadline_at ASC, event_id ASC
              LIMIT ?3",
        )
        .map_err(|error| super::classify(&error))?;
    let rows = statement
        .query_map(
            params![
                RecordState::Started.as_str(),
                format_instant(cutoff),
                i64::try_from(limit).unwrap_or(i64::MAX),
            ],
            |row| {
                Ok(OverdueStart {
                    event_id: row.get(0)?,
                    start_json: row.get(1)?,
                })
            },
        )
        .map_err(|error| super::classify(&error))?;
    let mut overdue = Vec::new();
    for row in rows {
        overdue.push(row.map_err(|error| super::classify(&error))?);
    }
    Ok(overdue)
}

/// Atomically close one overdue start as `incomplete`.
///
/// The guard on `state = 'started'` is what makes it atomic against a completion
/// that lands in the same instant: whichever transaction commits first wins, and
/// the loser changes nothing. A real completion must never be overwritten by a
/// synthesized incomplete.
pub fn synthesize_incomplete(
    transaction: &Transaction<'_>,
    event_id: &str,
    terminal_json: &[u8],
    terminal_at: DateTime<Utc>,
    now: DateTime<Utc>,
) -> Result<bool, DbError> {
    let now_text = format_instant(now);
    let changed = transaction
        .execute(
            "UPDATE audit_records
                SET state = ?2,
                    terminal_json = ?3,
                    terminal_at = ?4,
                    next_attempt_at = ?5,
                    updated_at = ?5
              WHERE event_id = ?1 AND state = ?6",
            params![
                event_id,
                RecordState::Incomplete.as_str(),
                terminal_json,
                format_instant(terminal_at),
                now_text,
                RecordState::Started.as_str(),
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(changed > 0)
}

/// Record a capture acceptance. `posthog_accepted_at` is set; verification is a
/// separate, later state.
pub fn mark_accepted(
    transaction: &Transaction<'_>,
    event_ids: &[String],
    now: DateTime<Utc>,
) -> Result<usize, DbError> {
    if event_ids.is_empty() {
        return Ok(0);
    }
    let now_text = format_instant(now);
    let mut changed = 0usize;
    for event_id in event_ids {
        changed += transaction
            .execute(
                "UPDATE audit_records
                    SET state = ?2,
                        posthog_accepted_at = ?3,
                        next_attempt_at = NULL,
                        last_delivery_code = NULL,
                        updated_at = ?3
                  WHERE event_id = ?1 AND state IN (?4, ?5)",
                params![
                    event_id,
                    RecordState::PosthogAccepted.as_str(),
                    now_text,
                    RecordState::Complete.as_str(),
                    RecordState::Incomplete.as_str(),
                ],
            )
            .map_err(|error| super::classify(&error))?;
    }
    Ok(changed)
}

/// Record a failed attempt: bump the counter and schedule the next try.
pub fn mark_retry(
    transaction: &Transaction<'_>,
    event_id: &str,
    next_attempt_at: DateTime<Utc>,
    code: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    transaction
        .execute(
            "UPDATE audit_records
                SET capture_attempts = capture_attempts + 1,
                    next_attempt_at = ?2,
                    last_delivery_code = ?3,
                    updated_at = ?4
              WHERE event_id = ?1",
            params![
                event_id,
                format_instant(next_attempt_at),
                code,
                format_instant(now)
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(())
}

/// Give up on one record permanently. It is RETAINED, with the stable reason.
pub fn mark_dead_letter(
    transaction: &Transaction<'_>,
    event_id: &str,
    code: &str,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    transaction
        .execute(
            "UPDATE audit_records
                SET state = ?2,
                    capture_attempts = capture_attempts + 1,
                    next_attempt_at = NULL,
                    last_delivery_code = ?3,
                    updated_at = ?4
              WHERE event_id = ?1",
            params![
                event_id,
                RecordState::DeadLetter.as_str(),
                code,
                format_instant(now)
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(())
}

/// Mark records the verification query actually read back.
pub fn mark_verified(
    transaction: &Transaction<'_>,
    event_ids: &[String],
    now: DateTime<Utc>,
) -> Result<usize, DbError> {
    let now_text = format_instant(now);
    let mut changed = 0usize;
    for event_id in event_ids {
        changed += transaction
            .execute(
                "UPDATE audit_records
                    SET state = ?2,
                        posthog_verified_at = ?3,
                        updated_at = ?3
                  WHERE event_id = ?1 AND state = ?4",
                params![
                    event_id,
                    RecordState::PosthogVerified.as_str(),
                    now_text,
                    RecordState::PosthogAccepted.as_str(),
                ],
            )
            .map_err(|error| super::classify(&error))?;
    }
    Ok(changed)
}

/// Send an accepted-but-still-absent record back for another capture with the
/// SAME uuid. PostHog deduplicates on it, so a re-capture is safe; what is not
/// safe is leaving a row that says "accepted" when nothing can read it back.
pub fn requeue_for_recapture(
    transaction: &Transaction<'_>,
    event_id: &str,
    restore_state: RecordState,
    now: DateTime<Utc>,
) -> Result<(), DbError> {
    let now_text = format_instant(now);
    transaction
        .execute(
            "UPDATE audit_records
                SET state = ?2,
                    posthog_accepted_at = NULL,
                    next_attempt_at = ?3,
                    last_delivery_code = ?4,
                    updated_at = ?3
              WHERE event_id = ?1 AND state = ?5",
            params![
                event_id,
                restore_state.as_str(),
                now_text,
                "verification_absent",
                RecordState::PosthogAccepted.as_str(),
            ],
        )
        .map_err(|error| super::classify(&error))?;
    Ok(())
}

/// Delete verified rows past the dedup/query overlap window. This is the ONLY
/// delete in the relay.
pub fn purge_verified(
    transaction: &Transaction<'_>,
    verified_before: DateTime<Utc>,
) -> Result<usize, DbError> {
    transaction
        .execute(
            "DELETE FROM audit_records
              WHERE state = ?1 AND posthog_verified_at IS NOT NULL AND posthog_verified_at < ?2",
            params![
                RecordState::PosthogVerified.as_str(),
                format_instant(verified_before)
            ],
        )
        .map_err(|error| super::classify(&error))
}

/// Which of `event_ids` are still in `posthog_accepted`, for the absent/recapture
/// decision after a verification read.
pub fn accepted_before(
    connection: &Connection,
    event_ids: &[String],
    accepted_before: DateTime<Utc>,
) -> Result<Vec<StoredRecord>, DbError> {
    if event_ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = (0..event_ids.len())
        .map(|index| format!("?{}", index + 3))
        .collect::<Vec<_>>()
        .join(", ");
    let mut statement = connection
        .prepare(&format!(
            "SELECT {RECORD_COLUMNS} FROM audit_records
              WHERE state = ?1
                AND posthog_accepted_at IS NOT NULL
                AND posthog_accepted_at <= ?2
                AND event_id IN ({placeholders})"
        ))
        .map_err(|error| super::classify(&error))?;
    let mut values: Vec<rusqlite::types::Value> = vec![
        rusqlite::types::Value::Text(RecordState::PosthogAccepted.as_str().to_string()),
        rusqlite::types::Value::Text(format_instant(accepted_before)),
    ];
    values.extend(
        event_ids
            .iter()
            .map(|id| rusqlite::types::Value::Text(id.clone())),
    );
    collect(statement.query_map(params_from_iter(values.iter()), StoredRecord::from_row))
}

/// Collect a mapped row iterator, turning a decode failure into a bounded error.
fn collect(
    rows: rusqlite::Result<
        rusqlite::MappedRows<'_, impl FnMut(&rusqlite::Row<'_>) -> rusqlite::Result<StoredRecord>>,
    >,
) -> Result<Vec<StoredRecord>, DbError> {
    let rows = rows.map_err(|error| super::classify(&error))?;
    let mut records = Vec::new();
    for row in rows {
        match row {
            Ok(record) => records.push(record),
            Err(error) => tracing::warn!(
                reason = super::row::decode_error(&error).as_str(),
                "audit relay: skipping an undecodable stored record"
            ),
        }
    }
    Ok(records)
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
