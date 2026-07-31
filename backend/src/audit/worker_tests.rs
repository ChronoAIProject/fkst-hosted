//! End-to-end worker behaviour against a mock PostHog: batching, retry with
//! stable deduplication uuids, queue-full backpressure, and the bounded drain.

use std::time::Duration;

use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::audit::config::AuditConfig;
use crate::audit::test_support::human_event;

/// Build a worker sink over `server` with the given overrides, plus the metrics
/// handle the assertions read.
fn sink_for(server: &MockServer, extra: &[(&str, &str)]) -> (PostHogSink, AuditMetrics) {
    let uri = server.uri();
    let pairs = crate::audit::test_support::merge_vars(
        &[
            ("FKST_POSTHOG_ENABLED", "true"),
            ("FKST_POSTHOG_HOST", uri.as_str()),
            ("FKST_POSTHOG_PROJECT_TOKEN", "phc_t"),
            ("FKST_DEPLOYMENT_ENVIRONMENT", "test"),
            // Fast timings so the tests observe real transitions quickly.
            ("FKST_POSTHOG_FLUSH_INTERVAL_MS", "50"),
            ("FKST_POSTHOG_RETRY_INITIAL_MS", "10"),
            ("FKST_POSTHOG_RETRY_MAX_MS", "20"),
            ("FKST_POSTHOG_SHUTDOWN_FLUSH_SECS", "5"),
        ],
        extra,
    );
    let config = AuditConfig::from_vars(&pairs).expect("test config is valid");
    let client = crate::audit::posthog::PostHogClient::from_config(&config).expect("client");
    let metrics = AuditMetrics::new();
    (
        crate::audit::worker::spawn(&config, client, metrics.clone()),
        metrics,
    )
}

fn ok_response() -> ResponseTemplate {
    ResponseTemplate::new(200).set_body_json(serde_json::json!({"status": 1}))
}

/// Poll until `condition` holds or the budget expires. Cheaper and far less
/// flaky than sleeping for a fixed worst case.
async fn wait_until(label: &str, mut condition: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while std::time::Instant::now() < deadline {
        if condition() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("timed out waiting for {label}");
}

async fn request_bodies(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("recorded")
        .iter()
        .map(|request| serde_json::from_slice(&request.body).expect("JSON body"))
        .collect()
}

#[tokio::test]
async fn events_are_batched_up_to_the_configured_size() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/batch/"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(
        &server,
        &[
            ("FKST_POSTHOG_BATCH_SIZE", "3"),
            // Long enough that only the size trigger can fire.
            ("FKST_POSTHOG_FLUSH_INTERVAL_MS", "60000"),
        ],
    );
    for index in 0..3 {
        let mut event = human_event();
        event.request_id = format!("req-{index}");
        sink.submit(event).expect("admitted");
    }

    wait_until("the size-triggered batch", || {
        metrics.snapshot().batches_accepted == 1
    })
    .await;
    let bodies = request_bodies(&server).await;
    assert_eq!(bodies.len(), 1, "one batch, not one request per event");
    assert_eq!(bodies[0]["batch"].as_array().expect("batch array").len(), 3);
    assert_eq!(metrics.snapshot().attempts_accepted, 1);
    assert_eq!(sink.queue_depth(), 0);
}

#[tokio::test]
async fn a_partial_batch_is_flushed_on_the_interval() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/capture/"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(&server, &[("FKST_POSTHOG_BATCH_SIZE", "50")]);
    sink.submit(human_event()).expect("admitted");

    wait_until("the interval flush", || {
        metrics.snapshot().batches_accepted == 1
    })
    .await;
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn a_retried_batch_reuses_the_same_event_uuid() {
    // Stable uuids are the whole deduplication story: an at-least-once retry
    // must not create a second row in PostHog.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(503))
        .up_to_n_times(1)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(&server, &[("FKST_POSTHOG_BATCH_SIZE", "1")]);
    let event = human_event();
    let expected_uuid = event.event_id.to_string();
    sink.submit(event).expect("admitted");

    wait_until("the retried batch to be accepted", || {
        metrics.snapshot().batches_accepted == 1
    })
    .await;

    let bodies = request_bodies(&server).await;
    assert_eq!(bodies.len(), 2, "one failed attempt plus one retry");
    for body in &bodies {
        assert_eq!(body["uuid"], serde_json::json!(expected_uuid));
    }
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.attempts_retryable, 1);
    assert_eq!(snapshot.attempts_accepted, 1);
    assert_eq!(snapshot.batches_retryable, 0);
}

#[tokio::test]
async fn retry_exhaustion_drops_the_batch_loudly() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(
        &server,
        &[
            ("FKST_POSTHOG_BATCH_SIZE", "1"),
            ("FKST_POSTHOG_MAX_RETRIES", "2"),
        ],
    );
    sink.submit(human_event()).expect("admitted");

    wait_until("the batch to be abandoned", || {
        metrics.snapshot().batches_retryable == 1
    })
    .await;
    let snapshot = metrics.snapshot();
    // One initial attempt plus two retries.
    assert_eq!(snapshot.attempts_retryable, 3);
    assert_eq!(snapshot.dropped_retryable, 1);
    assert_eq!(snapshot.batches_accepted, 0);
}

#[tokio::test]
async fn a_permanent_rejection_is_not_retried() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(401))
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(&server, &[("FKST_POSTHOG_BATCH_SIZE", "1")]);
    sink.submit(human_event()).expect("admitted");

    wait_until("the permanent rejection", || {
        metrics.snapshot().batches_permanent == 1
    })
    .await;
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.attempts_permanent, 1, "no retries on a 401");
    assert_eq!(snapshot.dropped_permanent, 1);
    assert_eq!(request_bodies(&server).await.len(), 1);
}

#[tokio::test]
async fn a_contract_violating_record_is_dropped_before_any_request() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(&server, &[("FKST_POSTHOG_BATCH_SIZE", "1")]);
    let mut event = human_event();
    event.route_template = "/api/v1/logs?token=shhh".to_string();
    sink.submit(event).expect("admission does not validate");

    wait_until("the invalid record to be dropped", || {
        metrics.snapshot().dropped_invalid == 1
    })
    .await;
    assert!(
        request_bodies(&server).await.is_empty(),
        "a rejected record must never reach the wire"
    );
}

#[tokio::test]
async fn an_oversized_record_is_dropped_with_its_own_reason() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(
        &server,
        &[
            ("FKST_POSTHOG_BATCH_SIZE", "1"),
            ("FKST_POSTHOG_MAX_EVENT_BYTES", "4096"),
        ],
    );
    let mut arguments = serde_json::Map::new();
    arguments.insert("blob".to_string(), serde_json::json!("x".repeat(8_192)));
    sink.submit(
        human_event().with_arguments(arguments, crate::audit::ArgumentsParseStatus::Parsed),
    )
    .expect("admitted");

    wait_until("the oversized record to be dropped", || {
        metrics.snapshot().dropped_oversized == 1
    })
    .await;
    assert!(request_bodies(&server).await.is_empty());
}

#[tokio::test]
async fn a_full_queue_refuses_admission_instead_of_blocking() {
    // Audit pressure must never become product latency: the newest event is
    // dropped with a metric rather than awaiting capacity.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_response().set_delay(Duration::from_secs(3)))
        .mount(&server)
        .await;

    let (sink, _metrics) = sink_for(
        &server,
        &[
            ("FKST_POSTHOG_BATCH_SIZE", "1"),
            ("FKST_POSTHOG_QUEUE_CAPACITY", "1"),
        ],
    );
    // The first event is picked up immediately and pins the worker in a slow
    // POST; the second fills the one-slot queue; the third has nowhere to go.
    sink.submit(human_event()).expect("first is admitted");
    wait_until("the worker to pick up the first event", || {
        sink.queue_depth() == 0
    })
    .await;
    sink.submit(human_event()).expect("second fills the queue");
    assert_eq!(sink.submit(human_event()), Err(SubmitError::QueueFull));
    assert_eq!(sink.queue_depth(), 1);
}

#[tokio::test]
async fn the_drain_flushes_queued_events_and_reports_no_residue() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(
        &server,
        &[
            // Neither trigger can fire before the drain, so the drain itself is
            // what delivers.
            ("FKST_POSTHOG_BATCH_SIZE", "100"),
            ("FKST_POSTHOG_FLUSH_INTERVAL_MS", "60000"),
        ],
    );
    sink.submit(human_event()).expect("admitted");

    let report = sink.drain().await;
    assert_eq!(report.remaining, 0, "the drain must deliver what it holds");
    assert_eq!(metrics.snapshot().batches_accepted, 1);
    assert_eq!(request_bodies(&server).await.len(), 1);
    assert_eq!(metrics.snapshot().shutdown_remaining, 0);
}

#[tokio::test]
async fn the_drain_closes_admission() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ok_response())
        .mount(&server)
        .await;

    let (sink, _metrics) = sink_for(&server, &[]);
    sink.drain().await;
    assert_eq!(
        sink.submit(human_event()),
        Err(SubmitError::ShuttingDown),
        "no event may slip in behind the drain"
    );
}

#[tokio::test]
async fn an_undeliverable_drain_reports_its_residue() {
    // PostHog down at shutdown: the drain must state what it could not deliver
    // rather than reporting a clean exit. This is the exact outage the shutdown
    // report exists for, so `remaining` must NOT read zero.
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let (sink, metrics) = sink_for(
        &server,
        &[
            ("FKST_POSTHOG_BATCH_SIZE", "100"),
            ("FKST_POSTHOG_FLUSH_INTERVAL_MS", "60000"),
            ("FKST_POSTHOG_MAX_RETRIES", "1"),
            ("FKST_POSTHOG_SHUTDOWN_FLUSH_SECS", "1"),
        ],
    );
    // Neither send trigger can fire before the drain (batch 100, interval 60s),
    // so all three events are in the drain's own batch.
    for index in 0..3 {
        let mut event = human_event();
        event.request_id = format!("req-{index}");
        sink.submit(event).expect("admitted");
    }

    let report = sink.drain().await;
    assert_eq!(
        report.remaining, 3,
        "events the drain could not deliver are remaining, not delivered"
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.shutdown_remaining, 3, "the metric agrees");
    assert_eq!(snapshot.batches_retryable, 1);
    // Counted once, under the reason that names WHEN they were lost; a second
    // count under `retryable` would double-report the same events.
    assert_eq!(snapshot.dropped_shutdown, 3);
    assert_eq!(snapshot.dropped_retryable, 0);
}

#[test]
fn the_retry_delay_never_exceeds_the_configured_maximum() {
    // The cap is a real ceiling: jitter may shorten a maxed-out wait, never
    // stretch it past FKST_POSTHOG_RETRY_MAX_MS.
    let max = Duration::from_millis(5_000);
    for _ in 0..256 {
        // A server hint far above the cap.
        let hinted = retry_delay(Some(Duration::from_secs(3_600)), max, max);
        assert!(hinted <= max, "Retry-After must be capped, got {hinted:?}");
        assert!(hinted >= Duration::from_millis(4_000), "{hinted:?}");

        // Backoff already at the ceiling.
        let backed_off = retry_delay(None, max, max);
        assert!(
            backed_off <= max,
            "backoff must be capped, got {backed_off:?}"
        );

        // A hint below the cap is honoured (jittered), not raised to it.
        let short = retry_delay(Some(Duration::from_millis(100)), max, max);
        assert!(
            short >= Duration::from_millis(80) && short <= Duration::from_millis(120),
            "{short:?}"
        );
    }
}

#[tokio::test]
async fn the_worker_reports_itself_as_delivering() {
    let server = MockServer::start().await;
    let (sink, _metrics) = sink_for(&server, &[]);
    assert!(sink.is_delivering());
}
