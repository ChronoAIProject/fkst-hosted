//! Unit tests for the reusable session-visibility gate: readiness, the
//! anti-enumeration `404`, and the accessible-vs-all scope difference.

use super::*;
use crate::github_identity::GithubUser;
use crate::session_access::registry::SessionAccessRegistry;
use crate::session_access::test_support::{context, policy_with_admins, repo};
use crate::session_access::viewer::{RequestedScope, ScopeRequest};

const ALICE: (i64, &str) = (101, "alice");
const ERIN: (i64, &str) = (105, "erin");
const GRACE: (i64, &str) = (107, "grace");

fn viewer(who: (i64, &str), access: &crate::access_policy::AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: who.1.to_string(),
            id: who.0,
        },
        access,
    )
}

fn scope(viewer: &AuthenticatedViewer, requested: RequestedScope) -> ViewerScope {
    viewer
        .resolve_scope(ScopeRequest::new(Some(requested)))
        .expect("scope resolves")
}

/// A ready registry holding one session created by Alice.
fn ready_registry() -> SessionAccessRegistry {
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &repo("site"),
        vec![(
            "sess-alice".to_string(),
            context(Some(ALICE.0), ALICE.1, &[], &[]),
        )],
    );
    registry
}

#[test]
fn the_creator_is_authorized_in_accessible_scope() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let decision = authorize_session_visibility(
        &ready_registry(),
        &alice,
        &scope(&alice, RequestedScope::Personal),
        &access,
        &[],
        "sess-alice",
    )
    .expect("the creator sees their own session");
    assert!(decision.allowed);
}

#[test]
fn an_unauthorized_exact_session_is_indistinguishable_from_an_unknown_one() {
    let access = policy_with_admins(GRACE.1);
    let erin = viewer(ERIN, &access);
    let personal = scope(&erin, RequestedScope::Personal);
    let registry = ready_registry();

    let unauthorized =
        authorize_session_visibility(&registry, &erin, &personal, &access, &[], "sess-alice")
            .expect_err("a stranger must not see the session");
    let unknown = authorize_session_visibility(
        &registry,
        &erin,
        &personal,
        &access,
        &[],
        "sess-does-not-exist",
    )
    .expect_err("an unknown session is not found");
    assert!(matches!(unauthorized, AppError::NotFound(_)));
    assert!(matches!(unknown, AppError::NotFound(_)));
    assert_eq!(
        format!("{unauthorized}"),
        format!("{unknown}"),
        "the two must be byte-identical or the endpoint becomes an existence oracle"
    );
}

#[test]
fn a_cold_projection_is_unavailable_not_a_false_not_found() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let cold = SessionAccessRegistry::new(true);
    let err = authorize_session_visibility(
        &cold,
        &alice,
        &scope(&alice, RequestedScope::Personal),
        &access,
        &[],
        "sess-alice",
    )
    .expect_err("nothing can be concluded from a cold projection");
    assert!(
        matches!(err, AppError::SessionVisibilityUnavailable(_)),
        "{err:?}"
    );
}

#[test]
fn the_global_scope_is_the_explicit_administrator_bypass() {
    let access = policy_with_admins(GRACE.1);
    let grace = viewer(GRACE, &access);
    let registry = ready_registry();

    let accessible = authorize_session_visibility(
        &registry,
        &grace,
        &scope(&grace, RequestedScope::Personal),
        &access,
        &[],
        "sess-alice",
    );
    assert!(
        accessible.is_err(),
        "scope=accessible shows an admin what they directly own or were granted"
    );

    let all = authorize_session_visibility(
        &registry,
        &grace,
        &scope(&grace, RequestedScope::Global),
        &access,
        &[],
        "sess-alice",
    )
    .expect("scope=all is the explicit bypass");
    assert!(all.allowed);
}

#[test]
fn a_legacy_log_admin_reaches_the_session_in_accessible_scope() {
    let access = policy_with_admins(GRACE.1);
    let dana = viewer((104, "dana"), &access);
    let decision = authorize_session_visibility(
        &ready_registry(),
        &dana,
        &scope(&dana, RequestedScope::Personal),
        &access,
        &["dana".to_string()],
        "sess-alice",
    )
    .expect("FKST_LOG_ADMINS keeps its explicit cross-session grant");
    assert!(decision.allowed);
}
