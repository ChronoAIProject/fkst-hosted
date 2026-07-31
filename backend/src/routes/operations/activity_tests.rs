//! Handler-level unit tests for the pure decisions the route makes before any
//! source call: what a refused probe records, and what the effective scope is.
//!
//! The end-to-end authorization matrix (identity, `403`, `404`, cursors, source
//! outages) lives in `tests/operations_activity.rs`, which drives the real
//! router.

use k8s_openapi::chrono::{TimeZone, Utc};

use super::*;
use crate::access_policy::AccessPolicy;
use crate::github_identity::GithubUser;
use crate::routes::operations::query::{normalize, ActivityQueryParams};
use crate::session_access::test_support::policy_with_admins;

fn now() -> k8s_openapi::chrono::DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, 31, 12, 0, 0)
        .single()
        .expect("a valid fixed instant")
}

fn viewer(id: i64, login: &str, access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: login.to_string(),
            id,
        },
        access,
    )
}

fn request(params: ActivityQueryParams) -> NormalizedActivityRequest {
    normalize(&params, now(), 100, 200, 30).expect("the fixture normalizes")
}

#[test]
fn a_regular_callers_natural_scope_is_personal_and_an_admins_is_global() {
    let access = policy_with_admins("root");
    assert_eq!(
        natural_scope(&viewer(101, "alice", &access)),
        ActivityScope::Mine
    );
    assert_eq!(
        natural_scope(&viewer(900, "root", &access)),
        ActivityScope::All
    );
}

#[test]
fn an_allowed_query_records_its_effective_scope_and_normalized_filters() {
    let access = policy_with_admins("");
    let alice = viewer(101, "alice", &access);
    let normalized = request(ActivityQueryParams {
        record_kind: Some("all".to_string()),
        session_id: Some("sess-1".to_string()),
        limit: Some(25),
        cursor: Some("opaque-cursor-text".to_string()),
        ..ActivityQueryParams::default()
    });
    let resolved = alice
        .resolve_scope(scope_request(&normalized))
        .map_err(|_| AppError::ScopeForbidden("unreachable in this fixture".to_string()));
    let safe = safe_arguments(&normalized, &resolved, &alice);

    assert_eq!(safe.scope, ActivityScope::Mine);
    assert!(
        safe.requested_scope.is_none(),
        "an unstated scope adds nothing to the record"
    );
    assert_eq!(safe.record_kind, ActivityRecordKind::All);
    assert_eq!(safe.limit, 25);
    assert!(safe.cursor_present);
    assert!(!safe.actor_filter_present);
    assert_eq!(safe.session_id.as_deref(), Some("sess-1"));
    assert_eq!(safe.from.as_deref(), Some("2026-07-30T12:00:00.000Z"));
    assert_eq!(safe.to.as_deref(), Some("2026-07-31T12:00:00.000Z"));

    // The cursor TEXT is never recorded — only that one was supplied.
    let rendered = serde_json::to_string(&safe).expect("serializes");
    assert!(!rendered.contains("opaque-cursor-text"), "{rendered}");
}

/// A denied cross-user probe must record the ATTEMPT, never the probe.
#[test]
fn a_denied_probe_records_the_attempt_without_the_probed_identity() {
    let access = policy_with_admins("root");
    let alice = viewer(101, "alice", &access);
    let normalized = request(ActivityQueryParams {
        scope: Some("all".to_string()),
        actor_login: Some("carol".to_string()),
        actor_id: Some(9_876_543),
        ..ActivityQueryParams::default()
    });
    let resolved = alice
        .resolve_scope(scope_request(&normalized))
        .map_err(|_| AppError::ScopeForbidden("refused".to_string()));
    assert!(resolved.is_err(), "a regular caller may not do this");

    let safe = safe_arguments(&normalized, &resolved, &alice);
    assert_eq!(
        safe.scope,
        ActivityScope::Mine,
        "the record states the caller's own scope, which is what actually applied"
    );
    assert_eq!(
        safe.requested_scope,
        Some(ActivityScope::All),
        "and the scope they asked for, which is what makes the denial legible"
    );
    assert!(safe.actor_filter_present);

    let rendered = serde_json::to_string(&safe).expect("serializes");
    assert!(!rendered.contains("carol"), "{rendered}");
    assert!(!rendered.contains("9876543"), "{rendered}");
}

#[test]
fn an_administrator_selecting_the_personal_scope_records_both_scopes() {
    let access = policy_with_admins("root");
    let root = viewer(900, "root", &access);
    let normalized = request(ActivityQueryParams {
        scope: Some("mine".to_string()),
        ..ActivityQueryParams::default()
    });
    let resolved = root
        .resolve_scope(scope_request(&normalized))
        .map_err(|_| AppError::ScopeForbidden("unreachable".to_string()));
    let safe = safe_arguments(&normalized, &resolved, &root);
    assert_eq!(
        safe.scope,
        ActivityScope::Mine,
        "an administrator may exercise the same isolation semantics as a regular \
         caller, and the record says which scope actually applied"
    );
    assert!(
        safe.requested_scope.is_none(),
        "the requested scope is recorded only when it DIFFERS from the effective \
         one; an identical pair carries no information"
    );
}

#[test]
fn the_effective_scope_projection_matches_the_resolved_scope() {
    let access = policy_with_admins("root");
    let personal = viewer(101, "alice", &access)
        .resolve_scope(ScopeRequest::new(None))
        .expect("resolves");
    let global = viewer(900, "root", &access)
        .resolve_scope(ScopeRequest::new(None))
        .expect("resolves");
    assert!(matches!(effective_scope(&personal), EffectiveScope::Mine));
    assert!(matches!(effective_scope(&global), EffectiveScope::All));
}
