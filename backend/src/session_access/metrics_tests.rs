//! Unit tests for the bounded scope telemetry.

use super::*;
use crate::github_identity::GithubUser;
use crate::session_access::test_support::policy_with_admins;
use crate::session_access::viewer::{AuthenticatedViewer, ScopeRequest, ViewerScope};

/// A genuinely resolved scope. Built through the sealed path (a verified viewer),
/// because there is deliberately no other way to obtain one — not even in a test.
fn resolved(scope: &'static str) -> Result<ViewerScope, ScopeDenialReason> {
    let (admins, requested) = match scope {
        "mine" => ("", RequestedScope::Personal),
        "all" => ("alice", RequestedScope::Global),
        other => panic!("unexpected scope {other}"),
    };
    let access = policy_with_admins(admins);
    let viewer = AuthenticatedViewer::new(
        GithubUser {
            login: "alice".to_string(),
            id: 1,
        },
        &access,
    );
    viewer.resolve_scope(ScopeRequest::new(Some(requested)))
}

#[test]
fn every_outcome_has_distinct_closed_enum_labels() {
    let mut seen = std::collections::HashSet::new();
    for outcome in ScopeOutcome::ALL {
        let labels = outcome.labels();
        assert!(seen.insert(labels), "duplicate label triple {labels:?}");
        for label in [labels.0, labels.1, labels.2] {
            assert!(
                label.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
                "label {label} must be a bounded slug"
            );
        }
    }
    assert_eq!(seen.len(), ScopeOutcome::COUNT);
}

#[test]
fn classification_distinguishes_default_from_explicit_selection() {
    assert_eq!(
        ScopeOutcome::of(ScopeRequest::new(None), &resolved("mine")),
        ScopeOutcome::MineDefault
    );
    assert_eq!(
        ScopeOutcome::of(
            ScopeRequest::new(Some(RequestedScope::Personal)),
            &resolved("mine")
        ),
        ScopeOutcome::MineExplicit
    );
    assert_eq!(
        ScopeOutcome::of(ScopeRequest::new(None), &resolved("all")),
        ScopeOutcome::AllDefault
    );
    assert_eq!(
        ScopeOutcome::of(
            ScopeRequest::new(Some(RequestedScope::Global)),
            &resolved("all")
        ),
        ScopeOutcome::AllExplicit
    );
}

#[test]
fn classification_names_the_exact_denial_reason() {
    assert_eq!(
        ScopeOutcome::of(
            ScopeRequest::new(Some(RequestedScope::Global)),
            &Err(ScopeDenialReason::GlobalScope)
        ),
        ScopeOutcome::AllForbidden
    );
    assert_eq!(
        ScopeOutcome::of(
            ScopeRequest::new(None).with_cross_actor_filter(),
            &Err(ScopeDenialReason::CrossActorFilter)
        ),
        ScopeOutcome::CrossActorForbidden
    );
}

#[test]
fn counters_are_independent_and_shared_between_clones() {
    let metrics = ScopeMetrics::new();
    let handle = metrics.clone();
    handle.record(ScopeOutcome::MineDefault);
    handle.record(ScopeOutcome::MineDefault);
    metrics.record(ScopeOutcome::AllForbidden);

    let snapshot = metrics.snapshot();
    assert_eq!(snapshot.count(ScopeOutcome::MineDefault), 2);
    assert_eq!(snapshot.count(ScopeOutcome::AllForbidden), 1);
    assert_eq!(snapshot.count(ScopeOutcome::MineExplicit), 0);
}

#[test]
fn debug_output_reports_only_the_series_count() {
    let metrics = ScopeMetrics::new();
    metrics.record(ScopeOutcome::MineDefault);
    let rendered = format!("{metrics:?}");
    assert!(rendered.contains("outcomes"), "{rendered}");
    assert!(!rendered.contains("alice"), "{rendered}");
}
