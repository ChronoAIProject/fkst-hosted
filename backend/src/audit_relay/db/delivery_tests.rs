//! Delivery-state tests: FIFO claiming, the accepted/verified split, the
//! atomic incomplete close, and the retention rule that only ever deletes
//! verified rows.

use k8s_openapi::chrono::Duration;

use super::*;
use crate::audit_relay::db::{ingest, Database};
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    commit, completion, durable_request, now, open_database, register,
};

const FIRST: &str = "a1111111-1111-4111-8111-111111111111";
const SECOND: &str = "b1111111-1111-4111-8111-111111111111";

async fn state_of(database: &Database, event_id: &str) -> RecordState {
    let event_id = event_id.to_string();
    let raw: String = database
        .read(move |connection| {
            connection
                .query_row(
                    "SELECT state FROM audit_records WHERE event_id = ?1",
                    rusqlite::params![event_id],
                    |row| row.get(0),
                )
                .map_err(|error| crate::audit_relay::db::classify(&error))
        })
        .await
        .expect("reads the state");
    RecordState::parse(&raw).expect("a known state")
}

#[tokio::test]
async fn due_records_are_claimed_oldest_terminal_first() {
    let (_dir, database) = open_database();
    // SECOND completes one second AFTER first, so FIRST must be attempted first
    // even though its id sorts later than nothing in particular.
    durable_request(&database, FIRST, Some(101)).await;
    register(&database, SECOND).await;
    let mut later = completion(SECOND, Some(101));
    later.completed_at = crate::audit_relay::protocol::format_instant(now() + Duration::seconds(1));
    later.duration_ms = 1_000;
    commit(&database, later).await;

    let claimed = database
        .read(|connection| claim_due(connection, now() + Duration::seconds(5), 10))
        .await
        .expect("claims");
    let ids: Vec<&str> = claimed.iter().map(|r| r.event_id.as_str()).collect();
    assert_eq!(ids, vec![FIRST, SECOND]);
}

#[tokio::test]
async fn acceptance_and_verification_are_separate_transitions() {
    let (_dir, database) = open_database();
    durable_request(&database, FIRST, Some(101)).await;

    let ids = vec![FIRST.to_string()];
    let accepted = ids.clone();
    database
        .write(move |tx| mark_accepted(tx, &accepted, now()))
        .await
        .expect("acceptance commits");
    assert_eq!(
        state_of(&database, FIRST).await,
        RecordState::PosthogAccepted
    );

    // An accepted row is NOT claimable for capture again.
    let due = database
        .read(|connection| claim_due(connection, now() + Duration::hours(1), 10))
        .await
        .expect("claims");
    assert!(due.is_empty());

    let verified = ids;
    database
        .write(move |tx| mark_verified(tx, &verified, now()))
        .await
        .expect("verification commits");
    assert_eq!(
        state_of(&database, FIRST).await,
        RecordState::PosthogVerified
    );
}

#[tokio::test]
async fn a_retry_schedules_the_next_attempt_and_bumps_the_counter() {
    let (_dir, database) = open_database();
    durable_request(&database, FIRST, Some(101)).await;
    let next = now() + Duration::seconds(30);
    database
        .write(move |tx| mark_retry(tx, FIRST, next, "retryable", now()))
        .await
        .expect("retry commits");

    // Not due yet.
    let early = database
        .read(|connection| claim_due(connection, now() + Duration::seconds(5), 10))
        .await
        .expect("claims");
    assert!(early.is_empty());

    let due = database
        .read(|connection| claim_due(connection, now() + Duration::seconds(60), 10))
        .await
        .expect("claims");
    assert_eq!(due.len(), 1);
    assert_eq!(due[0].capture_attempts, 1);
    assert_eq!(due[0].last_delivery_code.as_deref(), Some("retryable"));
}

#[tokio::test]
async fn a_dead_letter_is_retained_and_never_reclaimed() {
    let (_dir, database) = open_database();
    durable_request(&database, FIRST, Some(101)).await;
    database
        .write(move |tx| mark_dead_letter(tx, FIRST, "permanent", now()))
        .await
        .expect("dead letter commits");
    assert_eq!(state_of(&database, FIRST).await, RecordState::DeadLetter);
    let due = database
        .read(|connection| claim_due(connection, now() + Duration::days(30), 10))
        .await
        .expect("claims");
    assert!(due.is_empty(), "a dead letter must not be retried forever");
    assert_eq!(
        database.read(ingest::record_count).await.expect("counts"),
        1,
        "a dead letter is RETAINED"
    );
}

#[tokio::test]
async fn an_overdue_start_is_closed_atomically_and_a_real_completion_wins() {
    let (_dir, database) = open_database();
    register(&database, FIRST).await;
    let overdue = database
        .read(|connection| claim_overdue_starts(connection, now() + Duration::seconds(120), 60, 10))
        .await
        .expect("claims");
    assert_eq!(overdue.len(), 1);

    // A real completion lands first.
    commit(&database, completion(FIRST, Some(101))).await;
    let closed = database
        .write(move |tx| synthesize_incomplete(tx, FIRST, b"{}", now(), now()))
        .await
        .expect("the synthesis runs");
    assert!(
        !closed,
        "a synthesized incomplete must never overwrite a real completion"
    );
    assert_eq!(state_of(&database, FIRST).await, RecordState::Complete);
}

#[tokio::test]
async fn only_verified_rows_are_ever_purged() {
    let (_dir, database) = open_database();
    // One of each state that must survive, plus one verified row past retention.
    durable_request(&database, FIRST, Some(101)).await;
    register(&database, SECOND).await;
    let dead = "c1111111-1111-4111-8111-111111111111";
    durable_request(&database, dead, Some(101)).await;
    database
        .write(move |tx| mark_dead_letter(tx, dead, "permanent", now()))
        .await
        .expect("dead letter commits");

    let verified = "d1111111-1111-4111-8111-111111111111";
    durable_request(&database, verified, Some(101)).await;
    let accepted = vec![verified.to_string()];
    let long_ago = now() - Duration::days(30);
    database
        .write(move |tx| mark_accepted(tx, &accepted, long_ago))
        .await
        .expect("acceptance commits");
    let to_verify = vec![verified.to_string()];
    database
        .write(move |tx| mark_verified(tx, &to_verify, long_ago))
        .await
        .expect("verification commits");

    let purged = database
        .write(move |tx| purge_verified(tx, now() - Duration::days(7)))
        .await
        .expect("purge runs");
    assert_eq!(purged, 1);
    assert_eq!(
        database.read(ingest::record_count).await.expect("counts"),
        3,
        "complete, started, and dead-letter rows must all survive"
    );
}

#[tokio::test]
async fn an_absent_accepted_record_can_be_requeued_with_the_same_event_id() {
    let (_dir, database) = open_database();
    durable_request(&database, FIRST, Some(101)).await;
    let accepted = vec![FIRST.to_string()];
    database
        .write(move |tx| mark_accepted(tx, &accepted, now()))
        .await
        .expect("acceptance commits");

    let stale = database
        .read(|connection| {
            accepted_before(
                connection,
                &[FIRST.to_string()],
                now() + Duration::seconds(1),
            )
        })
        .await
        .expect("finds the stale record");
    assert_eq!(stale.len(), 1);

    database
        .write(move |tx| requeue_for_recapture(tx, FIRST, RecordState::Complete, now()))
        .await
        .expect("requeue commits");
    assert_eq!(state_of(&database, FIRST).await, RecordState::Complete);
    let due = database
        .read(|connection| claim_due(connection, now() + Duration::seconds(1), 10))
        .await
        .expect("claims");
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].event_id, FIRST,
        "a re-capture reuses the SAME event id so PostHog deduplicates"
    );
    assert!(due[0].posthog_accepted_at.is_none());
}
