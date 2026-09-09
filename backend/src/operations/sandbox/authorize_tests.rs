//! Unit tests for per-row authorization: the full fixture matrix, plus the three
//! ways a row is hidden and the one way the decision fails outright.

use super::super::test_support::item;
use super::*;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::session_access::test_support::{context, policy_with_admins};
use crate::session_access::ScopeRequest;

/// Alice created the session; Bob collaborates; Carol holds the per-session log
/// grant; Dana is a deployment `FKST_LOG_ADMINS` entry; Erin is unrelated; Frank
/// is the repository owner (deliberately not a tier); Grace is a global admin.
const ALICE: (i64, &str) = (101, "alice");
const BOB: (i64, &str) = (102, "bob");
const CAROL: (i64, &str) = (103, "carol");
const DANA: (i64, &str) = (104, "dana");
const ERIN: (i64, &str) = (105, "erin");
const FRANK: (i64, &str) = (106, "acme");
const GRACE: (i64, &str) = (900, "grace");

const SESSION: &str = "sess-alice";

fn viewer(who: (i64, &str), access: &AccessPolicy) -> AuthenticatedViewer {
    AuthenticatedViewer::new(
        GithubUser {
            login: who.1.to_string(),
            id: who.0,
        },
        access,
    )
}

fn ready_registry() -> SessionAccessRegistry {
    // `new(false)` = dispatch off, so the projection starts READY: this fixture
    // is about the tiers, not about readiness.
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        vec![(
            SESSION.to_string(),
            context(Some(ALICE.0), ALICE.1, &[BOB.1], &[CAROL.1]),
        )],
    );
    registry
}

/// The scope is always stated EXPLICITLY: omitting it resolves to the caller's
/// natural default, which for an administrator is the global scope — and these
/// fixtures are precisely about what the personal scope does for one.
fn scope(viewer: &AuthenticatedViewer, global: bool) -> ViewerScope {
    let requested = if global {
        crate::session_access::RequestedScope::Global
    } else {
        crate::session_access::RequestedScope::Personal
    };
    viewer
        .resolve_scope(ScopeRequest::new(Some(requested)))
        .expect("the fixture resolves")
}

/// `decide_row` for one caller, in the personal scope, against the ready fixture.
fn visible(who: (i64, &str), session: Option<&str>) -> bool {
    let access = policy_with_admins(GRACE.1);
    let viewer = viewer(who, &access);
    let scope = scope(&viewer, false);
    let registry = ready_registry();
    let admins = [DANA.1.to_string()];
    RowAuthorizer::new(&registry, &viewer, &scope, &access, &admins)
        .decide_row(&item("fkst-sess-alice", session))
        .expect("the projection is ready")
        .is_none()
}

#[test]
fn the_creator_collaborator_log_grantee_and_legacy_log_admin_all_see_the_session() {
    for who in [ALICE, BOB, CAROL, DANA] {
        assert!(visible(who, Some(SESSION)), "{} must see it", who.1);
    }
}

/// Repository ownership is deliberately not a tier, so Frank is refused exactly
/// like the unrelated Erin.
#[test]
fn an_unrelated_user_and_the_repository_owner_are_both_hidden() {
    for who in [ERIN, FRANK] {
        assert!(!visible(who, Some(SESSION)), "{} must not see it", who.1);
    }
}

/// The whole point of `accessible`: a global administrator may deliberately
/// inspect only what they directly own or were granted.
#[test]
fn a_global_admin_in_the_personal_scope_does_not_use_their_bypass() {
    assert!(
        !visible(GRACE, Some(SESSION)),
        "the accessible scope evaluates even an administrator on their DIRECT tiers"
    );
}

#[test]
fn a_global_admin_in_the_global_scope_sees_every_managed_runtime() {
    let access = policy_with_admins(GRACE.1);
    let grace = viewer(GRACE, &access);
    let scope = scope(&grace, true);
    let registry = ready_registry();
    let authorizer = RowAuthorizer::new(&registry, &grace, &scope, &access, &[]);
    for session in [Some(SESSION), Some("sess-unknown"), None] {
        assert!(
            authorizer
                .decide_row(&item("fkst-any", session))
                .expect("global scope needs no projection")
                .is_none(),
            "an administrator must see even the unattributable rows"
        );
    }
}

/// The three ways a regular caller's row is hidden, each with its own bounded
/// reason — none of which ever reaches a response.
#[test]
fn an_orphan_a_malformed_id_and_an_unknown_session_are_hidden_for_their_own_reasons() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let scope = scope(&alice, false);
    let registry = ready_registry();
    let authorizer = RowAuthorizer::new(&registry, &alice, &scope, &access, &[]);
    for (session, expected) in [
        (None, HiddenReason::UnusableSessionId),
        (Some("not a session id"), HiddenReason::UnusableSessionId),
        (Some("sess-somebody-else"), HiddenReason::UnknownContext),
    ] {
        assert_eq!(
            authorizer
                .decide_row(&item("fkst-any", session))
                .expect("the projection is ready"),
            Some(expected)
        );
    }
}

/// A runtime's own annotation claims Alice created it — and Alice really did
/// create SOME session. The claim still grants nothing on a session the registry
/// does not associate with her.
#[test]
fn a_creator_annotation_alone_never_grants_access() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let scope = scope(&alice, false);
    let registry = ready_registry();
    let forged = RuntimeInventoryItem {
        creator_id: Some(ALICE.0),
        creator_login: Some(ALICE.1.to_string()),
        ..item("fkst-forged", Some("sess-foreign"))
    };
    assert_eq!(
        RowAuthorizer::new(&registry, &alice, &scope, &access, &[])
            .decide_row(&forged)
            .expect("the projection is ready"),
        Some(HiddenReason::UnknownContext),
        "runtime metadata is display data; the registry is the only authority"
    );
}

/// A numeric creator id is authoritative: a caller whose LOGIN matches a stale
/// snapshot but whose id differs is a different account.
#[test]
fn a_stale_login_with_a_different_id_does_not_inherit_the_session() {
    let access = policy_with_admins(GRACE.1);
    let impostor = viewer((999, ALICE.1), &access);
    let scope = scope(&impostor, false);
    let registry = ready_registry();
    assert_eq!(
        RowAuthorizer::new(&registry, &impostor, &scope, &access, &[])
            .decide_row(&item("fkst-sess-alice", Some(SESSION)))
            .expect("the projection is ready"),
        Some(HiddenReason::NotAuthorized)
    );
}

/// The login fallback exists only where GitHub gave us no id at all (an
/// assignee-derived creator).
#[test]
fn a_missing_creator_id_falls_back_to_the_login_snapshot() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let scope = scope(&alice, false);
    let registry = SessionAccessRegistry::new(false);
    registry.replace_repo(
        1,
        &RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        },
        vec![(SESSION.to_string(), context(None, ALICE.1, &[], &[]))],
    );
    assert!(RowAuthorizer::new(&registry, &alice, &scope, &access, &[])
        .decide_row(&item("fkst-sess-alice", Some(SESSION)))
        .expect("the projection is ready")
        .is_none());
}

/// A projection that stops answering mid-read is an ERROR, never a silently
/// dropped row: dropping would render as a confident, complete, empty fleet.
#[test]
fn a_cold_projection_fails_the_request_instead_of_hiding_every_row() {
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let scope = scope(&alice, false);
    // `new(true)` = dispatch on, so the projection is COLD until a generation
    // lands.
    let registry = SessionAccessRegistry::new(true);
    let error = RowAuthorizer::new(&registry, &alice, &scope, &access, &[])
        .decide_row(&item("fkst-sess-alice", Some(SESSION)))
        .expect_err("a cold projection cannot answer");
    assert!(matches!(error, AppError::SessionVisibilityUnavailable(_)));
}

/// The tally is what an operator actually reads, so it must count each reason
/// independently and label each with its own distinct wire value.
#[test]
fn the_hidden_tally_counts_each_reason_under_its_own_distinct_label() {
    let mut tally = HiddenTally::default();
    tally.record(HiddenReason::UnknownContext);
    tally.record(HiddenReason::UnknownContext);
    tally.record(HiddenReason::NotAuthorized);

    assert_eq!(tally.count(HiddenReason::UnknownContext), 2);
    assert_eq!(tally.count(HiddenReason::NotAuthorized), 1);
    assert_eq!(
        tally.count(HiddenReason::UnusableSessionId),
        0,
        "a reason nothing was withheld for must stay silent"
    );

    let labels: Vec<&str> = HiddenReason::ALL.iter().map(|r| r.as_str()).collect();
    let mut unique = labels.clone();
    unique.sort_unstable();
    unique.dedup();
    assert_eq!(unique.len(), labels.len(), "{labels:?}");
    // `trace` is the only production consumer; exercising it keeps the emission
    // path compiled and proves it is total over the closed reason set.
    tally.trace();
}

/// The counting must be driven by real decisions, not only by hand-recorded
/// values — otherwise the tally could count the wrong gate and still pass.
#[test]
fn a_mixed_fleet_tallies_the_gate_that_actually_withheld_each_row() {
    const THEIRS: &str = "sess-stranger";
    let access = policy_with_admins(GRACE.1);
    let alice = viewer(ALICE, &access);
    let scope = scope(&alice, false);
    // A projection that knows Alice's session AND a stranger's, so "the registry
    // never heard of it" and "the tiers said no" are two distinct rows.
    let registry = ready_registry();
    registry.replace_repo(
        2,
        &RepoRef {
            owner: "acme".to_string(),
            name: "other".to_string(),
        },
        vec![(THEIRS.to_string(), context(Some(707), "stranger", &[], &[]))],
    );
    let authorizer = RowAuthorizer::new(&registry, &alice, &scope, &access, &[]);

    let mut tally = HiddenTally::default();
    for row in [
        item("fkst-orphan", None),
        item("fkst-foreign", Some("sess-foreign")),
        item("fkst-theirs", Some(THEIRS)),
        item("fkst-mine", Some(SESSION)),
    ] {
        if let Some(reason) = authorizer
            .decide_row(&row)
            .expect("the projection is ready")
        {
            tally.record(reason);
        }
    }
    assert_eq!(tally.count(HiddenReason::UnusableSessionId), 1);
    assert_eq!(tally.count(HiddenReason::UnknownContext), 1);
    assert_eq!(tally.count(HiddenReason::NotAuthorized), 1);
}
