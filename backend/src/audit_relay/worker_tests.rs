//! Worker-level fixtures plus the sweep's own assertions: gauges, the capacity
//! latch, and the fact that one failing pass never skips the others.

use std::sync::Arc;

use k8s_openapi::chrono::Duration;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::audit_relay::http::RelayState;
use crate::audit_relay::metrics::RelayMetrics;
use crate::audit_relay::record::RecordState;
use crate::audit_relay::test_support::{
    config, durable_request, local_worker, now, open_database, register, state_of, worker_against,
};

use super::*;

#[tokio::test]
async fn a_relay_without_posthog_still_accepts_and_stays_ready() {
    // The point of an outbox: records commit durably and accumulate while the
    // destination is absent, and readiness stays true.
    let (_dir, database) = open_database();
    let state = RelayState::new(
        database.clone(),
        Arc::new(config(std::path::PathBuf::from("unused"))),
        RelayMetrics::new(),
    );
    let worker = RelayWorker::new(&state).expect("worker builds without PostHog");
    durable_request(&database, "11111111-1111-4111-8111-111111111111", Some(101)).await;

    worker.sweep(now()).await;
    assert_eq!(
        state_of(&database, "11111111-1111-4111-8111-111111111111").await,
        RecordState::Complete
    );
    assert!(state.db.ingress_ready());
}

#[tokio::test]
async fn the_sweep_publishes_the_bounded_state_gauges() {
    let (_dir, state, worker) = local_worker();
    let database = state.db.clone();
    register(&database, "11111111-1111-4111-8111-111111111111").await;
    durable_request(&database, "22222222-2222-4222-8222-222222222222", Some(101)).await;

    worker.sweep(now()).await;
    let gauges = state.metrics.gauges();
    let started = RecordState::ALL
        .iter()
        .position(|state| *state == RecordState::Started)
        .expect("started is a state");
    let complete = RecordState::ALL
        .iter()
        .position(|state| *state == RecordState::Complete)
        .expect("complete is a state");
    assert_eq!(gauges.records[started], 1);
    assert_eq!(gauges.records[complete], 1);
    assert!(gauges.db_bytes > 0, "the on-disk gauge must be measured");
}

#[tokio::test]
async fn the_capacity_latch_flips_on_and_off_with_the_record_count() {
    let (_dir, database) = open_database();
    let mut relay_config = config(std::path::PathBuf::from("unused"));
    relay_config.max_records = 1;
    let state = RelayState::new(
        database.clone(),
        Arc::new(relay_config),
        RelayMetrics::new(),
    );
    let worker = RelayWorker::new(&state).expect("worker builds");

    worker.sweep(now()).await;
    assert!(!state.is_at_capacity());

    register(&database, "11111111-1111-4111-8111-111111111111").await;
    worker.sweep(now()).await;
    assert!(
        state.is_at_capacity(),
        "at the ceiling, ingress must be refused"
    );
}

#[tokio::test]
async fn a_posthog_outage_does_not_stop_incomplete_synthesis() {
    // Capture is down, so nothing is delivered — but the closer must still run,
    // because an invocation that produced no response is exactly what an
    // operator needs to see during an outage.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capture/"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;
    let (_dir, state, worker) = worker_against(&server, |_| {}).await;
    register(&state.db, "11111111-1111-4111-8111-111111111111").await;

    worker.sweep(now() + Duration::seconds(300)).await;
    assert_eq!(
        state_of(&state.db, "11111111-1111-4111-8111-111111111111").await,
        RecordState::Incomplete
    );
    assert_eq!(state.metrics.incomplete_count(), 1);
}
