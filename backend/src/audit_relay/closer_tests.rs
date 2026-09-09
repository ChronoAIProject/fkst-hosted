//! Closer tests: incomplete synthesis past deadline + grace, the race with a
//! real completion, and the retention rule.

use k8s_openapi::chrono::Duration;

use crate::audit_relay::db::ingest;
use crate::audit_relay::incomplete::INCOMPLETE_ERROR_CODE;
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    commit, completion, durable_request, now, open_database, register, state_of,
};
use crate::audit_relay::{http::RelayState, metrics::RelayMetrics, worker::RelayWorker};

const EVENT: &str = "a1111111-1111-4111-8111-111111111111";

fn worker(database: &crate::audit_relay::db::Database) -> (RelayState, RelayWorker) {
    let state = RelayState::new(
        database.clone(),
        std::sync::Arc::new(crate::audit_relay::test_support::config(
            std::path::PathBuf::from("unused"),
        )),
        RelayMetrics::new(),
    );
    let worker = RelayWorker::new(&state).expect("worker builds");
    (state, worker)
}

#[tokio::test]
async fn a_start_is_not_closed_before_its_deadline_plus_grace() {
    let (_dir, database) = open_database();
    let (_state, worker) = worker(&database);
    register(&database, EVENT).await;

    // The fixture's deadline is start + 60s and the grace is 60s.
    worker
        .close_overdue_starts(now() + Duration::seconds(90))
        .await;
    assert_eq!(state_of(&database, EVENT).await, RecordState::Started);

    worker
        .close_overdue_starts(now() + Duration::seconds(180))
        .await;
    assert_eq!(state_of(&database, EVENT).await, RecordState::Incomplete);
}

#[tokio::test]
async fn the_synthesized_record_states_no_status_and_no_actor() {
    let (_dir, database) = open_database();
    let (state, worker) = worker(&database);
    register(&database, EVENT).await;
    worker
        .close_overdue_starts(now() + Duration::seconds(300))
        .await;
    assert_eq!(state.metrics.incomplete_count(), 1);

    let (terminal, actor_id): (Vec<u8>, Option<String>) = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT terminal_json, actor_id FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![EVENT],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| crate::audit_relay::db::classify(&error))
        })
        .await
        .expect("reads the synthesized terminal");
    let body: serde_json::Value = serde_json::from_slice(&terminal).expect("valid JSON");
    assert_eq!(body["status_code"], serde_json::Value::Null);
    assert_eq!(body["outcome"], "incomplete");
    assert_eq!(body["error_code"], INCOMPLETE_ERROR_CODE);
    assert_eq!(body["actor_id"], serde_json::Value::Null);
    assert_eq!(
        actor_id, None,
        "a record with no verified actor stays global-admin-only"
    );
}

#[tokio::test]
async fn a_real_completion_always_beats_the_synthesized_one() {
    let (_dir, database) = open_database();
    let (_state, worker) = worker(&database);
    register(&database, EVENT).await;
    commit(&database, completion(EVENT, Some(101))).await;

    worker
        .close_overdue_starts(now() + Duration::seconds(600))
        .await;
    assert_eq!(state_of(&database, EVENT).await, RecordState::Complete);
}

#[tokio::test]
async fn retention_removes_verified_rows_only() {
    let (_dir, database) = open_database();
    let (state, worker) = worker(&database);
    durable_request(&database, EVENT, Some(101)).await;

    // A `complete` row is never purged, however old.
    worker.purge_expired(now() + Duration::days(365)).await;
    assert_eq!(
        database.read(ingest::record_count).await.expect("counts"),
        1
    );

    let ids = vec![EVENT.to_string()];
    let accepted = ids.clone();
    database
        .write(move |tx| crate::audit_relay::db::delivery::mark_accepted(tx, &accepted, now()))
        .await
        .expect("acceptance commits");
    database
        .write(move |tx| crate::audit_relay::db::delivery::mark_verified(tx, &ids, now()))
        .await
        .expect("verification commits");

    worker.purge_expired(now() + Duration::days(365)).await;
    assert_eq!(
        database.read(ingest::record_count).await.expect("counts"),
        0
    );
    let _ = state;
}
