//! Durability-contract tests: the pragmas are really applied, migrations are
//! transactional and idempotent, and a newer database is refused.

use rusqlite::Connection;
use tempfile::TempDir;

use super::*;

fn open() -> (TempDir, Connection) {
    let dir = TempDir::new().expect("temp dir");
    let connection = Connection::open(dir.path().join("audit.sqlite3")).expect("opens");
    (dir, connection)
}

#[test]
fn wal_and_full_synchronous_are_really_applied() {
    let (_dir, connection) = open();
    let journal_mode = apply_pragmas(&connection, 5_000).expect("pragmas apply");
    assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
    // Read the values BACK rather than trusting the statements: SQLite silently
    // keeps `delete` mode on some filesystems, and `synchronous` is the whole
    // reason a `2xx` may be called durable.
    assert_eq!(
        pragma_text(&connection, "journal_mode")
            .expect("journal_mode")
            .to_ascii_lowercase(),
        "wal"
    );
    // `2` is SQLite's numeric spelling of FULL.
    assert_eq!(
        pragma_text(&connection, "synchronous").expect("synchronous"),
        "2"
    );
    assert_eq!(
        pragma_text(&connection, "foreign_keys").expect("foreign_keys"),
        "1"
    );
}

#[test]
fn migration_creates_the_table_and_every_declared_index() {
    let (_dir, mut connection) = open();
    apply_pragmas(&connection, 5_000).expect("pragmas apply");
    assert_eq!(migrate(&mut connection).expect("migrates"), SCHEMA_VERSION);

    let mut statement = connection
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'audit_records'",
        )
        .expect("prepares");
    let indexes: Vec<String> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .expect("queries")
        .filter_map(Result::ok)
        .collect();
    for expected in [
        "audit_records_state_next_attempt",
        "audit_records_terminal",
        "audit_records_request_id",
        "audit_records_operation_id",
        "audit_records_actor_terminal",
        "audit_records_session_terminal",
    ] {
        assert!(
            indexes.iter().any(|name| name == expected),
            "index `{expected}` must exist; found {indexes:?}"
        );
    }
}

#[test]
fn migration_is_idempotent() {
    let (_dir, mut connection) = open();
    apply_pragmas(&connection, 5_000).expect("pragmas apply");
    migrate(&mut connection).expect("first migration");
    migrate(&mut connection).expect("second migration is a no-op");
    let applied: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("counts");
    assert_eq!(applied, SCHEMA_VERSION);
}

#[test]
fn a_database_from_a_newer_build_is_refused_rather_than_used() {
    let (_dir, mut connection) = open();
    apply_pragmas(&connection, 5_000).expect("pragmas apply");
    migrate(&mut connection).expect("migrates");
    connection
        .execute(
            "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![SCHEMA_VERSION + 1],
        )
        .expect("stages a future version");
    let error = migrate(&mut connection).expect_err("a newer schema is refused");
    assert!(format!("{error}").contains("newer"));
}

#[test]
fn a_failing_migration_leaves_no_partial_schema() {
    // Simulate a mid-migration failure by pre-creating one of the objects the
    // migration creates: the transaction must roll the whole step back, so
    // neither the table NOR the version row survives.
    let (_dir, mut connection) = open();
    apply_pragmas(&connection, 5_000).expect("pragmas apply");
    connection
        .execute("CREATE TABLE audit_records (event_id TEXT)", [])
        .expect("pre-creates a conflicting table");
    assert!(migrate(&mut connection).is_err());
    let applied: i64 = connection
        .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
            row.get(0)
        })
        .expect("counts");
    assert_eq!(applied, 0, "a failed migration must record no version");
}
