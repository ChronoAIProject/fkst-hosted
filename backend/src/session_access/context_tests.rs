//! Unit tests for the session authorization context and its borrowed view.

use super::*;
use crate::session_access::test_support::context;

#[test]
fn facts_borrow_the_context_without_copying_the_lists() {
    let ctx = context(Some(42), "alice", &["bob"], &["carol"]);
    let facts = ctx.facts();
    assert_eq!(facts.creator_id, Some(42));
    assert_eq!(facts.creator_login, "alice");
    assert_eq!(facts.collaborators, ["bob".to_string()]);
    assert_eq!(facts.log_access, ["carol".to_string()]);
}

#[test]
fn creator_matching_is_id_authoritative_when_an_id_exists() {
    let ctx = context(Some(583231), "old-login", &[], &[]);
    let facts = ctx.facts();
    assert!(
        facts.creator_matches(583231, "renamed-since"),
        "the immutable id still identifies the creator after a rename"
    );
    assert!(
        !facts.creator_matches(999, "old-login"),
        "a stale matching login with a different id must not inherit the session"
    );
}

#[test]
fn creator_matching_falls_back_to_login_only_without_an_id() {
    let ctx = context(None, "Seed-Owner", &[], &[]);
    let facts = ctx.facts();
    assert!(
        facts.creator_matches(999, "seed-owner"),
        "ASCII-case-folded"
    );
    assert!(!facts.creator_matches(999, "someone-else"));
}

#[test]
fn a_blank_assignee_derived_creator_login_never_matches() {
    for blank in ["", "   "] {
        let ctx = context(None, blank, &[], &[]);
        assert!(
            !ctx.facts().creator_matches(999, ""),
            "a blank creator login must not authorize a blank caller login"
        );
    }
}

#[test]
fn belongs_to_discriminates_installation_and_repository() {
    let ctx = context(Some(1), "alice", &[], &[]);
    assert!(ctx.belongs_to(ctx.installation_id, &ctx.repo));
    assert!(!ctx.belongs_to(ctx.installation_id + 1, &ctx.repo));
    let other = RepoRef {
        owner: ctx.repo.owner.clone(),
        name: "other".to_string(),
    };
    assert!(!ctx.belongs_to(ctx.installation_id, &other));
}
