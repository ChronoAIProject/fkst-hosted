//! Unit tests for the sealed activity visibility constraint.

use super::*;
use crate::github_identity::GithubUser;
use crate::session_access::policy::AccessBasis;
use crate::session_access::test_support::policy_with_admins;
use crate::session_access::viewer::{AuthenticatedViewer, RequestedScope, ScopeRequest};

/// Resolve a scope through the only path that exists: a verified viewer.
fn scope(id: i64, login: &str, admins: &str, requested: Option<RequestedScope>) -> ViewerScope {
    let access = policy_with_admins(admins);
    let viewer = AuthenticatedViewer::new(
        GithubUser {
            login: login.to_string(),
            id,
        },
        &access,
    );
    viewer
        .resolve_scope(ScopeRequest::new(requested))
        .expect("scope resolves")
}

fn allowed() -> SessionAccessDecision {
    SessionAccessDecision {
        allowed: true,
        basis: AccessBasis::Creator,
    }
}

fn denied() -> SessionAccessDecision {
    SessionAccessDecision {
        allowed: false,
        basis: AccessBasis::None,
    }
}

#[test]
fn a_regular_viewer_can_only_constrain_to_their_own_actor_id() {
    let constraint = ActivityVisibilityConstraint::for_scope(&scope(101, "alice", "", None), None);
    assert_eq!(constraint.required_actor_id(), Some(101));
    assert_eq!(constraint.as_str(), "mine");
    match &constraint {
        ActivityVisibilityConstraint::Mine(personal) => {
            assert_eq!(personal.actor_id(), 101);
            assert_eq!(personal.lifecycle_session_id(), None);
        }
        other => panic!("expected personal constraint, got {other:?}"),
    }
}

#[test]
fn only_an_administrator_scope_yields_the_unconstrained_form() {
    let admin = scope(107, "grace", "grace", Some(RequestedScope::Global));
    let constraint = ActivityVisibilityConstraint::for_scope(&admin, None);
    assert_eq!(constraint.as_str(), "all");
    assert_eq!(
        constraint.required_actor_id(),
        None,
        "the global scope is the only form without an actor predicate"
    );
    match &constraint {
        ActivityVisibilityConstraint::All(global) => assert_eq!(global.admin_id(), 107),
        other => panic!("expected global constraint, got {other:?}"),
    }
}

#[test]
fn an_authorized_session_adds_lifecycle_rows_but_never_removes_the_actor_predicate() {
    let session = authorize_lifecycle_session("sess-shared", &allowed())
        .expect("an allowing decision mints the token");
    let constraint =
        ActivityVisibilityConstraint::for_scope(&scope(101, "alice", "", None), Some(session));
    assert_eq!(
        constraint.required_actor_id(),
        Some(101),
        "sharing a session must never surface another human's API rows"
    );
    match &constraint {
        ActivityVisibilityConstraint::Mine(personal) => {
            assert_eq!(personal.lifecycle_session_id(), Some("sess-shared"));
        }
        other => panic!("expected personal constraint, got {other:?}"),
    }
}

#[test]
fn a_denied_session_cannot_be_turned_into_an_authorized_id() {
    assert_eq!(
        authorize_lifecycle_session("sess-forbidden", &denied()),
        None,
        "ignoring the verdict must not be a way to obtain the token"
    );
}

#[test]
fn a_lifecycle_token_is_ignored_in_global_scope() {
    let session = authorize_lifecycle_session("sess-any", &allowed()).expect("minted");
    let admin = scope(107, "grace", "grace", Some(RequestedScope::Global));
    let constraint = ActivityVisibilityConstraint::for_scope(&admin, Some(session));
    assert!(matches!(constraint, ActivityVisibilityConstraint::All(_)));
    assert_eq!(constraint.required_actor_id(), None);
}

#[test]
fn an_administrator_selecting_personal_scope_gets_the_personal_predicate() {
    let admin = scope(107, "grace", "grace", Some(RequestedScope::Personal));
    let constraint = ActivityVisibilityConstraint::for_scope(&admin, None);
    assert_eq!(
        constraint.required_actor_id(),
        Some(107),
        "an admin may exercise the same personal semantics"
    );
}

#[test]
fn unattributed_rows_are_excluded_from_every_personal_constraint() {
    // Structural: a personal constraint ALWAYS carries a required actor id, so a
    // row without one can never satisfy it. This asserts the invariant holds for
    // both the bare and the session-scoped personal forms.
    let bare = ActivityVisibilityConstraint::for_scope(&scope(101, "alice", "", None), None);
    let with_session = ActivityVisibilityConstraint::for_scope(
        &scope(101, "alice", "", None),
        authorize_lifecycle_session("sess-shared", &allowed()),
    );
    for constraint in [bare, with_session] {
        assert!(
            constraint.required_actor_id().is_some(),
            "ownership must always be provable in personal scope"
        );
    }
}
