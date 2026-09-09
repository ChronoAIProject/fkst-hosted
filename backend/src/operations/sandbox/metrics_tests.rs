//! Unit tests for the bounded inventory telemetry.
//!
//! The property under test is CARDINALITY as much as correctness: every series is
//! a tuple of closed enums, so the exposition size is a compile-time constant and
//! no request can add one.

use super::*;
use std::collections::BTreeSet;

#[test]
fn the_series_count_is_fixed_by_the_closed_label_sets() {
    let snapshot = SandboxMetrics::new().snapshot();
    assert_eq!(snapshot.requests().count(), REQUEST_SERIES);
    assert_eq!(snapshot.durations().count(), DURATION_SERIES);
    assert_eq!(snapshot.items().count(), ITEM_SERIES);
    assert_eq!(snapshot.rejections().count(), SandboxRejectionReason::COUNT);
}

#[test]
fn every_label_tuple_is_unique_so_no_two_series_collide() {
    let snapshot = SandboxMetrics::new().snapshot();
    let requests: BTreeSet<(&str, &str, &str)> = snapshot
        .requests()
        .map(|(backend, scope, result, _)| (backend, scope, result))
        .collect();
    assert_eq!(requests.len(), REQUEST_SERIES);
    let items: BTreeSet<(&str, &str)> = snapshot
        .items()
        .map(|(backend, scope, _)| (backend, scope))
        .collect();
    assert_eq!(items.len(), ITEM_SERIES);
}

#[test]
fn a_request_increments_exactly_its_own_series_and_observes_its_duration() {
    let metrics = SandboxMetrics::new();
    metrics.record_request(
        BackendLabel::Kubernetes,
        ScopeLabel::Accessible,
        InventoryResult::Success,
        250,
    );
    let snapshot = metrics.snapshot();
    let counted: Vec<(&str, &str, &str, u64)> = snapshot
        .requests()
        .filter(|(_, _, _, count)| *count > 0)
        .collect();
    assert_eq!(
        counted,
        vec![("kubernetes", "accessible", "success", 1)],
        "exactly one series moves"
    );
    let durations: Vec<(&str, &str, u64, u64)> = snapshot
        .durations()
        .filter(|(_, _, _, count)| *count > 0)
        .collect();
    assert_eq!(durations, vec![("kubernetes", "success", 250, 1)]);
}

/// The item gauge is a LAST-VALUE aggregate per closed scope category: two
/// requests in the same scope do not accumulate, and a per-requester series never
/// appears.
#[test]
fn the_item_gauge_stores_the_last_authorized_result_size_per_scope() {
    let metrics = SandboxMetrics::new();
    metrics.record_items(BackendLabel::Kubernetes, ScopeLabel::Accessible, 3);
    metrics.record_items(BackendLabel::Kubernetes, ScopeLabel::Accessible, 5);
    metrics.record_items(BackendLabel::Kubernetes, ScopeLabel::All, 42);
    let snapshot = metrics.snapshot();
    let items: Vec<(&str, &str, u64)> = snapshot
        .items()
        .filter(|(_, _, count)| *count > 0)
        .collect();
    assert_eq!(
        items,
        vec![("kubernetes", "accessible", 5), ("kubernetes", "all", 42)]
    );
}

#[test]
fn a_deployment_with_no_runtime_backend_still_has_a_closed_label() {
    assert_eq!(BackendLabel::of(None).as_str(), "none");
    assert_eq!(
        BackendLabel::of(Some(RuntimeBackendKind::Kubernetes)),
        BackendLabel::Kubernetes
    );
    assert_eq!(
        BackendLabel::of(Some(RuntimeBackendKind::OpenSandbox)),
        BackendLabel::OpenSandbox
    );
}

#[test]
fn every_rejection_reason_counts_into_its_own_bounded_slot() {
    let metrics = SandboxMetrics::new();
    for reason in SandboxRejectionReason::ALL {
        metrics.record_rejection(reason);
    }
    let snapshot = metrics.snapshot();
    for (_, count) in snapshot.rejections() {
        assert_eq!(count, 1);
    }
}

/// An unknown session and an unauthorized one share ONE reason, so the counter
/// cannot become the existence oracle the response refuses to be.
#[test]
fn the_rejection_reasons_do_not_distinguish_unknown_from_unauthorized() {
    let rendered: BTreeSet<&str> = SandboxRejectionReason::ALL
        .iter()
        .map(|reason| reason.as_str())
        .collect();
    assert!(rendered.contains("session_not_found"));
    assert!(!rendered.contains("session_forbidden"));
}

/// The counters must never render an identity-bearing value; `Debug` on the
/// handle is the easiest accidental route to one.
#[test]
fn the_debug_projection_is_bounded() {
    let rendered = format!("{:?}", SandboxMetrics::new());
    assert!(rendered.contains("request_series"), "{rendered}");
    assert!(!rendered.contains("AtomicU64"), "{rendered}");
}
