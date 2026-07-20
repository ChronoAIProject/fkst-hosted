//! Deployment-wide GitHub-identity access policy (issue #463).
//!
//! `FKST_ACCESS_ALLOWED_USERS` — a comma-separated list of GitHub numeric user
//! ids and/or logins (the same entry format as `FKST_LOG_ADMINS`) — restricts who
//! may use the service. **Unset/blank = open** (today's behavior, the dev/test
//! default). **Set = enforced** at the two choke points:
//!
//! 1. The [`crate::github_identity::GithubUser`] extractor: a VERIFIED GitHub
//!    identity that is not listed gets `403 Forbidden` — covering every
//!    token-authenticated route (env store, dashboard, and anything added later)
//!    in one place.
//! 2. The reconciler's registration intake ([`crate::reconcile::repo`]): a
//!    trigger issue authored by an unlisted user is IGNORED before parsing —
//!    it never spawns a session (webhook or full-resync path alike), never gets
//!    an invalid-config comment, and an already-running session whose author is
//!    removed from the list is torn down on the next reconcile (revocation).
//!
//! Fail-closed nuance: a var that is SET but yields no valid entries (e.g. `","`)
//! is ENFORCED-and-denies-all — the operator explicitly asked for enforcement, so
//! a mangled list must never silently fall open. Startup logs the mode + entry
//! count (never the entries themselves).
//!
//! Numeric ids are preferred (stable across login renames) and are the ONLY key
//! matched everywhere; logins additionally match on the token-authenticated
//! routes and the trigger gate (both have the login in hand). Matching is
//! delegated to [`entry_matches`] — one source of truth shared with the
//! log-download authz tiers ([`crate::reconcile::log_authz`]).

/// The env var holding the allowlist.
const ACCESS_ALLOWED_USERS_VAR: &str = "FKST_ACCESS_ALLOWED_USERS";

/// The resolved access policy. `entries: None` = open (var unset/blank);
/// `Some(list)` = enforced against the list (possibly empty = deny all).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessPolicy {
    entries: Option<Vec<String>>,
}

impl AccessPolicy {
    /// Resolve the policy from the caller's already-collected env snapshot
    /// (the [`crate::config::Config::from_vars`] testable-seam convention).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Self {
        let raw = vars
            .iter()
            .find(|(key, _)| key == ACCESS_ALLOWED_USERS_VAR)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty());
        let entries = raw.map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|entry| !entry.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        });
        Self { entries }
    }

    /// Whether the policy is enforcing (the var was set non-blank).
    pub fn enforced(&self) -> bool {
        self.entries.is_some()
    }

    /// Number of allowlist entries (0 when open OR when set-but-empty; pair with
    /// [`Self::enforced`] for the startup log).
    pub fn entry_count(&self) -> usize {
        self.entries.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Whether the VERIFIED GitHub identity `(id, login)` may use the service.
    /// Open policy allows everyone; an enforced policy allows only a matching
    /// entry (numeric id as decimal string, or login case-insensitively).
    pub fn allows(&self, id: i64, login: &str) -> bool {
        match &self.entries {
            None => true,
            Some(entries) => {
                let id_str = id.to_string();
                entries
                    .iter()
                    .any(|entry| entry_matches(entry, &id_str, login))
            }
        }
    }
}

/// Whether one allow-list `entry` names the caller. After trimming and ignoring a
/// single leading `@`, the entry's SHAPE decides which namespace it addresses — the
/// id and login namespaces are kept strictly DISJOINT:
///
/// - An **all-ASCII-digit** entry is a numeric GitHub id and matches ONLY the
///   caller's id (as a decimal string), never the login.
/// - Any **other** entry is a login and matches ONLY the caller's login (ASCII
///   case-insensitive).
///
/// An empty normalized entry never matches (so blank/junk tokens can never grant
/// access). Why the disjointness matters for security: all-numeric GitHub usernames
/// are real (e.g. `github.com/0`), so if a numeric entry also matched a login,
/// whoever registers the all-numeric username equal to an id could impersonate the
/// identity listed by that id — and a numeric login entry would cross-match an
/// unrelated user whose id happens to equal it. Numeric entries are therefore
/// id-only.
///
/// The ONE matcher shared by this policy and the log-download authz tiers
/// ([`crate::reconcile::log_authz`]) so the two allowlist syntaxes can never drift.
pub(crate) fn entry_matches(entry: &str, requester_id_str: &str, requester_login: &str) -> bool {
    let normalized = entry.trim().trim_start_matches('@');
    if normalized.is_empty() {
        return false;
    }
    // All-digit ⇒ numeric id; match the id namespace only (never a login).
    if normalized.bytes().all(|b| b.is_ascii_digit()) {
        return normalized == requester_id_str;
    }
    // Otherwise a login; match the login namespace only.
    normalized.eq_ignore_ascii_case(requester_login)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn unset_or_blank_is_open() {
        for v in [vars(&[]), vars(&[("FKST_ACCESS_ALLOWED_USERS", "   ")])] {
            let policy = AccessPolicy::from_vars(&v);
            assert!(!policy.enforced());
            assert!(policy.allows(1, "anyone"), "open policy allows everyone");
        }
    }

    #[test]
    fn set_list_enforces_by_id_and_login() {
        let policy = AccessPolicy::from_vars(&vars(&[(
            "FKST_ACCESS_ALLOWED_USERS",
            " 583231 , @Alice-Dev ,bob",
        )]));
        assert!(policy.enforced());
        assert_eq!(policy.entry_count(), 3);
        // Numeric id match.
        assert!(policy.allows(583231, "whatever-login"));
        // Login match: case-insensitive, leading @ on the entry tolerated.
        assert!(policy.allows(999, "alice-dev"));
        assert!(policy.allows(999, "BOB"));
        // Neither id nor login listed.
        assert!(!policy.allows(999, "mallory"));
        // A NUMERIC entry is id-only: it must never match a user whose LOGIN happens
        // to equal it. Here the numeric entry "583231" must not admit a different
        // user (id 999) whose login is the string "583231".
        assert!(!policy.allows(999, "583231"));
    }

    #[test]
    fn numeric_entry_matches_only_the_id_not_a_login() {
        // A numeric entry matches the requester's id...
        assert!(entry_matches("583231", "583231", "whatever-login"));
        assert!(entry_matches("@583231", "583231", "whatever-login"));
        // ...but NEVER a login that happens to be the same digits (all-numeric
        // GitHub usernames exist, so this disjointness is a real impersonation guard).
        assert!(!entry_matches("583231", "999", "583231"));
    }

    #[test]
    fn non_numeric_login_entry_matches_login_only() {
        // A login entry is unchanged: case-insensitive login match, never an id.
        assert!(entry_matches("OctoCat", "999", "octocat"));
        assert!(!entry_matches("octocat", "999", "someone-else"));
    }

    #[test]
    fn set_but_empty_enforces_and_denies_all() {
        // "," yields zero valid entries: the operator asked for enforcement, so
        // this must deny everyone rather than silently fall open.
        let policy = AccessPolicy::from_vars(&vars(&[("FKST_ACCESS_ALLOWED_USERS", " , ")]));
        assert!(policy.enforced());
        assert_eq!(policy.entry_count(), 0);
        assert!(!policy.allows(583231, "octocat"));
    }

    #[test]
    fn junk_entries_never_grant_access() {
        let policy = AccessPolicy::from_vars(&vars(&[("FKST_ACCESS_ALLOWED_USERS", "@,alice")]));
        // "@" normalizes to empty → never matches, even an empty login.
        assert!(!policy.allows(7, ""));
        assert!(policy.allows(7, "alice"));
    }

    #[test]
    fn entry_matches_contract() {
        assert!(entry_matches("583231", "583231", "octocat"));
        assert!(entry_matches("@OctoCat", "583231", "octocat"));
        assert!(!entry_matches("", "583231", "octocat"));
        assert!(!entry_matches("   ", "583231", "octocat"));
        assert!(!entry_matches("583232", "583231", "octocat"));
    }
}
