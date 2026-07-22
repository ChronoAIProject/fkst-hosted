//! Pure session-scoped authorization for on-demand session-log downloads.
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
//! 1. **Creator** — the caller is the session's effective creator (matched by
//!    immutable id when available, otherwise by assignee-derived login).
//! 2. **Per-issue allow-list** — the caller matches an entry the author listed in the
//!    trigger issue's `### Log Access Allowlist` section (frozen by config-immutability).
//! 3. **Legacy log admins** — the caller matches an operator-configured entry
//!    (`FKST_LOG_ADMINS`) that may pull ANY session's logs. Deployment-wide
//!    `FKST_GLOBAL_ADMINS` are checked by the route-level policy before this
//!    compatibility function is called.
//!
//! A list entry (tiers 2 and 3) is matched against BOTH the caller's numeric id (as a
//! decimal string) AND the caller's login (case-insensitively), so an operator or
//! author may list either form. A leading `@` on an entry is ignored so `@alice` and
//! `alice` are equivalent. An empty/whitespace entry never matches anything.

/// Decide whether the verified caller `(requester_id, requester_login)` may download
/// the logs of a session owned by `creator` and
/// whose `### Log Access Allowlist` allow-list is `per_issue_allow`, given the operator's
/// `legacy_log_admins`.
///
/// Returns `true` iff the caller satisfies AT LEAST ONE of the three tiers described
/// in the module docs; `false` (deny) otherwise. Purely a function of its inputs.
pub fn is_authorized(
    requester_id: i64,
    requester_login: &str,
    creator: &crate::reconcile::creator::SessionCreator,
    per_issue_allow: &[String],
    legacy_log_admins: &[String],
) -> bool {
    // Tier 1: the effective creator. Prefer the immutable id for human-authored
    // triggers; App-authored triggers have only the assignee login available.
    let creator_matches = match creator.id {
        Some(creator_id) => requester_id == creator_id,
        None => {
            !creator.login.trim().is_empty() && requester_login.eq_ignore_ascii_case(&creator.login)
        }
    };
    if creator_matches {
        return true;
    }
    // Tiers 2 + 3: any per-issue OR any global-admin entry that matches the caller by
    // numeric id (as a string) OR by case-insensitive login.
    let requester_id_str = requester_id.to_string();
    per_issue_allow
        .iter()
        .chain(legacy_log_admins.iter())
        .any(|entry| entry_matches(entry, &requester_id_str, requester_login))
}

// The entry matcher is shared with the deployment-wide access policy so the two
// allowlist syntaxes can never drift — see `crate::access_policy::entry_matches`.
use crate::access_policy::entry_matches;

#[cfg(test)]
#[path = "log_authz_tests.rs"]
mod tests;
