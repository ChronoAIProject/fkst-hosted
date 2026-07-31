//! The relay's SQLite schema, its migrations, and the pragmas that make a `2xx`
//! mean "durably committed".
//!
//! ## Pragmas are the durability contract
//!
//! - `journal_mode=WAL` — a reader never blocks the single writer, which is what
//!   lets the scoped read run while ingress continues.
//! - `synchronous=FULL` — every commit is fsynced. This is the expensive one, and
//!   it is the point: the control plane's `required` mode answers a client only
//!   after the relay says "committed", so anything weaker would turn that promise
//!   into a guess about the page cache.
//! - `foreign_keys=ON` — no foreign keys exist today; the pragma is set anyway so
//!   a future migration cannot silently inherit SQLite's off-by-default.
//! - `busy_timeout` — bounded, so a contended write fails with a metric instead
//!   of hanging a request forever.
//!
//! ## Migrations are transactional and fail-closed
//!
//! Each migration runs inside one transaction with its `schema_migrations` row;
//! a partial upgrade therefore cannot exist. A failure aborts startup rather than
//! serving from a half-migrated database — a relay that accepts records it cannot
//! later read is worse than one that refuses to start.
//!
//! ## Why the indexes are exactly these
//!
//! Every scoped read applies its predicate BEFORE `LIMIT` (epic `AUTH-06`), and
//! "before `LIMIT`" is only meaningful if the engine can satisfy the predicate
//! and the ordering from an index. `audit_records_actor_terminal` and
//! `audit_records_session_terminal` are `(scope key, terminal_at DESC,
//! event_id DESC)` so a personal query seeks straight to the caller's own rows in
//! sort order; `audit_records_state_next_attempt` drives the delivery sweep;
//! the remaining three serve exact-value correlation lookups. A query-plan test
//! asserts the first two are actually chosen.

use rusqlite::{Connection, Transaction};

/// The schema version this build expects. A database at a HIGHER version is
/// refused: a newer relay may have written columns this build would silently
/// ignore, and silently ignoring audit columns is data loss.
pub const SCHEMA_VERSION: i64 = 1;

/// One migration step.
struct Migration {
    version: i64,
    statements: &'static [&'static str],
}

const MIGRATIONS: &[Migration] = &[Migration {
    version: 1,
    statements: &[
        "CREATE TABLE audit_records (
            event_id               TEXT PRIMARY KEY,
            schema_version         INTEGER NOT NULL,
            kind                   TEXT NOT NULL,
            request_id             TEXT,
            operation_id           TEXT,
            actor_id               TEXT,
            session_id             TEXT,
            record_kind            TEXT NOT NULL,
            started_at             TEXT NOT NULL,
            terminal_at            TEXT,
            completion_deadline_at TEXT,
            state                  TEXT NOT NULL,
            start_json             BLOB NOT NULL,
            terminal_json          BLOB,
            capture_attempts       INTEGER NOT NULL DEFAULT 0,
            next_attempt_at        TEXT,
            posthog_accepted_at    TEXT,
            posthog_verified_at    TEXT,
            last_delivery_code     TEXT,
            created_at             TEXT NOT NULL,
            updated_at             TEXT NOT NULL
        )",
        "CREATE INDEX audit_records_state_next_attempt
            ON audit_records (state, next_attempt_at)",
        "CREATE INDEX audit_records_terminal
            ON audit_records (terminal_at DESC, event_id DESC)",
        "CREATE INDEX audit_records_request_id
            ON audit_records (request_id)",
        "CREATE INDEX audit_records_operation_id
            ON audit_records (operation_id)",
        "CREATE INDEX audit_records_actor_terminal
            ON audit_records (actor_id, terminal_at DESC, event_id DESC)",
        "CREATE INDEX audit_records_session_terminal
            ON audit_records (session_id, terminal_at DESC, event_id DESC)",
    ],
}];

/// Apply every configured pragma. Returns the effective journal mode so the
/// caller can prove WAL is really on rather than assume the statement worked.
pub fn apply_pragmas(connection: &Connection, busy_timeout_ms: u64) -> rusqlite::Result<String> {
    connection.busy_timeout(std::time::Duration::from_millis(busy_timeout_ms))?;
    // `journal_mode` ANSWERS with the resulting mode; a plain `execute` would
    // discard the row and hide a refusal (SQLite silently keeps `delete` mode on
    // some filesystems).
    let journal_mode: String =
        connection.query_row("PRAGMA journal_mode = WAL", [], |row| row.get(0))?;
    connection.pragma_update(None, "synchronous", "FULL")?;
    connection.pragma_update(None, "foreign_keys", "ON")?;
    Ok(journal_mode)
}

/// Read back a pragma as text, for the startup assertions and the tests that
/// prove the durability contract rather than trusting it.
pub fn pragma_text(connection: &Connection, name: &str) -> rusqlite::Result<String> {
    connection.query_row(&format!("PRAGMA {name}"), [], |row| {
        let value: rusqlite::types::Value = row.get(0)?;
        Ok(match value {
            rusqlite::types::Value::Integer(number) => number.to_string(),
            rusqlite::types::Value::Text(text) => text,
            other => format!("{other:?}"),
        })
    })
}

/// Bring the database to [`SCHEMA_VERSION`], transactionally.
pub fn migrate(connection: &mut Connection) -> rusqlite::Result<i64> {
    connection.execute(
        "CREATE TABLE IF NOT EXISTS schema_migrations (
            version    INTEGER PRIMARY KEY,
            applied_at TEXT NOT NULL
        )",
        [],
    )?;
    let current: i64 = connection
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if current > SCHEMA_VERSION {
        return Err(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_ERROR),
            Some(format!(
                "audit relay database is at schema {current}, newer than this build's \
                 {SCHEMA_VERSION}"
            )),
        ));
    }
    for migration in MIGRATIONS.iter().filter(|m| m.version > current) {
        let transaction = connection.transaction()?;
        apply(&transaction, migration)?;
        transaction.commit()?;
        tracing::info!(
            version = migration.version,
            "audit relay: applied schema migration"
        );
    }
    Ok(SCHEMA_VERSION)
}

/// Run one migration's statements plus its version row inside `transaction`.
fn apply(transaction: &Transaction<'_>, migration: &Migration) -> rusqlite::Result<()> {
    for statement in migration.statements {
        transaction.execute(statement, [])?;
    }
    transaction.execute(
        "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
        rusqlite::params![
            migration.version,
            k8s_openapi::chrono::Utc::now().to_rfc3339()
        ],
    )?;
    Ok(())
}

#[cfg(test)]
#[path = "schema_tests.rs"]
mod tests;
