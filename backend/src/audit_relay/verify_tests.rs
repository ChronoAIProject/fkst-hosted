//! Verification-pass tests: only a query read-back verifies, absence inside the
//! lag window is tolerated, and absence past it re-captures the SAME event id.

use k8s_openapi::chrono::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit_relay::db::delivery;
use crate::audit_relay::metrics::VerificationResult;
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    accepting_capture, durable_request, now, state_of, verification_returning, worker_against,
};

const FIRST: &str = "a1111111-1111-4111-8111-111111111111";
const SECOND: &str = "b1111111-1111-4111-8111-111111111111";

#[tokio::test]
async fn a_record_is_verified_only_when_the_query_reads_it_back() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    verification_returning(&server, &[FIRST]).await;
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

    // Past the ingestion-lag delay, only the id PostHog returned is verified.
    worker.verify_accepted(now() + Duration::seconds(60)).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogVerified
    );
    assert_eq!(
        state_of(&state.db, SECOND).await,
        RecordState::PosthogAccepted,
        "absence inside the lag window is a delay, not a loss"
    );
    assert_eq!(
        state
            .metrics
            .verification_count(VerificationResult::Verified),
        1
    );
    assert_eq!(
        state.metrics.verification_count(VerificationResult::Absent),
        1
    );
}

#[tokio::test]
async fn verification_does_not_run_inside_the_configured_delay() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    verification_returning(&server, &[FIRST]).await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    // The fixture's delay is 30s; a sweep one second later must not verify.
    worker.verify_accepted(now() + Duration::seconds(1)).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted
    );
}

#[tokio::test]
async fn an_event_absent_past_the_lag_threshold_is_recaptured_with_the_same_id() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    verification_returning(&server, &[]).await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    // The fixture's max age is 300s.
    worker.verify_accepted(now() + Duration::seconds(600)).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::Complete,
        "an unverifiable record goes back for another capture"
    );
    let due = state
        .db
        .read(|connection| delivery::claim_due(connection, now() + Duration::seconds(601), 10))
        .await
        .expect("claims");
    assert_eq!(due.len(), 1);
    assert_eq!(
        due[0].event_id, FIRST,
        "the re-capture reuses the SAME uuid so PostHog deduplicates"
    );
}

#[tokio::test]
async fn a_failing_verification_query_leaves_records_accepted_not_verified() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    Mock::given(method("POST"))
        .and(path("/api/projects/42/query/"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    worker.verify_accepted(now() + Duration::seconds(60)).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted,
        "a broken verification credential must not fabricate verification"
    );
    assert_eq!(
        state.metrics.verification_count(VerificationResult::Failed),
        1
    );
}

#[tokio::test]
async fn a_relay_without_a_query_key_never_verifies_anything() {
    let server = MockServer::start().await;
    accepting_capture(&server).await;
    let (_dir, state, worker) = worker_against(&server, |config| {
        config.posthog_query_api_key = secrecy::SecretString::from(String::new());
    })
    .await;
    durable_request(&state.db, FIRST, Some(101)).await;

    worker.capture_due(now()).await;
    worker.verify_accepted(now() + Duration::seconds(600)).await;
    assert_eq!(
        state_of(&state.db, FIRST).await,
        RecordState::PosthogAccepted,
        "without verification configured, acceptance is where a record honestly stops"
    );
}
