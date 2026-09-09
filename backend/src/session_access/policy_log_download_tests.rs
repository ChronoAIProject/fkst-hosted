//! The log-download matrix, asserted against the capability policy directly.
//!
//! These cases used to live beside a thin `reconcile::log_authz::is_authorized`
//! wrapper. The wrapper is gone (the route decides through
//! [`super::decide`]), but the matrix it guarded is exactly the shipped
//! `/api/v1/logs/{session_id}` contract, so it is asserted here instead of being
//! lost with the shim: creator tier, per-issue allow-list, legacy log admins,
//! id-vs-login matching, and deny-by-default.

use super::*;
use crate::session_access::test_support::{context, policy_with_admins};

/// The effective creator's numeric id, reused across the matrix.
const CREATOR_ID: i64 = 1001;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

/// Decide `LogDownload` for a caller against a session created by `alice`
/// (id [`CREATOR_ID`]) with no collaborators, under an open deployment policy.
fn authorized(
    caller_id: i64,
    caller_login: &str,
    per_issue_allow: &[String],
    legacy_log_admins: &[String],
) -> bool {
    let allow: Vec<&str> = per_issue_allow.iter().map(String::as_str).collect();
    let ctx = context(Some(CREATOR_ID), "alice", &[], &allow);
    let access = policy_with_admins("");
    decide(&SessionAccessRequest::new(
        SessionCapability::LogDownload,
        VerifiedCaller::from_github_metadata(caller_id, caller_login),
        ctx.facts(),
        PolicyEnvironment {
            access: &access,
            legacy_log_admins,
            github_bot_login: None,
        },
    ))
    .allowed()
}

// ---- Tier 1: the effective creator ------------------------------------------

#[test]
fn the_creator_is_authorized_by_numeric_id() {
    assert!(authorized(CREATOR_ID, "alice", &[], &[]));
}

#[test]
fn the_creator_match_ignores_login_and_uses_id_only() {
    // The same login on a DIFFERENT account must not inherit the session: logins
    // are renamable, only the id is the identity key.
    assert!(!authorized(2002, "alice", &[], &[]));
}

#[test]
fn an_assignee_derived_creator_matches_by_login_only() {
    let ctx = context(None, "Seed-Owner", &[], &[]);
    let access = policy_with_admins("");
    let decide_for = |id: i64, login: &str| {
        decide(&SessionAccessRequest::new(
            SessionCapability::LogDownload,
            VerifiedCaller::from_github_metadata(id, login),
            ctx.facts(),
            PolicyEnvironment {
                access: &access,
                legacy_log_admins: &[],
                github_bot_login: None,
            },
        ))
        .allowed()
    };
    assert!(decide_for(999, "seed-owner"), "ASCII case folded");
    assert!(!decide_for(CREATOR_ID, "app-bot"));
}

// ---- Tier 2: the per-issue allow-list ----------------------------------------

#[test]
fn a_per_issue_login_grants_case_insensitively() {
    assert!(authorized(2002, "Bob", &s(&["bob"]), &[]));
    assert!(authorized(2002, "bob", &s(&["BOB"]), &[]));
}

#[test]
fn a_per_issue_numeric_id_or_at_prefixed_login_grants() {
    assert!(authorized(2002, "bob", &s(&["2002"]), &[]));
    assert!(authorized(2002, "bob", &s(&["@bob"]), &[]));
}

#[test]
fn a_per_issue_non_member_is_denied() {
    assert!(!authorized(2002, "carol", &s(&["bob", "9999"]), &[]));
}

// ---- Tier 3: the legacy log admins -------------------------------------------

#[test]
fn a_legacy_log_admin_grants_for_any_session() {
    assert!(authorized(3003, "Ops", &[], &s(&["ops"])));
    assert!(authorized(3003, "ops", &[], &s(&["3003"])));
    // The tiers are OR-ed: the per-issue list naming someone else is irrelevant.
    assert!(authorized(3003, "ops", &s(&["bob"]), &s(&["ops"])));
}

// ---- Deny by default ---------------------------------------------------------

#[test]
fn a_stranger_is_denied() {
    assert!(!authorized(4004, "mallory", &s(&["bob"]), &s(&["ops"])));
}

#[test]
fn empty_and_whitespace_entries_never_match() {
    // Blank/junk tokens must never grant access, even to a caller whose login/id
    // would trivially "equal" an empty string under a buggy comparison.
    assert!(!authorized(0, "", &s(&["", "   ", "@"]), &s(&["", " "])));
}

#[test]
fn a_numeric_entry_addresses_the_id_namespace_only() {
    // All-numeric GitHub usernames exist, so conflating the two namespaces would
    // let someone register the login "42" and impersonate the identity an operator
    // listed by id 42.
    assert!(!authorized(2002, "42", &s(&["42"]), &[]));
    assert!(authorized(42, "anything", &s(&["42"]), &[]));
    assert!(!authorized(2002, "43", &s(&["42"]), &[]));
}
