//! Unit tests for the RESERVED operations-surface safe arguments.
//!
//! The routes arrive with issues #5672/#5675; these prove the boundary they will
//! attach to is already correct, so neither issue has to invent one.

use super::*;
use crate::audit::arguments::test_support::{
    assert_no_canary, assert_policy_matches, assert_within_allowlist, properties, string,
};

fn activity() -> SafeOperationsListActivity {
    SafeOperationsListActivity {
        scope: ActivityScope::Mine,
        requested_scope: Some(ActivityScope::All),
        record_kind: ActivityRecordKind::ApiRequest,
        from: Some("2026-07-01T00:00:00.000Z".to_string()),
        to: Some("2026-07-31T00:00:00.000Z".to_string()),
        limit: 50,
        cursor_present: true,
        actor_filter_present: true,
        session_id: filter_session_id("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e"),
        repo_full_name: filter_repo_full_name("acme", "site"),
        trigger_issue: Some(42),
        request_id: Some("req-1".to_string()),
        method: Some("GET".to_string()),
        operation_id: Some("canvas_overview".to_string()),
        status: Some(403),
        status_class: Some("4xx".to_string()),
        outcome: Some("client_error".to_string()),
    }
}

#[test]
fn both_reserved_dtos_are_wired_to_their_declared_policies() {
    assert_policy_matches::<SafeOperationsListActivity>();
    assert_policy_matches::<SafeOperationsListSandboxes>();
}

/// A denied cross-user probe records the ATTEMPT — the closed requested scope
/// and a boolean — never the login or id that was guessed at, and never the
/// cursor or the query the server ran.
#[test]
fn a_denied_probe_records_the_attempt_and_not_the_probe() {
    let safe = activity();
    assert_within_allowlist(&safe);
    assert_no_canary(
        &safe,
        &[
            "canary-other-login",
            "canary-cursor-payload",
            "canary-hogql",
            "canary-project-key",
        ],
    );

    let values = properties(&safe);
    assert_eq!(string(&values, "scope").as_deref(), Some("mine"));
    assert_eq!(string(&values, "requested_scope").as_deref(), Some("all"));
    assert_eq!(
        values.get("actor_filter_present").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        values.get("cursor_present").and_then(|v| v.as_bool()),
        Some(true)
    );
    assert_eq!(
        string(&values, "record_kind").as_deref(),
        Some("api_request")
    );
    // Every ACCEPTED filter that becomes a source predicate must be
    // reconstructable from the record: a reader given a query narrowed by
    // status family and outcome must not see a record with no constraint at all.
    assert_eq!(values.get("status").and_then(|v| v.as_u64()), Some(403));
    assert_eq!(string(&values, "status_class").as_deref(), Some("4xx"));
    assert_eq!(string(&values, "outcome").as_deref(), Some("client_error"));
}

/// The verified actor is already the record's `actor_id`; duplicating it inside
/// `arguments` would invite a reader to filter on a field that is not the
/// authorization one.
#[test]
fn the_activity_allowlist_names_no_actor_identity_property() {
    for forbidden in ["actor_id", "actor_login", "viewer_id", "cursor", "hogql"] {
        assert!(
            !SafeOperationsListActivity::ALLOWED_FIELDS.contains(&forbidden),
            "{forbidden} must never be an activity argument"
        );
    }
}

#[test]
fn a_sandbox_query_records_its_effective_scope_and_normalized_filters() {
    let safe = SafeOperationsListSandboxes {
        scope: SandboxScope::Accessible,
        requested_scope: Some(SandboxScope::All),
        session_id: filter_session_id("8f0a1c22-6b1e-11ee-9d0e-2f7a1b3c4d5e"),
        repo_full_name: filter_repo_full_name("acme", "site"),
        trigger_issue: Some(7),
        status: Some("running".to_string()),
        backend: Some("kubernetes".to_string()),
        creator_id: Some(101),
        creator_login: Some("alice".to_string()),
        attribution_source: Some("launch_metadata".to_string()),
    };
    assert_within_allowlist(&safe);
    let values = properties(&safe);
    assert_eq!(string(&values, "scope").as_deref(), Some("accessible"));
    assert_eq!(string(&values, "requested_scope").as_deref(), Some("all"));
    assert_eq!(
        string(&values, "repo_full_name").as_deref(),
        Some("acme/site")
    );
}

/// An exact unauthorized/nonexistent session probe must be indistinguishable, so
/// an unvalidated filter is never echoed back into the trail.
#[test]
fn unvalidated_filters_are_dropped_before_they_become_properties() {
    assert_eq!(filter_session_id("canary-session/../escape"), None);
    assert_eq!(filter_repo_full_name("acme", "canary site"), None);
}

#[test]
fn every_scope_and_kind_renders_its_closed_wire_value() {
    for (safe, expected) in [
        (
            SafeOperationsListActivity {
                scope: ActivityScope::All,
                record_kind: ActivityRecordKind::SandboxLifecycle,
                ..activity()
            },
            ("all", "sandbox_lifecycle"),
        ),
        (
            SafeOperationsListActivity {
                scope: ActivityScope::Mine,
                record_kind: ActivityRecordKind::All,
                ..activity()
            },
            ("mine", "all"),
        ),
    ] {
        let values = properties(&safe);
        assert_eq!(string(&values, "scope").as_deref(), Some(expected.0));
        assert_eq!(string(&values, "record_kind").as_deref(), Some(expected.1));
    }
}
