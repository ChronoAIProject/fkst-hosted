//! Ingress tests: exact idempotent replay, conflict safety, and the refusal of
//! a completion with no registered start.

use super::*;
use crate::audit_relay::db::DbError;
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    commit, completion, durable_lifecycle, lifecycle, now, open_database, register, start,
    wire_instant,
};

const EVENT: &str = "11111111-1111-4111-8111-111111111111";

#[tokio::test]
async fn a_first_start_is_created_and_an_exact_replay_is_acknowledged() {
    let (_dir, database) = open_database();
    assert_eq!(
        register(&database, EVENT).await,
        Ingested::Created(RecordState::Started)
    );
    assert_eq!(
        register(&database, EVENT).await,
        Ingested::Replayed(RecordState::Started)
    );
    let stored = database.read(record_count).await.expect("counts");
    assert_eq!(stored, 1, "a replay must never create a second row");
}

#[tokio::test]
async fn a_start_replayed_with_different_content_is_a_conflict() {
    let (_dir, database) = open_database();
    register(&database, EVENT).await;
    let mut divergent = start(EVENT);
    divergent.operation_id = "logs_download".to_string();
    let identity = divergent.to_identity().expect("valid start");
    let error = database
        .write(move |tx| register_start(tx, &divergent, &identity, now()))
        .await
        .expect_err("a divergent start conflicts");
    assert_eq!(error, DbError::Conflict);
}

#[tokio::test]
async fn a_completion_with_no_registered_start_is_refused() {
    let (_dir, database) = open_database();
    let terminal = completion(EVENT, Some(101));
    let error = database
        .write(move |tx| {
            commit_completion(tx, &terminal, wire_instant(&terminal.completed_at), now())
        })
        .await
        .expect_err("a completion needs a start");
    assert_eq!(error, DbError::NoStart);
    // Nothing may be auto-registered in its place: the whole value of `required`
    // mode is that a start exists BEFORE the handler ran.
    assert_eq!(database.read(record_count).await.expect("counts"), 0);
}

#[tokio::test]
async fn a_completion_disagreeing_with_its_start_is_a_conflict() {
    let (_dir, database) = open_database();
    register(&database, EVENT).await;
    let mut terminal = completion(EVENT, Some(101));
    terminal.method = "DELETE".to_string();
    let error = database
        .write(move |tx| {
            commit_completion(tx, &terminal, wire_instant(&terminal.completed_at), now())
        })
        .await
        .expect_err("an immutable-field disagreement conflicts");
    assert_eq!(error, DbError::Conflict);
}

#[tokio::test]
async fn an_exact_completion_replay_is_acknowledged_and_a_different_one_conflicts() {
    let (_dir, database) = open_database();
    register(&database, EVENT).await;
    assert_eq!(
        commit(&database, completion(EVENT, Some(101))).await,
        Ingested::Created(RecordState::Complete)
    );
    assert_eq!(
        commit(&database, completion(EVENT, Some(101))).await,
        Ingested::Replayed(RecordState::Complete)
    );

    let mut different = completion(EVENT, Some(101));
    different.status_code = Some(500);
    different.outcome = "server_error".to_string();
    let error = database
        .write(move |tx| {
            commit_completion(tx, &different, wire_instant(&different.completed_at), now())
        })
        .await
        .expect_err("a second, different terminal conflicts");
    assert_eq!(error, DbError::Conflict);

    // History is intact: the FIRST terminal is still what is stored.
    let stored = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT terminal_json FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![EVENT],
                    |row| row.get::<_, Vec<u8>>(0),
                )
                .map_err(|error| crate::audit_relay::db::classify(&error))
        })
        .await
        .expect("reads the stored terminal");
    let decoded: serde_json::Value =
        serde_json::from_slice(&stored).expect("the stored body is JSON");
    assert_eq!(decoded["status_code"], serde_json::json!(200));
}

#[tokio::test]
async fn a_completion_stores_the_scope_columns_for_indexed_reads() {
    let (_dir, database) = open_database();
    register(&database, EVENT).await;
    commit(
        &database,
        crate::audit_relay::test_support::completion_in_session(EVENT, Some(101), "sess-1"),
    )
    .await;
    let (actor_id, session_id): (Option<String>, Option<String>) = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT actor_id, session_id FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![EVENT],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|error| crate::audit_relay::db::classify(&error))
        })
        .await
        .expect("reads the scope columns");
    assert_eq!(actor_id.as_deref(), Some("101"));
    assert_eq!(session_id.as_deref(), Some("sess-1"));
}

#[tokio::test]
async fn a_lifecycle_event_is_terminal_on_arrival_and_idempotent() {
    let (_dir, database) = open_database();
    durable_lifecycle(&database, EVENT, "sess-1").await;
    let event = lifecycle(EVENT, "sess-1");
    let replay = database
        .write(move |tx| commit_lifecycle(tx, &event, wire_instant(&event.occurred_at), now()))
        .await
        .expect("the replay is acknowledged");
    assert_eq!(replay, Ingested::Replayed(RecordState::Complete));

    let mut divergent = lifecycle(EVENT, "sess-1");
    divergent.lifecycle_action = "deleted".to_string();
    let error = database
        .write(move |tx| {
            commit_lifecycle(tx, &divergent, wire_instant(&divergent.occurred_at), now())
        })
        .await
        .expect_err("a divergent lifecycle event conflicts");
    assert_eq!(error, DbError::Conflict);
}

#[tokio::test]
async fn a_lifecycle_row_carries_no_actor_id() {
    // A system effect is visible through its SESSION, never through an actor: an
    // actor id here would make a reconciler action look like a person's own call.
    let (_dir, database) = open_database();
    durable_lifecycle(&database, EVENT, "sess-1").await;
    let actor_id: Option<String> = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT actor_id FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![EVENT],
                    |row| row.get(0),
                )
                .map_err(|error| crate::audit_relay::db::classify(&error))
        })
        .await
        .expect("reads the actor column");
    assert_eq!(actor_id, None);
}

#[tokio::test]
async fn state_and_age_gauges_group_by_the_closed_state_vocabulary() {
    let (_dir, database) = open_database();
    register(&database, EVENT).await;
    let second = "22222222-2222-4222-8222-222222222222";
    register(&database, second).await;
    commit(&database, completion(second, Some(101))).await;

    let counts = database.read(state_counts).await.expect("counts by state");
    assert!(counts.contains(&("started".to_string(), 1)));
    assert!(counts.contains(&("complete".to_string(), 1)));

    let oldest = database
        .read(oldest_per_state)
        .await
        .expect("oldest by state");
    assert_eq!(oldest.len(), 2);
}

#[tokio::test]
async fn stored_sort_timestamps_are_canonical_whatever_the_wire_rendering_was() {
    // `terminal_at` and `completion_deadline_at` are compared as TEXT by the
    // read window and by the overdue sweep. Storing a caller's own rendering
    // would make two spellings of one instant order differently, so both are
    // written through `format_instant`.
    let (_dir, database) = open_database();
    let mut equivalent_start = start(EVENT);
    equivalent_start.completion_deadline_at = "2026-07-31T12:01:00Z".to_string();
    let identity = equivalent_start.to_identity().expect("valid start");
    database
        .write(move |tx| register_start(tx, &equivalent_start, &identity, now()))
        .await
        .expect("the start commits");

    let mut equivalent_terminal = completion(EVENT, Some(101));
    equivalent_terminal.completed_at = "2026-07-31T12:00:00.120+00:00".to_string();
    let terminal_at = wire_instant(&equivalent_terminal.completed_at);
    database
        .write(move |tx| commit_completion(tx, &equivalent_terminal, terminal_at, now()))
        .await
        .expect("the completion commits");

    let stored: (String, Option<String>) = database
        .read(|connection| {
            connection
                .query_row(
                    "SELECT completion_deadline_at, terminal_at FROM audit_records \
                     WHERE event_id = ?1",
                    rusqlite::params![EVENT],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .map_err(|_| DbError::Internal("read"))
        })
        .await
        .expect("the row is readable");
    assert_eq!(stored.0, "2026-07-31T12:01:00.000Z");
    assert_eq!(stored.1.as_deref(), Some("2026-07-31T12:00:00.120Z"));
}
