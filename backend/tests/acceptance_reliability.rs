//! Milestone acceptance: required delivery under process, storage, and replica
//! failure.
//!
//! The unit suites cover each failure mode in isolation against a mocked relay.
//! What they cannot show is the composite behaviour the epic's `OPS-01`/`OPS-03`
//! and `AUD-07` actually promise, over a REAL relay process and a real SQLite
//! file:
//!
//! - a duplicate `event_id` arriving concurrently from two replicas leaves one
//!   record, not two and not zero;
//! - a database that cannot be written to refuses ingress rather than accepting
//!   a record it will lose;
//! - a relay that dies after the handler leaves a durable start that closes as
//!   `incomplete`, never a fabricated status;
//! - a restarted replica needs no in-process state to serve history, because
//!   there is none — that is what "stateless control plane" has to mean.

#[path = "audit_relay_harness/mod.rs"]
mod relay;

use std::sync::Arc;

use fkst_control_plane::audit::relay::{
    AuditDeliveryConfig, AuditDeliveryMode, AuditRelayClient, RelayClientMetrics,
};
use fkst_control_plane::audit_relay::protocol::format_instant;
use k8s_openapi::chrono::Duration as ChronoDuration;
use secrecy::SecretString;

/// Two replicas submit the same start concurrently, plus one unique start each.
///
/// At-least-once delivery with deterministic ids means the duplicate must be
/// acknowledged (so neither replica fails a user's request) while storing one
/// row — the epic's `AUD-07` contract, exercised across a socket rather than a
/// mocked client.
#[tokio::test]
async fn two_replicas_racing_one_duplicate_event_id_leave_exactly_one_record() {
    let node = relay::Relay::start().await;
    let shared = "51515151-1111-4111-8111-515151515151";
    let unique = [
        "61616161-1111-4111-8111-616161616161",
        "71717171-1111-4111-8111-717171717171",
    ];

    let mut submissions = Vec::new();
    for (index, replica) in [node.client(), node.client()].into_iter().enumerate() {
        let own = unique[index];
        submissions.push(tokio::spawn(async move {
            // Every replica submits the SHARED id and its own.
            for event_id in [shared, own] {
                replica
                    .register_start(&relay::Relay::start_body(event_id))
                    .await
                    .expect("a duplicate start is acknowledged, never refused");
                replica
                    .complete(&relay::Relay::completion_body(event_id, Some(relay::ALICE)))
                    .await
                    .expect("a duplicate completion is acknowledged");
            }
        }));
    }
    for submission in submissions {
        submission.await.expect("the replica task completes");
    }

    let rows = node.read_all().await;
    let ids: Vec<&str> = rows.iter().map(|row| row.event_id.as_str()).collect();
    assert_eq!(
        ids.iter().filter(|id| **id == shared).count(),
        1,
        "the shared event id was stored more than once: {ids:?}"
    );
    for own in unique {
        assert!(
            ids.contains(&own),
            "a replica's own record was lost in the race: {ids:?}"
        );
    }
    assert_eq!(
        rows.len(),
        3,
        "expected exactly three durable records: {ids:?}"
    );
}

/// A damaged or absent database must be an explicit startup failure.
///
/// This is one half of `OPS-01`'s storage side. The control plane's contract is
/// "no durable start, no handler"; it only holds if a relay over unusable
/// storage refuses to come up at all, rather than starting and acknowledging
/// writes it will lose. Both failures are asserted at `Database::open`, which is
/// where the schema is applied and therefore the first moment either is
/// detectable.
///
/// A read-only DIRECTORY is deliberately not one of them: the relay's own
/// `prepare_directory` chmods its data directory to `0700` before opening, and a
/// process that owns the directory can always do that — so the scenario cannot
/// be staged without changing ownership, and staging it with mode bits alone
/// would assert nothing. What CAN be staged, and is what an operator actually
/// hits, is a damaged file and a path that is not a file at all (the classic
/// "the volume did not mount, so the mount point is an empty directory").
#[tokio::test]
async fn a_damaged_or_unmounted_database_refuses_to_open_instead_of_losing_records() {
    let dir = tempfile::TempDir::new().expect("temp dir");

    let corrupt = dir.path().join("corrupt.sqlite3");
    std::fs::write(&corrupt, b"SQLite format 3\0this is not a database at all")
        .expect("the corrupt file is written");
    assert!(
        open_relay_database(&corrupt).is_err(),
        "a relay opened a corrupt database instead of refusing"
    );

    let unmounted = dir.path().join("unmounted.sqlite3");
    std::fs::create_dir(&unmounted).expect("the stand-in mount point is created");
    assert!(
        open_relay_database(&unmounted).is_err(),
        "a relay accepted a data path that is not a database file"
    );

    // The positive control: the same settings over healthy storage DO open, so
    // the two refusals above are about the storage rather than the settings.
    assert!(open_relay_database(&dir.path().join("healthy.sqlite3")).is_ok());
}

/// A read-only database never ACKNOWLEDGES a record it cannot store.
///
/// This is the scenario the previous name claimed and its body never staged, and
/// staging it turns out to be more interesting than "open must fail". What this
/// deployment does with an owner-owned read-only file is deliberate and layered:
/// `Database::open` calls `restrict_file_permissions`, which chmods the MAIN
/// file back to `0600` — so a restored backup copied with restrictive modes
/// heals rather than wedging the relay — while SQLite's `-wal` and `-shm`
/// sidecars inherit the mode the main file had when they were created, and are
/// not repaired.
///
/// Asserting "open fails" would therefore be asserting a behaviour the code does
/// not have; asserting "ingress works" would be asserting one it does not have
/// either. The property that actually matters for `OPS-01` holds in both worlds
/// and is what is asserted here: whichever way the platform lands, a record is
/// never acknowledged and then lost. Either the write succeeds and the row is
/// readable, or it fails loudly and no row exists.
///
/// The variants that cannot be repaired at all — a `readOnly: true` volume
/// mount, or a file owned by another uid — are not stageable inside a test
/// process without root, and are deliberately left to the deployment gate: the
/// relay's Pod spec and its bound claim are checked by
/// `deploy/kubernetes/tests/audit-relay-verify-test.sh`.
#[tokio::test]
async fn a_read_only_database_never_acknowledges_a_record_it_cannot_store() {
    use std::os::unix::fs::PermissionsExt;

    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("read-only.sqlite3");
    drop(open_relay_database(&path).expect("the healthy database opens"));

    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o444))
        .expect("the database file is made read-only");
    assert!(
        std::fs::OpenOptions::new().write(true).open(&path).is_err(),
        "this process can write a 0444 file, so the scenario was never staged"
    );

    let database = open_relay_database(&path).expect("the relay repairs and opens the main file");
    let mode = std::fs::metadata(&path)
        .expect("the database file is readable")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(
        mode, 0o600,
        "the relay opened the file without restoring its own permissions"
    );
    drop(database);

    let node = relay::Relay::start_at(dir, path).await;
    let event_id = "d0d0d0d0-1111-4111-8111-d0d0d0d0d0d0";
    let acknowledged = node
        .client()
        .register_start(&relay::Relay::start_body(event_id))
        .await
        .is_ok();
    if acknowledged {
        node.client()
            .complete(&relay::Relay::completion_body(event_id, Some(relay::ALICE)))
            .await
            .expect("a relay that acknowledged the start must accept its completion");
        assert!(
            node.read_all()
                .await
                .iter()
                .any(|row| row.event_id == event_id),
            "the relay acknowledged a record it did not store"
        );
    } else {
        assert!(
            node.read_all().await.is_empty(),
            "the relay refused the write and stored something anyway"
        );
    }
}

/// A database from a NEWER build must refuse to open, not be downgraded.
///
/// This is the migration failure an operator actually hits: a newer relay
/// migrated the volume, the deployment was rolled back, and the older binary now
/// meets a schema it does not understand. Refusing is the only safe answer — a
/// relay that started anyway would write rows the newer schema's constraints
/// never sanctioned into the one durable copy of the history.
#[tokio::test]
async fn a_database_from_a_newer_build_refuses_to_open() {
    let dir = tempfile::TempDir::new().expect("temp dir");
    let path = dir.path().join("from-the-future.sqlite3");
    drop(open_relay_database(&path).expect("the healthy database opens"));

    {
        let future = rusqlite::Connection::open(&path).expect("the database re-opens");
        future
            .execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![9_999_i64, "2099-01-01T00:00:00Z"],
            )
            .expect("the future migration marker is recorded");
    }

    assert!(
        open_relay_database(&path).is_err(),
        "a relay opened a database migrated by a newer build and would have \
         written into a schema it does not understand"
    );
}

/// Open a relay database with the same settings the harness uses.
fn open_relay_database(
    path: &std::path::Path,
) -> Result<fkst_control_plane::audit_relay::Database, fkst_control_plane::audit_relay::DbError> {
    fkst_control_plane::audit_relay::Database::open(
        path,
        fkst_control_plane::audit_relay::DatabaseSettings {
            busy_timeout_ms: 1_000,
            writer_queue_capacity: 8,
            read_concurrency: 1,
            max_records: 1_000,
        },
    )
}

/// A relay that stops answering AFTER the handler ran leaves a durable start
/// that the closer turns into `incomplete` — never a status the relay invented.
#[tokio::test]
async fn a_relay_that_dies_after_the_handler_leaves_an_incomplete_not_a_status() {
    let node = relay::Relay::start().await;
    let event_id = "a5a5a5a5-1111-4111-8111-a5a5a5a5a5a5";

    let mut start = relay::Relay::start_body(event_id);
    start.started_at = format_instant(relay::anchor() - ChronoDuration::seconds(300));
    start.completion_deadline_at = format_instant(relay::anchor() - ChronoDuration::seconds(240));
    node.client()
        .register_start(&start)
        .await
        .expect("the start is durable before the handler runs");

    // The relay dies here. The completion, which the control plane WOULD have
    // submitted, never lands.
    let node = node.restart().await;
    node.sweep(k8s_openapi::chrono::Utc::now()).await;

    let rows = node.read_all_recent().await;
    let row = rows
        .iter()
        .find(|row| row.event_id == event_id)
        .unwrap_or_else(|| panic!("the durable start did not survive the restart: {rows:?}"));
    assert_eq!(row.terminal["outcome"], "incomplete");
    assert_eq!(row.terminal["status_code"], serde_json::Value::Null);
}

/// A replica restarted with a cold process serves the same history, because the
/// history was never in the process.
#[tokio::test]
async fn a_restarted_replica_needs_no_in_process_state_to_serve_history() {
    let node = relay::Relay::start().await;
    node.seed_cross_user_fixture().await;
    let before: Vec<String> = node
        .read_personal(relay::ALICE, None, "api_request", 50, None)
        .await
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    assert!(!before.is_empty(), "the fixture must seed Alice some rows");

    // A brand-new client with brand-new connection state — the shape a rolled
    // replica presents. Nothing is carried over from the first client.
    let fresh = fresh_client(&node);
    let after: Vec<String> = fresh
        .read_records(
            &fkst_control_plane::audit_relay::query::RecordsQueryV1 {
                scope: "mine".to_string(),
                actor_id: Some(relay::ALICE),
                record_kind: "api_request".to_string(),
                from: format_instant(relay::anchor() - ChronoDuration::hours(24)),
                to: format_instant(relay::anchor() + ChronoDuration::hours(24)),
                limit: 50,
                ..Default::default()
            },
            std::time::Duration::from_secs(5),
        )
        .await
        .expect("the relay answers a cold replica")
        .rows
        .into_iter()
        .map(|row| row.event_id)
        .collect();
    assert_eq!(
        before, after,
        "a cold replica saw a different history from a warm one"
    );
}

/// A client built from scratch against the same relay URL.
fn fresh_client(node: &relay::Relay) -> Arc<AuditRelayClient> {
    let config = AuditDeliveryConfig {
        mode: AuditDeliveryMode::Required,
        relay_url: Some(node.base_url().to_string()),
        write_token: SecretString::from(relay::WRITE_TOKEN.to_string()),
        read_token: SecretString::from(relay::READ_TOKEN.to_string()),
        start_timeout_ms: 2_000,
        completion_timeout_ms: 2_000,
        incomplete_grace_secs: 60,
    };
    Arc::new(
        AuditRelayClient::from_config(&config, RelayClientMetrics::new())
            .expect("the relay client builds"),
    )
}
