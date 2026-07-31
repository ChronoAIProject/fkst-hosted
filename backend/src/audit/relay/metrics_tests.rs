//! Client-telemetry tests: closed labels, and the emergency rejection counter.

use std::time::Duration;

use super::*;

#[test]
fn every_phase_and_result_spelling_is_stable() {
    let phases: Vec<&str> = RelayPhase::ALL
        .into_iter()
        .map(RelayPhase::as_str)
        .collect();
    assert_eq!(phases, vec!["start", "completion", "lifecycle", "read"]);
    let results: Vec<&str> = RelayCallResult::ALL
        .into_iter()
        .map(RelayCallResult::as_str)
        .collect();
    assert_eq!(results, vec!["ack", "conflict", "rejected", "unavailable"]);
}

#[test]
fn the_rejection_reasons_separate_an_outage_from_a_conflict_in_each_phase() {
    // Four stable reasons, not two: an outage ("the relay could not answer") and
    // a conflict ("the relay answered, and what it holds is a different fact")
    // are different alerts with different remedies, in both phases.
    let reasons: Vec<&str> = RequiredRejection::ALL
        .into_iter()
        .map(RequiredRejection::as_str)
        .collect();
    assert_eq!(
        reasons,
        vec![
            "audit_ingress_unavailable",
            "audit_ingress_conflict",
            "audit_completion_unconfirmed",
            "audit_completion_conflict",
        ]
    );
}

#[test]
fn calls_and_durations_accumulate_under_their_own_labels() {
    let metrics = RelayClientMetrics::new();
    metrics.record_call(
        RelayPhase::Start,
        RelayCallResult::Ack,
        Duration::from_millis(250),
    );
    metrics.record_call(
        RelayPhase::Start,
        RelayCallResult::Ack,
        Duration::from_millis(750),
    );
    metrics.record_call(
        RelayPhase::Completion,
        RelayCallResult::Unavailable,
        Duration::from_millis(100),
    );
    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.calls(RelayPhase::Start, RelayCallResult::Ack), 2);
    assert!(
        (snapshot.duration_seconds(RelayPhase::Start, RelayCallResult::Ack) - 1.0).abs() < 1e-9
    );
    assert_eq!(
        snapshot.calls(RelayPhase::Completion, RelayCallResult::Unavailable),
        1
    );
    assert_eq!(snapshot.calls(RelayPhase::Read, RelayCallResult::Ack), 0);
}

#[test]
fn rejections_are_counted_per_reason() {
    let metrics = RelayClientMetrics::new();
    metrics.record_rejection(RequiredRejection::IngressUnavailable);
    metrics.record_rejection(RequiredRejection::IngressUnavailable);
    metrics.record_rejection(RequiredRejection::CompletionUnconfirmed);
    let snapshot = metrics.snapshot();
    assert_eq!(
        snapshot.rejections(RequiredRejection::IngressUnavailable),
        2
    );
    assert_eq!(
        snapshot.rejections(RequiredRejection::CompletionUnconfirmed),
        1
    );
}
