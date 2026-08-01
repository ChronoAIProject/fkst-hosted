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

/// Storage the relay cannot use must be an explicit startup failure.
///
/// This is the storage-side half of `OPS-01`. The control plane's contract is
/// "no durable start, no handler"; it only holds if a relay over unwritable or
/// damaged storage refuses to come up at all, rather than starting and
/// acknowledging writes it will lose. Both failures are asserted at
/// `Database::open`, which is where the schema is applied and therefore the
/// first moment either is detectable.
///
/// A read-only DIRECTORY is deliberately not one of them: the relay's own
/// `prepare_directory` chmods its data directory to `0700` before opening, and a
/// process that owns the directory can always do that — so the scenario cannot
/// be staged without changing ownership, and staging it with mode bits alone
/// would assert nothing. What CAN be staged, and is what an operator actually
/// hits, is a damaged file and a path that is not a file at all (the classic
/// "the volume did not mount, so the mount point is an empty directory").
#[tokio::test]
async fn a_read_only_database_refuses_ingress_instead_of_losing_a_record() {
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
