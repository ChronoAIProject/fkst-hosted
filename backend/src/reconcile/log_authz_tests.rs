//! Exhaustive matrix for the pure [`super::is_authorized`]: author, per-issue
//! allow-list, and global-admin tiers; id-vs-login matching; and deny-by-default.

use super::is_authorized;

/// The issue author's numeric id, reused across the matrix.
const AUTHOR_ID: i64 = 1001;

fn s(values: &[&str]) -> Vec<String> {
    values.iter().map(|v| v.to_string()).collect()
}

// ---- Tier 1: the issue author ------------------------------------------------

#[test]
fn author_is_authorized_by_numeric_id() {
    // The author is matched by id even with an empty allow-list + no admins, and
    // even if their login differs from anything listed.
    assert!(is_authorized(AUTHOR_ID, "alice", AUTHOR_ID, &[], &[]));
}

#[test]
fn author_match_ignores_login_and_uses_id_only() {
    // Same login as the author but a DIFFERENT id must NOT be treated as the author
    // (logins are renamable; only the id is the identity key).
    assert!(!is_authorized(2002, "alice", AUTHOR_ID, &[], &[]));
}

// ---- Tier 2: the per-issue allow-list ----------------------------------------

#[test]
fn per_issue_login_grants_case_insensitively() {
    assert!(is_authorized(2002, "Bob", AUTHOR_ID, &s(&["bob"]), &[]));
    assert!(is_authorized(2002, "bob", AUTHOR_ID, &s(&["BOB"]), &[]));
}

#[test]
fn per_issue_numeric_id_grants() {
    // An entry that is the caller's numeric id (as a string) grants access.
    assert!(is_authorized(2002, "bob", AUTHOR_ID, &s(&["2002"]), &[]));
}

#[test]
fn per_issue_at_prefixed_login_grants() {
    // A `@`-prefixed entry is equivalent to the bare login.
    assert!(is_authorized(2002, "bob", AUTHOR_ID, &s(&["@bob"]), &[]));
}

#[test]
fn per_issue_non_member_is_denied() {
    // A caller who matches neither by id nor login is denied.
    assert!(!is_authorized(
        2002,
        "carol",
        AUTHOR_ID,
        &s(&["bob", "9999"]),
        &[]
    ));
}

// ---- Tier 3: the global admins -----------------------------------------------

#[test]
fn global_admin_login_grants_for_any_session() {
    assert!(is_authorized(3003, "Ops", AUTHOR_ID, &[], &s(&["ops"])));
}

#[test]
fn global_admin_numeric_id_grants() {
    assert!(is_authorized(3003, "ops", AUTHOR_ID, &[], &s(&["3003"])));
}

#[test]
fn admin_grants_even_when_per_issue_list_excludes_the_caller() {
    // The three tiers are OR-ed: an admin is authorized regardless of the per-issue
    // list, which names someone else entirely.
    assert!(is_authorized(
        3003,
        "ops",
        AUTHOR_ID,
        &s(&["bob"]),
        &s(&["ops"])
    ));
}

// ---- Deny by default ---------------------------------------------------------

#[test]
fn non_author_non_listed_non_admin_is_denied() {
    assert!(!is_authorized(
        4004,
        "mallory",
        AUTHOR_ID,
        &s(&["bob"]),
        &s(&["ops"])
    ));
}

#[test]
fn empty_and_whitespace_entries_never_match() {
    // Blank/junk allow-list tokens must never grant access, even to a caller whose
    // login/id would trivially "equal" an empty string under a buggy comparison.
    assert!(!is_authorized(
        0,
        "",
        AUTHOR_ID,
        &s(&["", "   ", "@"]),
        &s(&["", " "])
    ));
}

#[test]
fn a_numeric_entry_addresses_the_id_namespace_only() {
    // A numeric entry ("42") addresses the ID namespace ONLY: it must NOT admit a
    // caller merely because their LOGIN is the same digits. All-numeric GitHub
    // usernames exist, so conflating the two would let someone register the login
    // "42" and impersonate the identity an operator listed by id 42.
    assert!(!is_authorized(2002, "42", AUTHOR_ID, &s(&["42"]), &[]));
    // It DOES admit the caller whose numeric id equals the entry.
    assert!(is_authorized(42, "anything", AUTHOR_ID, &s(&["42"]), &[]));
    // A caller matching neither the id nor a login entry is denied.
    assert!(!is_authorized(2002, "43", AUTHOR_ID, &s(&["42"]), &[]));
}
