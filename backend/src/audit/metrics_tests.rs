//! The bounded delivery counters and their read projection.

use super::*;

#[test]
fn a_fresh_handle_starts_at_zero() {
    assert_eq!(
        AuditMetrics::new().snapshot(),
        AuditMetricsSnapshot::default()
    );
}

#[test]
fn every_closed_enum_label_has_its_own_counter() {
    let metrics = AuditMetrics::new();
    metrics.record_enqueued(EnqueueResult::Accepted);
    metrics.record_enqueued(EnqueueResult::Accepted);
    metrics.record_enqueued(EnqueueResult::Full);
    metrics.record_enqueued(EnqueueResult::Disabled);
    metrics.record_batch(DeliveryResult::Accepted);
    metrics.record_batch(DeliveryResult::Retryable);
    metrics.record_batch(DeliveryResult::Permanent);
    metrics.record_dropped(DropReason::QueueFull, 1);
    metrics.record_dropped(DropReason::Invalid, 2);
    metrics.record_dropped(DropReason::Oversized, 3);
    metrics.record_dropped(DropReason::Retryable, 4);
    metrics.record_dropped(DropReason::Permanent, 5);
    metrics.record_dropped(DropReason::Shutdown, 6);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.enqueued_accepted, 2);
    assert_eq!(snapshot.enqueued_full, 1);
    assert_eq!(snapshot.enqueued_disabled, 1);
    assert_eq!(snapshot.batches_accepted, 1);
    assert_eq!(snapshot.batches_retryable, 1);
    assert_eq!(snapshot.batches_permanent, 1);
    assert_eq!(snapshot.dropped_queue_full, 1);
    assert_eq!(snapshot.dropped_invalid, 2);
    assert_eq!(snapshot.dropped_oversized, 3);
    assert_eq!(snapshot.dropped_retryable, 4);
    assert_eq!(snapshot.dropped_permanent, 5);
    assert_eq!(snapshot.dropped_shutdown, 6);
}

#[test]
fn delivery_attempts_accumulate_a_duration_sum_and_count() {
    let metrics = AuditMetrics::new();
    metrics.record_delivery_attempt(DeliveryResult::Retryable, Duration::from_millis(500));
    metrics.record_delivery_attempt(DeliveryResult::Accepted, Duration::from_millis(250));

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.attempts_retryable, 1);
    assert_eq!(snapshot.attempts_accepted, 1);
    assert_eq!(snapshot.delivery_duration_count, 2);
    assert!((snapshot.delivery_duration_seconds_sum - 0.75).abs() < f64::EPSILON);
}

#[test]
fn gauges_are_set_rather_than_accumulated() {
    let metrics = AuditMetrics::new();
    metrics.set_queue_depth(9);
    metrics.set_queue_depth(4);
    metrics.set_shutdown_remaining(2);
    metrics.set_shutdown_remaining(0);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.queue_depth, 4);
    assert_eq!(snapshot.shutdown_remaining, 0);
}

#[test]
fn clones_share_one_set_of_counters() {
    // The worker, the sink, and the HTTP state all hold clones; they must all
    // write to the same counters or `/metrics` would show a partial picture.
    let metrics = AuditMetrics::new();
    let clone = metrics.clone();
    clone.record_enqueued(EnqueueResult::Accepted);
    assert_eq!(metrics.snapshot().enqueued_accepted, 1);
}

#[test]
fn capture_success_is_named_accepted_everywhere() {
    // A PostHog 200 means accepted by capture, never proven query-visible.
    assert_eq!(EnqueueResult::Accepted.as_str(), "accepted");
    assert_eq!(DeliveryResult::Accepted.as_str(), "accepted");
    assert_eq!(EnqueueResult::Full.as_str(), "full");
    assert_eq!(DeliveryResult::Retryable.as_str(), "retryable");
    assert_eq!(DeliveryResult::Permanent.as_str(), "permanent");
    assert_eq!(EnqueueResult::Disabled.as_str(), "disabled");
    assert_eq!(DropReason::QueueFull.as_str(), "queue_full");
    assert_eq!(DropReason::Invalid.as_str(), "invalid");
    assert_eq!(DropReason::Oversized.as_str(), "oversized");
    assert_eq!(DropReason::Shutdown.as_str(), "shutdown");
}
