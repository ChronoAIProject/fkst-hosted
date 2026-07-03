//! Pure three-tier authorization for on-demand session-log downloads.
//!
//! A session's redacted log bundle at chrono-storage key `logs/<session_id>/latest.tar.gz`
//! may be pulled only by someone the session's trigger issue authorizes. This module
//! is the single, PURE decision function the identity-gated download endpoint
//! ([`crate::routes::logs`]) calls once it has resolved the *verified* caller
//! identity (numeric GitHub id + login) — it holds no I/O and no clock, so the whole
//! allow/deny matrix is exhaustively unit-testable.
//!
//! The three tiers, ANY of which grants access (deny by default otherwise):
//!
//! 1. **Author** — the caller is the trigger issue's author (matched by immutable
//!    numeric id, never by a renamable login).
//! 2. **Per-issue allow-list** — the caller matches an entry the author listed in the
//!    trigger issue's `### Log Access Allowlist` section (frozen by config-immutability).
//! 3. **Global admins** — the caller matches an operator-configured entry
//!    (`FKST_LOG_ADMINS`) that may pull ANY session's logs.
//!
//! A list entry (tiers 2 and 3) is matched against BOTH the caller's numeric id (as a
//! decimal string) AND the caller's login (case-insensitively), so an operator or
//! author may list either form. A leading `@` on an entry is ignored so `@alice` and
//! `alice` are equivalent. An empty/whitespace entry never matches anything.

/// Decide whether the verified caller `(requester_id, requester_login)` may download
/// the logs of a session whose trigger issue was opened by `issue_author_id` and
/// whose `### Log Access Allowlist` allow-list is `per_issue_allow`, given the operator's
/// `global_admins`.
///
/// Returns `true` iff the caller satisfies AT LEAST ONE of the three tiers described
/// in the module docs; `false` (deny) otherwise. Purely a function of its inputs.
pub fn is_authorized(
    requester_id: i64,
    requester_login: &str,
    issue_author_id: i64,
    per_issue_allow: &[String],
    global_admins: &[String],
) -> bool {
    // Tier 1: the issue author, matched by IMMUTABLE numeric id (a login is
    // renamable, so it must never be the identity key for the author check).
    if requester_id == issue_author_id {
        return true;
    }
    // Tiers 2 + 3: any per-issue OR any global-admin entry that matches the caller by
    // numeric id (as a string) OR by case-insensitive login.
    let requester_id_str = requester_id.to_string();
    per_issue_allow
        .iter()
        .chain(global_admins.iter())
        .any(|entry| entry_matches(entry, &requester_id_str, requester_login))
}

/// Whether one allow-list `entry` names the caller. An entry matches when — after
/// trimming and ignoring a single leading `@` — it equals the caller's numeric id
/// (as a decimal string) OR the caller's login (ASCII case-insensitive). An empty
/// normalized entry never matches (so blank/junk tokens can never grant access).
fn entry_matches(entry: &str, requester_id_str: &str, requester_login: &str) -> bool {
    let normalized = entry.trim().trim_start_matches('@');
    if normalized.is_empty() {
        return false;
    }
    normalized == requester_id_str || normalized.eq_ignore_ascii_case(requester_login)
}

#[cfg(test)]
#[path = "log_authz_tests.rs"]
mod tests;
