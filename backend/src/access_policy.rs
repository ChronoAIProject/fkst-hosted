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
//! Four states:
//!
//! - **`all`** → open unconditionally, even if a stale `FKST_ACCESS_ALLOWED_USERS`
//!   or `FKST_ACCESS_BLOCKED_USERS` list is still present (the explicit
//!   "everyone" model always wins).
//! - **`allowlist`** (also `allow-list` / `selected`) → enforce the entries
//!   exactly as an enforced list does today, INCLUDING the fail-closed rule that
//!   an absent/empty list denies everyone. A stale `FKST_ACCESS_BLOCKED_USERS`
//!   list is tolerated and ignored (default-deny already governs).
//! - **`denylist`** (also `deny-list` / `blocklist` / `blacklist`, issue #3376)
//!   → every verified GitHub identity is allowed EXCEPT one matching
//!   `FKST_ACCESS_BLOCKED_USERS` (same entry grammar as the allowlist, same
//!   shared [`entry_matches`]). An unset/blank blocked list blocks nobody. A
//!   stale `FKST_ACCESS_ALLOWED_USERS` list is tolerated and ignored.
//! - **unset** → derived from the lists: only the allowed list set → allowlist
//!   (the exact legacy behavior); only the blocked list set → denylist (a set
//!   blocklist must never be silently ignored — that would fail open); BOTH
//!   set → a startup config error (ambiguous: the operator must pick the model
//!   explicitly); neither → open.
//!
//! Denylist fail-closed nuances: `FKST_ACCESS_BLOCKED_USERS` that is SET but
//! yields zero valid entries (e.g. `","`) under an effective denylist model is a
//! startup config error — a mangled blocklist must never silently admit the
//! users it meant to block, and (unlike the allowlist) "enforce-and-deny-all"
//! would contradict the model's purpose, so refusing to boot is the only honest
//! fail-closed state. `FKST_GLOBAL_ADMINS` always pass, INCLUDING an admin who
//! is also named in the blocked list — a conflict between the two operator-owned
//! lists resolves in the admin's favor (fix the config, not the gate).
//!
//! A non-empty but UNRECOGNIZED `FKST_AUTH_MODEL` fails closed at startup
//! (`from_vars` returns a config error naming the var + bad value), consistent
//! with the other hand-parsed enum knobs (e.g. `FKST_POD_MODE`).

use crate::error::AppError;

/// The env var holding the allowlist.
const ACCESS_ALLOWED_USERS_VAR: &str = "FKST_ACCESS_ALLOWED_USERS";

/// The env var holding the blocklist (consulted by the denylist model).
const ACCESS_BLOCKED_USERS_VAR: &str = "FKST_ACCESS_BLOCKED_USERS";

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
    /// allowlist or blocklist is present.
    All,
    /// Only allowlisted users are allowed — enforce `entries` (absent/empty list
    /// denies everyone, the fail-closed rule).
    Allowlist,
    /// Every authenticated GitHub user is allowed EXCEPT those matching
    /// `blocked` (absent/blank list blocks nobody; global admins always pass).
    Denylist,
}

/// The resolved access policy. `entries` / `blocked`: `None` = no list (var
/// unset/blank); `Some(list)` = a list is present (possibly empty). `mode` is
/// the explicit `FKST_AUTH_MODEL` override (`None` = derive the model from the
/// lists via [`Self::effective_model`], the legacy behavior). The model is
/// applied centrally in [`Self::allows`] / [`Self::enforced`] so the two gates
/// never branch on it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AccessPolicy {
    entries: Option<Vec<String>>,
    blocked: Option<Vec<String>>,
    mode: Option<AuthModel>,
    global_admins: Vec<String>,
}

impl AccessPolicy {
    /// Resolve the policy from the caller's already-collected env snapshot
    /// (the [`crate::config::Config::from_vars`] testable-seam convention).
    ///
    /// Fails closed (returns [`AppError::Config`]) when the configuration is
    /// mangled or ambiguous — an operator's explicit intent must never be
    /// silently ignored:
    /// - `FKST_AUTH_MODEL` set to a non-empty, unrecognized value;
    /// - BOTH `FKST_ACCESS_ALLOWED_USERS` and `FKST_ACCESS_BLOCKED_USERS` set
    ///   without an explicit `FKST_AUTH_MODEL` to disambiguate;
    /// - an effective denylist model whose `FKST_ACCESS_BLOCKED_USERS` is set
    ///   but yields zero valid entries (e.g. `","`).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Self, AppError> {
        // Shared comma-separated list parse: `None` = var unset/blank,
        // `Some(list)` = present (possibly empty after junk-token filtering).
        let list = |var: &str| {
            vars.iter()
                .find(|(key, _)| key == var)
                .map(|(_, value)| value.trim())
                .filter(|value| !value.is_empty())
                .map(|value| {
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|entry| !entry.is_empty())
                        .map(str::to_string)
                        .collect::<Vec<_>>()
                })
        };
        let entries = list(ACCESS_ALLOWED_USERS_VAR);
        let blocked = list(ACCESS_BLOCKED_USERS_VAR);

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
                "denylist" | "deny-list" | "blocklist" | "blacklist" => Some(AuthModel::Denylist),
                _ => {
                    return Err(AppError::Config(format!(
                        "FKST_AUTH_MODEL must be one of \"all\" | \"allowlist\" | \"denylist\" (got \"{value}\")"
                    )));
                }
            },
        };

        // Both lists with no explicit model is ambiguous: the legacy derivation
        // cannot pick one without silently ignoring the other (fail-open either
        // way). Refuse to boot; the operator names the model.
        if mode.is_none() && entries.is_some() && blocked.is_some() {
            return Err(AppError::Config(format!(
                "{ACCESS_ALLOWED_USERS_VAR} and {ACCESS_BLOCKED_USERS_VAR} are both set; \
                 set FKST_AUTH_MODEL to \"allowlist\" or \"denylist\" to pick the model"
            )));
        }

        let policy = Self {
            entries,
            blocked,
            mode,
            global_admins,
        };

        // A blocklist the denylist model will consult that is SET but yields no
        // valid entries (e.g. ",") is mangled config: silently blocking nobody
        // would admit the users it meant to block, and deny-all would defeat the
        // model — refusing to boot is the only honest fail-closed state. (Under
        // `all`/`allowlist` the same list is merely stale and tolerated.)
        if policy.effective_model() == Some(AuthModel::Denylist)
            && policy.blocked.as_deref().is_some_and(<[String]>::is_empty)
        {
            return Err(AppError::Config(format!(
                "{ACCESS_BLOCKED_USERS_VAR} is set but contains no valid entries"
            )));
        }

        Ok(policy)
    }

    /// The model actually in force: the explicit `FKST_AUTH_MODEL` override when
    /// set, else derived from the lists (allowed list → allowlist, blocked list
    /// → denylist — `from_vars` rejects both-at-once), else `None` = open. Used
    /// by startup logging to name the resolved model without exposing entries.
    /// The derivation never yields `All`, so `Some(All)` always means an
    /// explicit `FKST_AUTH_MODEL=all`.
    pub fn effective_model(&self) -> Option<AuthModel> {
        self.mode.or_else(|| {
            if self.blocked.is_some() {
                Some(AuthModel::Denylist)
            } else if self.entries.is_some() {
                Some(AuthModel::Allowlist)
            } else {
                None
            }
        })
    }

    /// Whether the policy is enforcing (an identity may be rejected).
    pub fn enforced(&self) -> bool {
        !matches!(self.effective_model(), None | Some(AuthModel::All))
    }

    /// Number of allowlist entries (0 when no list is present OR when set-but-empty;
    /// pair with [`Self::enforced`] for the startup log). Independent of `mode`.
    pub fn entry_count(&self) -> usize {
        self.entries.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Number of blocklist entries (0 when no list is present). Startup
    /// diagnostics only; entries themselves must never be logged.
    pub fn blocked_entry_count(&self) -> usize {
        self.blocked.as_ref().map(Vec::len).unwrap_or(0)
    }

    /// Number of configured global-admin entries. Used only for startup
    /// diagnostics; entries themselves must never be logged.
    pub fn global_admin_count(&self) -> usize {
        self.global_admins.len()
    }

    /// Configured global-admin LOGIN entries, normalized for the session runtime.
    /// Numeric-id entries are intentionally omitted: they remain valid for
    /// server-side authorization, but the packages' GitHub policy can match only
    /// issue/comment author logins. A leading `@` is stripped from login entries.
    pub fn global_admin_login_entries(&self) -> impl Iterator<Item = &str> {
        self.global_admins.iter().filter_map(|entry| {
            let normalized = entry.trim().trim_start_matches('@');
            if normalized.is_empty() || normalized.bytes().all(|byte| byte.is_ascii_digit()) {
                None
            } else {
                Some(normalized)
            }
        })
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
    /// A global admin is always allowed, whatever the model — including a
    /// denylist that (mis)lists the admin. Then, by [`Self::effective_model`]:
    ///
    /// - open / `All` → allowed unconditionally (open even with stale lists).
    /// - `Allowlist` → only a matching allowed entry; an absent/empty list
    ///   denies everyone (fail closed).
    /// - `Denylist` → allowed unless a blocked entry matches; an absent list
    ///   blocks nobody (a set-but-empty one is rejected in `from_vars`).
    pub fn allows(&self, id: i64, login: &str) -> bool {
        if self.is_global_admin(id, login) {
            return true;
        }
        match self.effective_model() {
            None | Some(AuthModel::All) => true,
            Some(AuthModel::Allowlist) => self.matches_entries(id, login),
            Some(AuthModel::Denylist) => !self.matches_blocked(id, login),
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

    /// Match `(id, login)` against the blocked list (absent list ⇒ no match ⇒
    /// nobody blocked). Same shared [`entry_matches`] grammar as the allowlist.
    fn matches_blocked(&self, id: i64, login: &str) -> bool {
        let id_str = id.to_string();
        self.blocked
            .as_deref()
            .unwrap_or(&[])
            .iter()
            .any(|entry| entry_matches(entry, &id_str, login))
    }
}

/// Whether one list `entry` (allow, block, or admin) names the caller. After trimming and ignoring a
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
    fn global_admin_login_entries_normalize_logins_and_exclude_numeric_ids() {
        let policy = policy(&vars(&[(
            "FKST_GLOBAL_ADMINS",
            " @Deploy-Admin, 583231, reviewer ",
        )]));
        assert_eq!(
            policy.global_admin_login_entries().collect::<Vec<_>>(),
            vec!["Deploy-Admin", "reviewer"]
        );
        assert_eq!(policy.global_admin_count(), 3);
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
    fn auth_model_denylist_blocks_listed_users_and_admits_everyone_else() {
        for value in [
            "denylist",
            "deny-list",
            "blocklist",
            "blacklist",
            "DenyList",
        ] {
            let policy = policy(&vars(&[
                ("FKST_AUTH_MODEL", value),
                ("FKST_ACCESS_BLOCKED_USERS", " 583231 , @Mallory-Dev ,eve"),
            ]));
            assert!(policy.enforced(), "denylist is an enforcing model");
            assert_eq!(policy.blocked_entry_count(), 3);
            // Blocked by numeric id (login irrelevant).
            assert!(!policy.allows(583231, "whatever-login"));
            // Blocked by login: case-insensitive, leading @ tolerated.
            assert!(!policy.allows(999, "mallory-dev"));
            assert!(!policy.allows(999, "EVE"));
            // Anyone else is admitted — the whole point of the model.
            assert!(policy.allows(999, "alice"));
            assert!(policy.allows(1, "anyone"));
        }
    }

    #[test]
    fn denylist_numeric_entry_blocks_only_the_id_not_a_login() {
        let policy = policy(&vars(&[
            ("FKST_AUTH_MODEL", "denylist"),
            ("FKST_ACCESS_BLOCKED_USERS", "583231"),
        ]));
        // The id/login disjointness guard carries over: a numeric blocked entry
        // must not block an unrelated user whose LOGIN is the same digits.
        assert!(policy.allows(999, "583231"));
        assert!(!policy.allows(583231, "anything"));
    }

    #[test]
    fn denylist_with_no_blocked_list_blocks_nobody() {
        let policy = policy(&vars(&[("FKST_AUTH_MODEL", "denylist")]));
        assert!(policy.enforced());
        assert_eq!(policy.blocked_entry_count(), 0);
        assert!(policy.allows(1, "anyone"));
    }

    #[test]
    fn denylist_junk_blocked_entries_never_block() {
        // "@" normalizes to empty at match time → never matches anyone (and is a
        // present entry at parse time, so no zero-entries error either).
        let policy = policy(&vars(&[
            ("FKST_AUTH_MODEL", "denylist"),
            ("FKST_ACCESS_BLOCKED_USERS", "@,mallory"),
        ]));
        assert!(policy.allows(7, ""));
        assert!(!policy.allows(7, "mallory"));
    }

    #[test]
    fn denylist_set_but_empty_blocked_list_fails_closed_at_startup() {
        // "," yields zero valid entries: silently blocking nobody would admit
        // the users the operator meant to block — refuse to boot instead.
        for pairs in [
            vec![
                ("FKST_AUTH_MODEL", "denylist"),
                ("FKST_ACCESS_BLOCKED_USERS", " , "),
            ],
            // Same via the derived model (blocked list present, mode unset).
            vec![("FKST_ACCESS_BLOCKED_USERS", " , ")],
        ] {
            let err = AccessPolicy::from_vars(&vars(&pairs))
                .expect_err("a mangled blocklist must fail closed");
            assert!(
                err.to_string().contains("FKST_ACCESS_BLOCKED_USERS"),
                "error names the var: {err}"
            );
        }
    }

    #[test]
    fn blocked_list_alone_derives_the_denylist_model() {
        // Mode unset + only FKST_ACCESS_BLOCKED_USERS set: a set blocklist must
        // never be silently ignored, so the denylist model is derived.
        let policy = policy(&vars(&[("FKST_ACCESS_BLOCKED_USERS", "mallory")]));
        assert_eq!(policy.effective_model(), Some(AuthModel::Denylist));
        assert!(policy.enforced());
        assert!(!policy.allows(999, "Mallory"));
        assert!(policy.allows(999, "alice"));
    }

    #[test]
    fn both_lists_without_an_explicit_model_fail_closed() {
        let err = AccessPolicy::from_vars(&vars(&[
            ("FKST_ACCESS_ALLOWED_USERS", "alice"),
            ("FKST_ACCESS_BLOCKED_USERS", "mallory"),
        ]))
        .expect_err("ambiguous dual-list config must fail closed");
        let msg = err.to_string();
        assert!(msg.contains("FKST_ACCESS_ALLOWED_USERS"), "{msg}");
        assert!(msg.contains("FKST_ACCESS_BLOCKED_USERS"), "{msg}");
        assert!(msg.contains("FKST_AUTH_MODEL"), "{msg}");
    }

    #[test]
    fn global_admin_wins_over_the_blocklist() {
        // An admin (mis)listed in the blocked list stays admitted — the two
        // operator-owned lists conflict and the admin role wins (documented).
        let policy = policy(&vars(&[
            ("FKST_AUTH_MODEL", "denylist"),
            ("FKST_ACCESS_BLOCKED_USERS", "chronoai-shining, 583231"),
            ("FKST_GLOBAL_ADMINS", "chronoai-shining"),
        ]));
        assert!(policy.allows(999, "ChronoAI-Shining"));
        assert!(!policy.allows(583231, "someone-else"));
    }

    #[test]
    fn explicit_models_tolerate_the_other_models_stale_list() {
        // all: open even with a stale blocklist (mirrors the stale-allowlist rule).
        let open = policy(&vars(&[
            ("FKST_AUTH_MODEL", "all"),
            ("FKST_ACCESS_BLOCKED_USERS", "mallory"),
        ]));
        assert!(!open.enforced());
        assert!(open.allows(999, "mallory"));

        // allowlist: a stale blocklist is inert (default-deny already governs) —
        // even for a user named in BOTH lists.
        let allow = policy(&vars(&[
            ("FKST_AUTH_MODEL", "allowlist"),
            ("FKST_ACCESS_ALLOWED_USERS", "alice"),
            ("FKST_ACCESS_BLOCKED_USERS", "alice, mallory"),
        ]));
        assert!(allow.allows(999, "alice"));
        assert!(!allow.allows(999, "mallory"));

        // denylist: a stale allowlist is inert — not being on it denies nobody.
        let deny = policy(&vars(&[
            ("FKST_AUTH_MODEL", "denylist"),
            ("FKST_ACCESS_ALLOWED_USERS", "alice"),
            ("FKST_ACCESS_BLOCKED_USERS", "mallory"),
        ]));
        assert!(deny.allows(999, "bob"));
        assert!(!deny.allows(999, "mallory"));
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
