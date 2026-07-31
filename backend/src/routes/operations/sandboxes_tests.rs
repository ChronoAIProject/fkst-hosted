//! Handler-level unit tests for the pure decisions the route makes before any
//! backend call: what a refused probe records, and what the effective scope is.
//!
//! The end-to-end authorization matrix (identity, `403`, `404`, `503`, isolation,
//! ordering, headers) lives in `tests/operations_sandboxes*.rs`, which drives the
//! real router.

use super::*;
use crate::access_policy::AccessPolicy;
use crate::github_identity::GithubUser;
use crate::routes::operations::sandbox_query::{normalize, SandboxQueryParams};
use crate::session_access::test_support::policy_with_admins;

fn viewer(id: i64, login: &str, access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: login.to_string(),
            id,
        },
        access,
    )
}

fn request(params: SandboxQueryParams) -> NormalizedSandboxRequest {
    normalize(&params).expect("the fixture normalizes")
}

#[test]
fn a_regular_callers_natural_scope_is_accessible_and_an_admins_is_global() {
    let access = policy_with_admins("grace");
    assert_eq!(
        natural_scope(&viewer(101, "alice", &access)),
        ScopeLabel::Accessible
    );
    assert_eq!(
        natural_scope(&viewer(900, "grace", &access)),
        ScopeLabel::All
    );
}

#[test]
fn an_allowed_read_records_its_effective_scope_and_normalized_filters() {
    let access = policy_with_admins("");
    let alice = viewer(101, "alice", &access);
    let normalized = request(SandboxQueryParams {
        session_id: Some("sess-alice".to_string()),
        repo_full_name: Some("acme/site".to_string()),
        trigger_issue: Some(7),
        status: Some("running".to_string()),
        backend: Some("kubernetes".to_string()),
        creator_id: Some(101),
        creator_login: Some("@Alice".to_string()),
        attribution_source: Some("launch_metadata".to_string()),
        ..SandboxQueryParams::default()
    });
    let resolved = alice
        .resolve_scope(ScopeRequest::new(normalized.requested_scope))
        .map_err(|_| AppError::ScopeForbidden("unreachable in this fixture".to_string()));
    let safe = safe_arguments(&normalized, &resolved, &alice);

    assert_eq!(safe.scope, SandboxScope::Accessible);
    assert!(
        safe.requested_scope.is_none(),
        "an unstated scope adds nothing to the record"
    );
    assert_eq!(safe.session_id.as_deref(), Some("sess-alice"));
    assert_eq!(safe.repo_full_name.as_deref(), Some("acme/site"));
    assert_eq!(safe.trigger_issue, Some(7));
    assert_eq!(safe.status.as_deref(), Some("running"));
    assert_eq!(safe.backend.as_deref(), Some("kubernetes"));
    assert_eq!(safe.creator_id, Some(101));
    assert_eq!(safe.creator_login.as_deref(), Some("Alice"));
    assert_eq!(safe.attribution_source.as_deref(), Some("launch_metadata"));
}

/// A denied scope probe records the ATTEMPT, never anything about the deployment
/// it was probing.
#[test]
fn a_denied_scope_probe_records_both_scopes_and_nothing_else() {
    let access = policy_with_admins("grace");
    let alice = viewer(101, "alice", &access);
    let normalized = request(SandboxQueryParams {
        scope: Some("all".to_string()),
        ..SandboxQueryParams::default()
    });
    let resolved = alice
        .resolve_scope(ScopeRequest::new(normalized.requested_scope))
        .map_err(|_| AppError::ScopeForbidden("refused".to_string()));
    assert!(resolved.is_err(), "a regular caller may not do this");

    let safe = safe_arguments(&normalized, &resolved, &alice);
    assert_eq!(
        safe.scope,
        SandboxScope::Accessible,
        "the record states the caller's own scope, which is what actually applied"
    );
    assert_eq!(
        safe.requested_scope,
        Some(SandboxScope::All),
        "and the scope they asked for, which is what makes the denial legible"
    );
    let rendered = serde_json::to_string(&safe).expect("serializes");
    assert!(!rendered.contains("grace"), "{rendered}");
}

#[test]
fn an_administrator_selecting_the_accessible_scope_records_both_scopes() {
    let access = policy_with_admins("grace");
    let grace = viewer(900, "grace", &access);
    let normalized = request(SandboxQueryParams {
        scope: Some("accessible".to_string()),
        ..SandboxQueryParams::default()
    });
    let resolved = grace
        .resolve_scope(ScopeRequest::new(normalized.requested_scope))
        .map_err(|_| AppError::ScopeForbidden("unreachable".to_string()));
    let safe = safe_arguments(&normalized, &resolved, &grace);
    assert_eq!(
        safe.scope,
        SandboxScope::Accessible,
        "an administrator may exercise the same isolation semantics as a regular \
         caller, and the record says which scope actually applied"
    );
    assert!(
        safe.requested_scope.is_none(),
        "the requested scope is recorded only when it DIFFERS from the effective \
         one; an identical pair carries no information"
    );
}

/// Every failure maps to exactly one bounded result label, so a counter and an
/// HTTP response can never disagree about what happened.
#[test]
fn every_inventory_failure_maps_onto_its_own_bounded_result() {
    for (error, expected) in [
        (
            AppError::SandboxInventoryTooLarge("x".to_string()),
            InventoryResult::TooLarge,
        ),
        (
            AppError::SessionVisibilityUnavailable("x".to_string()),
            InventoryResult::VisibilityUnavailable,
        ),
        (
            AppError::SandboxInventoryUnavailable("x".to_string()),
            InventoryResult::Unavailable,
        ),
    ] {
        assert_eq!(result_of(&error), expected);
    }
}
