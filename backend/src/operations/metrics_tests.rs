//! Metric tests: dense indexing, the closed label sets, and the guarantee that
//! no identity-bearing value can become a series.

use super::*;
use crate::operations::filters::RecordKind;
use crate::operations::record::ActivitySourceKind;

#[test]
fn every_label_tuple_is_exported_even_when_it_is_zero() {
    let snapshot = ActivityMetrics::new().snapshot();
    // 2 scopes x 3 record kinds x the documented result set.
    assert_eq!(snapshot.queries().count(), 2 * 3 * QueryResult::COUNT);
    assert_eq!(
        snapshot.source_durations().count(),
        ActivitySourceKind::ALL.len() * SourceResult::COUNT
    );
    assert_eq!(snapshot.rows().count(), RowResult::COUNT);
    assert_eq!(snapshot.partial().count(), ActivitySourceKind::ALL.len());
    assert_eq!(snapshot.rejections().count(), RejectionReason::COUNT);
    assert!(snapshot.queries().all(|(_, _, _, count)| count == 0));
}

#[test]
fn each_counter_lands_in_its_own_series() {
    let metrics = ActivityMetrics::new();
    metrics.record_query("mine", RecordKind::ApiRequest, QueryResult::Success);
    metrics.record_query("all", RecordKind::All, QueryResult::Forbidden);
    metrics.record_query("mine", RecordKind::SandboxLifecycle, QueryResult::NotFound);

    let snapshot = metrics.snapshot();
    let found: Vec<_> = snapshot
        .queries()
        .filter(|(_, _, _, count)| *count > 0)
        .collect();
    assert_eq!(
        found,
        vec![
            ("mine", "api_request", "success", 1),
            ("mine", "sandbox_lifecycle", "not_found", 1),
            ("all", "all", "forbidden", 1),
        ]
    );
}

#[test]
fn source_durations_are_summed_per_source_and_result() {
    let metrics = ActivityMetrics::new();
    metrics.record_source(ActivitySourceKind::Posthog, SourceResult::Success, 120);
    metrics.record_source(ActivitySourceKind::Posthog, SourceResult::Success, 80);
    metrics.record_source(
        ActivitySourceKind::Relay,
        SourceResult::UpstreamError,
        1_000,
    );
    let snapshot = metrics.snapshot();
    let found: Vec<_> = snapshot
        .source_durations()
        .filter(|(_, _, _, count)| *count > 0)
        .collect();
    assert_eq!(
        found,
        vec![
            ("posthog", "success", 200, 2),
            ("relay", "upstream_error", 1_000, 1),
        ]
    );
}

#[test]
fn row_partial_and_rejection_counters_use_their_closed_vocabularies() {
    let metrics = ActivityMetrics::new();
    metrics.record_rows(RowResult::Returned, 5);
    metrics.record_rows(RowResult::Invalid, 2);
    metrics.record_rows(RowResult::ConstraintViolation, 0);
    metrics.record_partial(ActivitySourceKind::Posthog);
    metrics.record_rejection(RejectionReason::LifecycleSession);
    metrics.record_rejection(RejectionReason::Capacity);

    let snapshot = metrics.snapshot();
    assert_eq!(
        snapshot.rows().collect::<Vec<_>>(),
        vec![
            ("returned", 5),
            ("invalid", 2),
            ("duplicate", 0),
            ("constraint_violation", 0),
        ]
    );
    assert_eq!(
        snapshot.partial().collect::<Vec<_>>(),
        vec![("posthog", 1), ("relay", 0)]
    );
    assert_eq!(
        snapshot
            .rejections()
            .filter(|(_, count)| *count > 0)
            .collect::<Vec<_>>(),
        vec![
            ("lifecycle_session_forbidden", 1),
            ("capacity_exhausted", 1)
        ]
    );
}

/// Every label value is a compile-time constant drawn from a closed enum, so the
/// series count cannot depend on request content.
#[test]
fn every_label_value_is_a_bounded_snake_case_constant() {
    let snapshot = ActivityMetrics::new().snapshot();
    let mut labels: Vec<&'static str> = Vec::new();
    for (scope, kind, result, _) in snapshot.queries() {
        labels.extend([scope, kind, result]);
    }
    for (source, result, _, _) in snapshot.source_durations() {
        labels.extend([source, result]);
    }
    labels.extend(snapshot.rows().map(|(result, _)| result));
    labels.extend(snapshot.partial().map(|(source, _)| source));
    labels.extend(snapshot.rejections().map(|(reason, _)| reason));
    for label in labels {
        assert!(!label.is_empty());
        assert!(
            label
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
            "{label} is not a bounded snake_case constant"
        );
    }
}

#[test]
fn clones_share_one_backing_store() {
    let metrics = ActivityMetrics::new();
    let clone = metrics.clone();
    clone.record_rejection(RejectionReason::Cursor);
    assert_eq!(
        metrics
            .snapshot()
            .rejections()
            .find(|(reason, _)| *reason == "invalid_cursor")
            .map(|(_, count)| count),
        Some(1)
    );
}
