//! Capture-pass tests: acceptance, retry with backoff, permanent dead letters,
//! and the poison record that dies alone.

use k8s_openapi::chrono::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit_relay::db::{delivery, ingest};
use crate::audit_relay::metrics::{CaptureResult, DeadLetterReason};
use crate::audit_relay::protocol::format_instant;
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    accepting_capture, commit, completion, durable_request, now, register, state_of, worker_against,
};

const FIRST: &str = "a1111111-1111-4111-8111-111111111111";
const SECOND: &str = "b1111111-1111-4111-8111-111111111111";

#[tokio::test]
async fn an_accepted_batch_becomes_accepted_never_verified() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted,
        "capture acceptance must never be called verification"
    );
    assert_eq!(state.metrics.capture_count(CaptureResult::Accepted), 1);
}

#[tokio::test]
async fn a_retryable_failure_schedules_a_backoff_rather_than_dropping() {
    let server = MockServer::start().await;
    for endpoint in ["/capture/", "/batch/"] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
    }
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    assert_eq!(state_of(&state.db, FIRST).await, RecordState::Complete);
    assert_eq!(state.metrics.capture_count(CaptureResult::Retryable), 1);

    // Not due again immediately, but due after the backoff.
    let due_now = state
        .db
        .read(|connection| delivery::claim_due(connection, now(), 10))
        .await
        .expect("claims");
    assert!(due_now.is_empty(), "a retry must wait out its backoff");
    let due_later = state
        .db
        .read(|connection| delivery::claim_due(connection, now() + Duration::seconds(60), 10))
        .await
        .expect("claims");
    assert_eq!(due_later.len(), 1);
}

#[tokio::test]
async fn retries_are_exhausted_into_a_retained_dead_letter() {
    let server = MockServer::start().await;
    for endpoint in ["/capture/", "/batch/"] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(503))
            .mount(&server)
            .await;
    }
    // `max_capture_attempts` is 3 in the fixture.
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    let mut clock = now();
    for _ in 0..3 {
        worker.capture_due(clock).await;
        clock += Duration::seconds(600);
    }
    assert_eq!(state_of(&state.db, FIRST).await, RecordState::DeadLetter);
    assert_eq!(
        state
            .metrics
            .dead_letter_count(DeadLetterReason::AttemptsExhausted),
        1
    );
    // Retained, never purged.
    assert_eq!(
        state.db.read(ingest::record_count).await.expect("counts"),
        1
    );
}

#[tokio::test]
async fn a_permanent_rejection_dead_letters_immediately() {
    let server = MockServer::start().await;
    for endpoint in ["/capture/", "/batch/"] {
        Mock::given(method("POST"))
            .and(path(endpoint))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;
    }
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    assert_eq!(state_of(&state.db, FIRST).await, RecordState::DeadLetter);
    assert_eq!(
        state.metrics.dead_letter_count(DeadLetterReason::Permanent),
        1
    );
}

#[tokio::test]
async fn a_poison_record_dies_alone_and_its_batch_mates_are_accepted() {
    // The batch endpoint rejects permanently (as PostHog does when ONE payload in
    // a batch is unacceptable), while the single-event endpoint accepts. The
    // isolation pass must therefore accept both records rather than condemning
    // the whole batch.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/batch/"))
        .respond_with(ResponseTemplate::new(400))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/capture/"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": 1})))
        .mount(&server)
        .await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;
    durable_request(&state.db, SECOND, Some(101)).await;

    worker.capture_due(now()).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted
    );
    assert_eq!(
        state_of(&state.db, SECOND).await,
        RecordState::PosthogAccepted
    );
}

#[tokio::test]
async fn the_backlog_drains_oldest_first() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    let (_dir, state, worker) =
        worker_against(&server, |config| config.capture_batch_size = 1).await;
    durable_request(&state.db, FIRST, Some(101)).await;
    register(&state.db, SECOND).await;
    let mut later = completion(SECOND, Some(101));
    later.completed_at = format_instant(now() + Duration::seconds(5));
    later.duration_ms = 5_000;
    commit(&state.db, later).await;

    worker.capture_due(now()).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted
    );
    assert_eq!(
        state_of(&state.db, SECOND).await,
        RecordState::Complete,
        "FIFO: the older terminal is attempted first"
    );
}
