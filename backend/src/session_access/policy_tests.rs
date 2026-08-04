//! The capability matrix.
//!
//! Cast, fixed for every test so the table below reads like the specification:
//!
//! | who | id | relationship |
//! |---|---|---|
//! | Alice | 101 | effective creator |
//! | Bob | 102 | `### Session Collaborators` |
//! | Carol | 103 | `### FKST Contributors` / log-access allow-list |
//! | Dana | 104 | legacy `FKST_LOG_ADMINS` |
//! | Erin | 105 | unrelated |
//! | Frank | 106 | repository administrator, and nothing else |
//! | Grace | 107 | deployment `FKST_GLOBAL_ADMINS` |

use super::*;
use crate::session_access::context::SessionAccessContext;
use crate::session_access::test_support::{context, denylist, policy_with_admins};

const ALICE: (i64, &str) = (101, "alice");
const BOB: (i64, &str) = (102, "bob");
const CAROL: (i64, &str) = (103, "carol");
const DANA: (i64, &str) = (104, "dana");
const ERIN: (i64, &str) = (105, "erin");
/// Frank is a repository administrator. The pure policy cannot see that, which
/// is exactly the point: repository role is never a tier.
const FRANK: (i64, &str) = (106, "frank");
const GRACE: (i64, &str) = (107, "grace");

fn cast_context() -> SessionAccessContext {
    context(Some(ALICE.0), ALICE.1, &[BOB.1], &[CAROL.1])
}

fn legacy_admins() -> Vec<String> {
    vec![DANA.1.to_string()]
}

fn caller(who: (i64, &str)) -> VerifiedCaller<'_> {
    VerifiedCaller::from_github_metadata(who.0, who.1)
}

/// Decide `capability` for `who` against the cast fixture.
fn verdict(
    capability: SessionCapability,
    who: (i64, &str),
    access: &AccessPolicy,
    legacy: &[String],
) -> SessionAccessDecision {
    let ctx = cast_context();
    decide(&SessionAccessRequest::new(
        capability,
        caller(who),
        ctx.facts(),
        PolicyEnvironment {
            access,
            legacy_log_admins: legacy,
            github_bot_login: Some("fkst-app[bot]"),
        },
    ))
}

#[test]
fn operations_visibility_admits_every_explicit_tier_and_no_one_else() {
    let access = policy_with_admins(GRACE.1);
    let legacy = legacy_admins();
    for (who, basis) in [
        (ALICE, AccessBasis::Creator),
        (BOB, AccessBasis::Collaborator),
        (CAROL, AccessBasis::LogAccess),
        (DANA, AccessBasis::LegacyLogAdmin),
        (GRACE, AccessBasis::GlobalAdmin),
    ] {
        let decision = verdict(
            SessionCapability::OperationsVisibility,
            who,
            &access,
            &legacy,
        );
        assert!(decision.allowed(), "{} must be admitted", who.1);
        assert_eq!(decision.basis(), basis, "{} basis", who.1);
    }
    for who in [ERIN, FRANK] {
        let decision = verdict(
            SessionCapability::OperationsVisibility,
            who,
            &access,
            &legacy,
        );
        assert!(!decision.allowed(), "{} must be rejected", who.1);
        assert_eq!(
            decision.basis(),
            AccessBasis::None,
            "a denial never names a near-miss tier"
        );
    }
}

#[test]
fn repository_administrator_status_never_changes_the_pure_result() {
    // Frank's repository role is invisible to the policy by construction, so the
    // verdict is identical to an entirely unrelated caller's.
    let access = policy_with_admins("");
    let legacy = legacy_admins();
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::LogDownload,
        SessionCapability::Observe,
        SessionCapability::WorkAuthority,
    ] {
        assert_eq!(
            verdict(capability, FRANK, &access, &legacy),
            verdict(capability, ERIN, &access, &legacy),
            "{capability:?}"
        );
    }
}

#[test]
fn a_collaborator_alone_cannot_download_logs_or_observe() {
    let access = policy_with_admins(GRACE.1);
    let legacy = legacy_admins();
    for capability in [SessionCapability::LogDownload, SessionCapability::Observe] {
        assert!(
            !verdict(capability, BOB, &access, &legacy).allowed(),
            "collaborator must not gain {capability:?}"
        );
        // The tiers that DO grant it still do.
        for who in [ALICE, CAROL, DANA, GRACE] {
            assert!(
                verdict(capability, who, &access, &legacy).allowed(),
                "{} must keep {capability:?}",
                who.1
            );
        }
    }
}

#[test]
fn a_log_grantee_or_legacy_log_admin_alone_cannot_raise_work() {
    let access = policy_with_admins(GRACE.1);
    let legacy = legacy_admins();
    for who in [CAROL, DANA] {
        assert!(
            !verdict(SessionCapability::WorkAuthority, who, &access, &legacy).allowed(),
            "{} must not gain work authority",
            who.1
        );
    }
    for who in [ALICE, BOB, GRACE] {
        assert!(
            verdict(SessionCapability::WorkAuthority, who, &access, &legacy).allowed(),
            "{} keeps work authority",
            who.1
        );
    }
}

#[test]
fn the_configured_app_is_a_work_system_principal_and_nothing_more() {
    let access = policy_with_admins("");
    let legacy = legacy_admins();
    let bot = (9000, "fkst-app[bot]");
    let work = verdict(SessionCapability::WorkAuthority, bot, &access, &legacy);
    assert!(work.allowed());
    assert_eq!(work.basis(), AccessBasis::AppSystem);
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::LogDownload,
        SessionCapability::Observe,
    ] {
        assert!(
            !verdict(capability, bot, &access, &legacy).allowed(),
            "the App is not a human observability tier ({capability:?})"
        );
    }
}

#[test]
fn creator_numeric_id_wins_over_a_stale_matching_login() {
    let access = policy_with_admins("");
    let ctx = context(Some(ALICE.0), ALICE.1, &[], &[]);
    let env = PolicyEnvironment {
        access: &access,
        legacy_log_admins: &[],
        github_bot_login: None,
    };
    // A different account that happens to now hold the creator's old login.
    let impostor = VerifiedCaller::from_github_metadata(999, ALICE.1);
    let renamed_creator = VerifiedCaller::from_github_metadata(ALICE.0, "alice-renamed");
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::LogDownload,
        SessionCapability::WorkAuthority,
    ] {
        assert!(
            !decide(&SessionAccessRequest::new(
                capability,
                impostor,
                ctx.facts(),
                env
            ))
            .allowed(),
            "a stale login with a different id must not inherit the session ({capability:?})"
        );
        assert!(
            decide(&SessionAccessRequest::new(
                capability,
                renamed_creator,
                ctx.facts(),
                env
            ))
            .allowed(),
            "the immutable id survives a rename ({capability:?})"
        );
    }
}

#[test]
fn a_missing_creator_id_uses_the_verified_login_fallback_only() {
    let access = policy_with_admins("");
    let ctx = context(None, "Seed-Owner", &[], &[]);
    let env = PolicyEnvironment {
        access: &access,
        legacy_log_admins: &[],
        github_bot_login: None,
    };
    let assignee = VerifiedCaller::from_github_metadata(555, "seed-owner");
    assert!(decide(&SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        assignee,
        ctx.facts(),
        env
    ))
    .allowed());
    let someone_else = VerifiedCaller::from_github_metadata(556, "someone-else");
    assert!(!decide(&SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        someone_else,
        ctx.facts(),
        env
    ))
    .allowed());
}

#[test]
fn a_trigger_author_differing_from_the_creator_gets_no_implicit_tier() {
    // The registry context deliberately carries no trigger author: it cannot be a
    // tier because the policy cannot see it. This asserts the shape stays that
    // way — an id that only ever appeared as the trigger author is a stranger.
    let access = policy_with_admins("");
    let ctx = context(None, "Seed-Owner", &[], &[]);
    let trigger_author = VerifiedCaller::from_github_metadata(9000, "fkst-app[bot]");
    let decision = decide(&SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        trigger_author,
        ctx.facts(),
        PolicyEnvironment {
            access: &access,
            legacy_log_admins: &[],
            github_bot_login: Some("fkst-app[bot]"),
        },
    ));
    assert!(!decision.allowed());
}

#[test]
fn a_blocked_ordinary_user_loses_every_capability() {
    // "Blocked users lose every gate" is the deployment contract, and the base
    // AccessPolicy is consulted before ANY session tier — including log download
    // and observe, whose routes resolve identity outside the GithubUser extractor.
    let access = denylist(BOB.1, "");
    let legacy = legacy_admins();
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::WorkAuthority,
        SessionCapability::LogDownload,
        SessionCapability::Observe,
    ] {
        let decision = verdict(capability, BOB, &access, &legacy);
        assert!(!decision.allowed(), "{capability:?}");
        assert_eq!(decision.basis(), AccessBasis::None);
    }

    // The tier itself is irrelevant: a blocked LOG-ACCESS grantee and a blocked
    // LEGACY log admin lose the bundle too, not only a blocked collaborator.
    for who in [CAROL, DANA] {
        let blocked = denylist(who.1, "");
        assert!(
            !verdict(SessionCapability::LogDownload, who, &blocked, &legacy).allowed(),
            "{} must not keep the log bundle while blocked",
            who.1
        );
        // ...and the same identity keeps it once the deployment admits them again.
        assert!(verdict(
            SessionCapability::LogDownload,
            who,
            &policy_with_admins(""),
            &legacy
        )
        .allowed());
    }
}

#[test]
fn global_admin_precedence_over_the_blocklist_is_preserved() {
    // A global admin also (mis)listed as blocked stays authorized — the existing
    // AccessPolicy precedence, unchanged by this milestone.
    let access = denylist(GRACE.1, GRACE.1);
    let legacy = legacy_admins();
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::WorkAuthority,
        SessionCapability::LogDownload,
    ] {
        assert!(
            verdict(capability, GRACE, &access, &legacy).allowed(),
            "{capability:?}"
        );
    }
}

#[test]
fn the_accessible_scope_evaluates_a_global_admin_on_direct_tiers_only() {
    let access = policy_with_admins(GRACE.1);
    let legacy = legacy_admins();
    let ctx = cast_context();
    let request = SessionAccessRequest::new(
        SessionCapability::OperationsVisibility,
        caller(GRACE),
        ctx.facts(),
        PolicyEnvironment {
            access: &access,
            legacy_log_admins: &legacy,
            github_bot_login: None,
        },
    )
    .without_global_admin();
    assert!(
        !decide(&request).allowed(),
        "scope=accessible shows what the admin directly owns or was granted"
    );
    // Alice, who IS the creator, is unaffected by the bypass being disabled.
    let mut alice = request;
    alice.caller = caller(ALICE);
    assert!(decide(&alice).allowed());
}

#[test]
fn blank_and_malformed_list_entries_never_match() {
    let access = policy_with_admins("");
    let ctx = context(Some(ALICE.0), ALICE.1, &["@", "  ", ""], &["@", " "]);
    let env = PolicyEnvironment {
        access: &access,
        legacy_log_admins: &[],
        github_bot_login: None,
    };
    for login in ["", " ", "@"] {
        let decision = decide(&SessionAccessRequest::new(
            SessionCapability::OperationsVisibility,
            VerifiedCaller::from_github_metadata(999, login),
            ctx.facts(),
            env,
        ));
        assert!(
            !decision.allowed(),
            "login {login:?} must not match a blank entry"
        );
    }
}

#[test]
fn list_entries_match_by_numeric_id_or_case_insensitive_login() {
    let access = policy_with_admins("");
    let ctx = context(Some(ALICE.0), ALICE.1, &["@Bob", "31337"], &["CAROL"]);
    let env = PolicyEnvironment {
        access: &access,
        legacy_log_admins: &[],
        github_bot_login: None,
    };
    let allowed = |id: i64, login: &str| {
        decide(&SessionAccessRequest::new(
            SessionCapability::OperationsVisibility,
            VerifiedCaller::from_github_metadata(id, login),
            ctx.facts(),
            env,
        ))
        .allowed()
    };
    assert!(allowed(BOB.0, "bob"), "leading @ and casing are ignored");
    assert!(allowed(31337, "renamed"), "numeric entry matches the id");
    assert!(allowed(CAROL.0, "carol"), "log-access entry, case folded");
    assert!(
        !allowed(BOB.0, "31337"),
        "a numeric entry must not match a login that looks numeric"
    );
}

#[test]
fn every_capability_denies_a_stranger_with_an_empty_environment() {
    let access = policy_with_admins("");
    for capability in [
        SessionCapability::OperationsVisibility,
        SessionCapability::LogDownload,
        SessionCapability::Observe,
        SessionCapability::WorkAuthority,
    ] {
        let decision = verdict(capability, ERIN, &access, &[]);
        assert!(!decision.allowed(), "{capability:?}");
        assert_eq!(decision.basis(), AccessBasis::None);
    }
}

#[test]
fn capability_and_basis_labels_are_stable_closed_enums() {
    assert_eq!(
        SessionCapability::OperationsVisibility.as_str(),
        "operations_visibility"
    );
    assert_eq!(SessionCapability::LogDownload.as_str(), "log_download");
    assert_eq!(SessionCapability::Observe.as_str(), "observe");
    assert_eq!(SessionCapability::WorkAuthority.as_str(), "work_authority");
    assert_eq!(AccessBasis::Creator.as_str(), "creator");
    assert_eq!(AccessBasis::LegacyLogAdmin.as_str(), "legacy_log_admin");
    assert_eq!(AccessBasis::AppSystem.as_str(), "app_system");
    assert_eq!(AccessBasis::None.as_str(), "none");
}
