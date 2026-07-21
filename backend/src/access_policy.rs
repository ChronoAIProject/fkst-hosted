//! Deployment-wide GitHub-identity access policy (issue #463).
//!
//! `FKST_ACCESS_ALLOWED_USERS` — a comma-separated list of GitHub logins and/or
//! numeric user ids (the same entry format as `FKST_GLOBAL_ADMINS`) — restricts
//! who may use the service. **Unset/blank = open** (today's behavior, the dev/test
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
//! Logins are the operator-friendly form (`chronoai-shining` or
//! `@chronoai-shining`, case-insensitive); numeric ids remain supported as the
//! immutable rename-safe form. Matching is
//! delegated to [`entry_matches`] — one source of truth shared with the
//! log-download authz tiers ([`crate::reconcile::log_authz`]).
//!
//! `FKST_GLOBAL_ADMINS` uses the same entry grammar. A verified global admin is
//! always admitted by this deployment access gate, including when
//! `FKST_AUTH_MODEL=allowlist` and the ordinary allowlist does not repeat the
//! identity. The role is consumed by the App-wide canvas and session/log observe
//! surfaces; the configured entries themselves are never exposed or logged.
//!
//! `FKST_AUTH_MODEL` (issue #594) makes the auth model an explicit choice that
//! overrides the entries-derived default, without changing any gate: it is
//! resolved centrally in [`AccessPolicy::allows`]/[`AccessPolicy::enforced`].
//! Three states:
//!
//! - **`all`** → open unconditionally, even if a stale `FKST_ACCESS_ALLOWED_USERS`
//!   list is still present (the explicit "everyone" model always wins).
//! - **`allowlist`** (also `allow-list` / `selected`) → enforce the entries
//!   exactly as an enforced list does today, INCLUDING the fail-closed rule that
//!   an absent/empty list denies everyone.
//! - **unset** → the exact legacy behavior: entries-if-set, else open.
//!
//! A non-empty but UNRECOGNIZED `FKST_AUTH_MODEL` fails closed at startup
//! (`from_vars` returns a config error naming the var + bad value), consistent
//! with the other hand-parsed enum knobs (e.g. `FKST_POD_MODE`).

use crate::error::AppError;

/// The env var holding the allowlist.
const ACCESS_ALLOWED_USERS_VAR: &str = "FKST_ACCESS_ALLOWED_USERS";

/// The env var holding the deployment-wide global administrator list.
const GLOBAL_ADMINS_VAR: &str = "FKST_GLOBAL_ADMINS";

/// The env var selecting the auth model (see [`AuthModel`]).
const AUTH_MODEL_VAR: &str = "FKST_AUTH_MODEL";

/// The platform auth model, selected by `FKST_AUTH_MODEL`. An explicit override
/// of the entries-derived default; `None` (var unset) keeps the legacy behavior.
/// Hand-parsed in [`AccessPolicy::from_vars`] (like [`crate::config::PodMode`]) so
/// a bad value surfaces our own precise error text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AuthModel {
    /// Every authenticated GitHub user is allowed — open even if a stale
    /// allowlist is present.
    All,
    /// Only allowlisted users are allowed — enforce `entries` (absent/empty list
    /// denies everyone, the fail-closed rule).
    Allowlist,
}

/// The resolved access policy. `entries: None` = no list (var unset/blank);
/// `Some(list)` = a list is present (possibly empty). `mode` is the explicit
/// `FKST_AUTH_MODEL` override (`None` = derive the model from `entries`, the
/// legacy behavior). The model is applied centrally in [`Self::allows`] /
/// [`Self::enforced`] so the two gates never branch on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessPolicy {
    entries: Option<Vec<String>>,
    mode: Option<AuthModel>,
    global_admins: Vec<String>,
}

impl AccessPolicy {
    /// Resolve the policy from the caller's already-collected env snapshot
    /// (the [`crate::config::Config::from_vars`] testable-seam convention).
    ///
    /// Fails closed (returns [`AppError::Config`]) when `FKST_AUTH_MODEL` is set
    /// to a non-empty, unrecognized value — an operator's explicit auth-model
    /// choice must never be silently ignored.
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
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

        let global_admins = vars
            .iter()
            .find(|(key, _)| key == GLOBAL_ADMINS_VAR)
            .map(|(_, value)| value.as_str())
            .unwrap_or_default()
            .split(',')
            .map(str::trim)
            .filter(|entry| !entry.is_empty())
            .map(str::to_string)
            .collect();

        // Explicit auth-model override. Case-insensitive; blank/unset defers to
        // the entries-derived default. A non-empty unrecognized value fails
        // closed, naming the var + the offending value (matching FKST_POD_MODE).
        let mode = match vars
            .iter()
            .find(|(key, _)| key == AUTH_MODEL_VAR)
            .map(|(_, value)| value.trim())
            .filter(|value| !value.is_empty())
        {
            None => None,
            Some(value) => match value.to_ascii_lowercase().as_str() {
                "all" => Some(AuthModel::All),
                "allowlist" | "allow-list" | "selected" => Some(AuthModel::Allowlist),
                _ => {
                    return Err(AppError::Config(format!(
                        "FKST_AUTH_MODEL must be one of \"all\" | \"allowlist\" (got \"{value}\")"
                    )));
                }
            },
        };

        Ok(Self {
            entries,
            mode,
            global_admins,
        })
    }

    /// The explicit `FKST_AUTH_MODEL` override, or `None` when the model is
    /// derived from the presence of a list. Used by startup logging to name the
    /// resolved model without exposing the entries.
    pub fn model(&self) -> Option<AuthModel> {
        self.mode
    }

    /// Whether the policy is enforcing (an identity may be rejected). Driven by
    /// the explicit model when set, else derived from the presence of a list.
    pub fn enforced(&self) -> bool {
        match self.mode {
            Some(AuthModel::All) => false,
            Some(AuthModel::Allowlist) => true,
            None => self.entries.is_some(),
        }
    }

    /// Number of allowlist entries (0 when no list is present OR when set-but-empty;
    /// pair with [`Self::enforced`] for the startup log). Independent of `mode`.
    pub fn entry_count(&self) -> usize {
        self.entries.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Number of configured global-admin entries. Used only for startup
    /// diagnostics; entries themselves must never be logged.
    pub fn global_admin_count(&self) -> usize {
        self.global_admins.len()
    }

    /// Whether the VERIFIED GitHub identity is a deployment-wide global admin.
    /// Login matching is ASCII case-insensitive and accepts an optional leading
    /// `@`; all-digit entries match only the immutable numeric id.
    pub fn is_global_admin(&self, id: i64, login: &str) -> bool {
        let id_str = id.to_string();
        self.global_admins
            .iter()
            .any(|entry| entry_matches(entry, &id_str, login))
    }

    /// Whether the VERIFIED GitHub identity `(id, login)` may use the service.
    ///
    /// - `mode = All` → allowed unconditionally (open even with a stale list).
    /// - `mode = Allowlist` → only a matching entry is allowed; an absent/empty
    ///   list denies everyone (fail closed).
    /// - `mode` unset → legacy behavior: open when no list is present, else only
    ///   a matching entry.
    pub fn allows(&self, id: i64, login: &str) -> bool {
        if self.is_global_admin(id, login) {
            return true;
        }
        match self.mode {
            Some(AuthModel::All) => true,
            Some(AuthModel::Allowlist) => self.matches_entries(id, login),
            None => match self.entries {
                None => true,
                Some(_) => self.matches_entries(id, login),
            },
        }
    }

    /// Match `(id, login)` against the entries list, treating an absent list as
    /// empty (⇒ no match). Used only by the enforcing paths, so an absent list
    /// denies everyone (the fail-closed rule).
    fn matches_entries(&self, id: i64, login: &str) -> bool {
        let id_str = id.to_string();
        self.entries
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|entry| entry_matches(entry, &id_str, login))
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

    /// Unwrap a policy the test expects to parse cleanly.
    fn policy(vars: &[(String, String)]) -> AccessPolicy {
        AccessPolicy::from_vars(vars).expect("policy should parse")
    }

    #[test]
    fn unset_or_blank_is_open() {
        for v in [vars(&[]), vars(&[("FKST_ACCESS_ALLOWED_USERS", "   ")])] {
            let policy = policy(&v);
            assert!(!policy.enforced());
            assert!(policy.allows(1, "anyone"), "open policy allows everyone");
        }
    }

    #[test]
    fn set_list_enforces_by_id_and_login() {
        let policy = policy(&vars(&[(
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
        let policy = policy(&vars(&[("FKST_ACCESS_ALLOWED_USERS", " , ")]));
        assert!(policy.enforced());
        assert_eq!(policy.entry_count(), 0);
        assert!(!policy.allows(583231, "octocat"));
    }

    #[test]
    fn junk_entries_never_grant_access() {
        let policy = policy(&vars(&[("FKST_ACCESS_ALLOWED_USERS", "@,alice")]));
        // "@" normalizes to empty → never matches, even an empty login.
        assert!(!policy.allows(7, ""));
        assert!(policy.allows(7, "alice"));
    }

    #[test]
    fn global_admin_login_is_first_class_and_bypasses_the_user_allowlist() {
        let policy = policy(&vars(&[
            ("FKST_AUTH_MODEL", "allowlist"),
            ("FKST_ACCESS_ALLOWED_USERS", "someone-else"),
            ("FKST_GLOBAL_ADMINS", " @ChronoAI-Shining, 583231 "),
        ]));

        assert_eq!(policy.global_admin_count(), 2);
        assert!(policy.is_global_admin(999, "chronoai-shining"));
        assert!(policy.allows(999, "CHRONOAI-SHINING"));
        assert!(policy.is_global_admin(583231, "renamed-login"));
        assert!(policy.allows(583231, "renamed-login"));
        assert!(!policy.is_global_admin(999, "someone-else"));
        assert!(policy.allows(999, "someone-else"));
        assert!(!policy.allows(999, "mallory"));
    }

    #[test]
    fn blank_global_admin_list_grants_nothing() {
        let policy = policy(&vars(&[("FKST_GLOBAL_ADMINS", " , ")]));
        assert_eq!(policy.global_admin_count(), 0);
        assert!(!policy.is_global_admin(1, "anyone"));
        // A global-admin list alone does not close an otherwise-open deployment.
        assert!(policy.allows(1, "anyone"));
    }

    #[test]
    fn auth_model_all_opens_even_with_a_populated_list() {
        // Explicit "all" wins over a present (stale) allowlist: everyone allowed.
        for value in ["all", "ALL", "All"] {
            let policy = policy(&vars(&[
                ("FKST_ACCESS_ALLOWED_USERS", "583231, alice"),
                ("FKST_AUTH_MODEL", value),
            ]));
            assert!(!policy.enforced(), "all model is never enforcing");
            assert!(
                policy.allows(999, "mallory"),
                "all model allows a non-listed user"
            );
            assert!(policy.allows(583231, "alice"));
        }
    }

    #[test]
    fn auth_model_allowlist_enforces_and_empty_denies_all() {
        // "allowlist" with a list enforces it exactly like the legacy set path.
        for value in ["allowlist", "allow-list", "selected", "AllowList"] {
            let policy = policy(&vars(&[
                ("FKST_ACCESS_ALLOWED_USERS", "583231"),
                ("FKST_AUTH_MODEL", value),
            ]));
            assert!(policy.enforced());
            assert!(policy.allows(583231, "whatever"));
            assert!(!policy.allows(999, "mallory"));
        }
        // "allowlist" with NO list present is fail-closed deny-all (not open).
        let policy = policy(&vars(&[("FKST_AUTH_MODEL", "allowlist")]));
        assert!(policy.enforced());
        assert_eq!(policy.entry_count(), 0);
        assert!(
            !policy.allows(583231, "octocat"),
            "allowlist + empty denies all"
        );
    }

    #[test]
    fn auth_model_unrecognized_fails_closed_naming_the_var() {
        let err = AccessPolicy::from_vars(&vars(&[("FKST_AUTH_MODEL", "everyone")]))
            .expect_err("an unrecognized auth model must fail closed");
        let msg = err.to_string();
        assert!(
            msg.contains("FKST_AUTH_MODEL"),
            "error names the var: {msg}"
        );
        assert!(msg.contains("everyone"), "error names the bad value: {msg}");
    }

    #[test]
    fn auth_model_blank_defers_to_the_entries_default() {
        // A blank FKST_AUTH_MODEL is treated as unset: legacy entries-derived
        // behavior (here: no list ⇒ open).
        let policy = policy(&vars(&[("FKST_AUTH_MODEL", "  ")]));
        assert!(!policy.enforced());
        assert!(policy.allows(1, "anyone"));
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
