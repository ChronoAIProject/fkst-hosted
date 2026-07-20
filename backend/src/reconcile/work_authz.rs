//! Pure work-issue authority predicate (R3, epic #572 · Wave 1).
//!
//! SECURITY-CRITICAL core of the work-issue authority gate: it decides which
//! GitHub user may raise work for a given session. Only the session's
//! **author ∪ Session Collaborators ∪ repo admins / org owners** may open an issue
//! carrying the session's work label; anyone else is rejected (the effectful reject
//! surface lives in [`crate::reconcile::work_ack`], and the pending gate in
//! [`crate::reconcile::pending`] counts only authorized authors).
//!
//! This module is PURE — no I/O, no clock — so the whole allow/deny matrix is
//! exhaustively unit-testable. The admin set is fetched by the driver
//! ([`crate::reconcile::repo`]) and passed in via [`WorkAuthz`], which also encodes
//! the enforcement opt-in and the three-state lookup-error policy (off / enforcing
//! with admins / enforcing with an empty admin set) — all the driver's concern, not
//! this predicate's.
//!
//! Identity keys, in preference order: an IMMUTABLE numeric GitHub id wherever it
//! is in hand (the trigger author and the admin set both carry ids — a login is
//! renamable and must never be the sole key there). Session Collaborators are
//! stored as the raw `### Session Collaborators` entries (logins, or ids), so they
//! are matched through the shared [`entry_matches`] matcher, which accepts EITHER a
//! numeric id OR a case-insensitive login (a leading `@` tolerated). Consequence: a
//! collaborator listed only by login who later RENAMES their GitHub account is no
//! longer matched — list a collaborator by numeric id to survive a rename.

use crate::access_policy::entry_matches;
use crate::models::GithubActor;
use crate::reconcile::desired::SessionRegistration;

/// Whether `(author_id, author_login)` — the verified author of a work-label issue —
/// is authorized to raise work for the session described by `reg`, given the repo's
/// admin/org-owner set `admins`.
///
/// Returns `true` iff the author satisfies AT LEAST ONE tier:
/// 1. **Author** — the session's own trigger author (matched by immutable id).
/// 2. **Repo admin / org owner** — a member of `admins` (matched by immutable id).
/// 3. **Session Collaborator** — a `reg.collaborators` entry matching the author by
///    numeric id (as a decimal string) OR case-insensitive login ([`entry_matches`]).
///
/// Deny by default otherwise. Purely a function of its inputs.
pub fn is_work_author_allowed(
    reg: &SessionRegistration,
    admins: &[GithubActor],
    author_id: i64,
    author_login: &str,
) -> bool {
    // Tier 1: the session's own trigger author, by IMMUTABLE numeric id (never the
    // renamable login — the id is the control-path authz subject everywhere).
    if author_id == reg.trigger_author_id {
        return true;
    }
    // Tier 2: a repo admin / org owner, also by immutable id (the admin set carries
    // ids straight from `list_repo_admins`).
    if admins.iter().any(|admin| admin.id == author_id) {
        return true;
    }
    // Tier 3: a listed Session Collaborator. The entries are raw `### Session
    // Collaborators` tokens (logins or ids), so the shared matcher accepts either
    // form; the numeric id is preferred (rename-safe) when the author listed one.
    let author_id_str = author_id.to_string();
    reg.collaborators
        .iter()
        .any(|entry| entry_matches(entry, &author_id_str, author_login))
}

/// The R3 work-issue authority decision for ONE repo reconcile pass, resolved by the
/// driver ([`crate::reconcile::repo`]) and threaded to the pending gate + the reject
/// surface. It encodes THREE distinct states (not two) so a transient admin-lookup
/// blip does not collapse enforcement back to fully permissive:
///
/// - **flag OFF** → `enforce == false`: no enforcement, byte-identical to pre-R3
///   (the author-blind pending count; the ack step never rejects).
/// - **flag ON + admin lookup FAILED this pass** → `enforce == true`, `admins`
///   EMPTY: still enforce the tiers that need NO GitHub call — the session author
///   and its Session Collaborators — so a stranger is STILL rejected; only the
///   admin/org-owner tier is unavailable that pass (it recovers on the next sweep).
/// - **flag ON + admin lookup SUCCEEDED** → `enforce == true`, `admins` populated:
///   full author ∪ collaborators ∪ admins enforcement.
#[derive(Debug, Clone)]
pub struct WorkAuthz {
    /// Whether the R3 authority gate is active this pass (the operator opted in).
    /// `false` is exactly the pre-R3 permissive behavior.
    pub enforce: bool,
    /// The repo admin / org-owner set. Empty when enforcement is off OR when the
    /// admin lookup failed this pass (in which case author ∪ collaborators are still
    /// enforced — never a full fail-open).
    pub admins: Vec<GithubActor>,
}

impl WorkAuthz {
    /// Enforcement is off — the legacy permissive path.
    pub fn off() -> Self {
        Self {
            enforce: false,
            admins: Vec::new(),
        }
    }

    /// Enforcement is on with the given admin set (empty = admin tier unavailable
    /// this pass; author ∪ collaborators are still enforced).
    pub fn enforcing(admins: Vec<GithubActor>) -> Self {
        Self {
            enforce: true,
            admins,
        }
    }

    /// Whether `(author_id, author_login)` may raise work for `reg` under this pass's
    /// decision. When enforcement is off, EVERY author is allowed (legacy). When on,
    /// delegates to [`is_work_author_allowed`] against this pass's admin set.
    pub fn allows(&self, reg: &SessionRegistration, author_id: i64, author_login: &str) -> bool {
        !self.enforce || is_work_author_allowed(reg, &self.admins, author_id, author_login)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::goals::trigger_parse::PackageRef;
    use crate::models::RepoRef;
    use crate::reconcile::desired::SessionDef;

    /// Build a minimal registration with the given trigger-author id + collaborator
    /// list; nothing else the predicate reads varies across these cases.
    fn reg(trigger_author_id: i64, collaborators: &[&str]) -> SessionRegistration {
        SessionRegistration {
            installation_id: 42,
            repo: RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
            trigger_issue: 7,
            trigger_author_id,
            trigger_author_login: "author-login".to_string(),
            def: SessionDef {
                name: "site".to_string(),
                packages: Vec::<PackageRef>::new(),
                work_label: Some("fkst-run".to_string()),
                environment: None,
                output_lang: None,
                engine_config: std::collections::BTreeMap::new(),
            },
            session_id: "sess-abc".to_string(),
            config_hash: "hash123".to_string(),
            auto_merge: false,
            log_access: vec![],
            collaborators: collaborators.iter().map(|s| s.to_string()).collect(),
        }
    }

    fn admin(id: i64, login: &str) -> GithubActor {
        GithubActor {
            id,
            login: login.to_string(),
        }
    }

    #[test]
    fn author_is_allowed_by_immutable_id() {
        let reg = reg(583231, &[]);
        // The author's login could be anything — the id is what authorizes.
        assert!(is_work_author_allowed(&reg, &[], 583231, "renamed-since"));
    }

    #[test]
    fn admin_is_allowed_by_id() {
        let reg = reg(7, &[]);
        let admins = [admin(500, "octo-admin"), admin(501, "org-owner")];
        assert!(is_work_author_allowed(&reg, &admins, 501, "whatever-login"));
    }

    #[test]
    fn collaborator_is_allowed_by_login_case_insensitively() {
        let reg = reg(7, &["@Bob", "carol"]);
        // Listed as "@Bob"; the author logs in as "bob" (case-insensitive, @ stripped).
        assert!(is_work_author_allowed(&reg, &[], 999, "bob"));
        assert!(is_work_author_allowed(&reg, &[], 1000, "CAROL"));
    }

    #[test]
    fn collaborator_is_allowed_by_numeric_id() {
        // A collaborator listed by numeric id survives a login rename (id is stable).
        let reg = reg(7, &["4242"]);
        assert!(is_work_author_allowed(&reg, &[], 4242, "any-current-login"));
    }

    #[test]
    fn stranger_is_rejected() {
        let reg = reg(7, &["bob"]);
        let admins = [admin(500, "octo-admin")];
        // Not the author (7), not an admin (500), not the collaborator (bob).
        assert!(!is_work_author_allowed(&reg, &admins, 999, "mallory"));
    }

    #[test]
    fn renamed_collaborator_listed_only_by_login_is_no_longer_matched() {
        // Documents the rename limitation: "bob" was listed by login, but the same
        // person now logs in as "bob-2" — the entry no longer matches (only a
        // numeric-id listing would have survived).
        let reg = reg(7, &["bob"]);
        assert!(!is_work_author_allowed(&reg, &[], 999, "bob-2"));
    }

    #[test]
    fn empty_admins_and_collaborators_allow_only_the_author() {
        let reg = reg(7, &[]);
        assert!(is_work_author_allowed(&reg, &[], 7, "author-login"));
        assert!(!is_work_author_allowed(&reg, &[], 8, "someone-else"));
    }

    #[test]
    fn blank_collaborator_entries_never_match() {
        // "@" normalizes to empty and a bare "" never grants — junk tokens are inert.
        let reg = reg(7, &["@", "  "]);
        assert!(!is_work_author_allowed(&reg, &[], 999, ""));
    }

    #[test]
    fn collaborator_numeric_entry_matches_id_only_not_login() {
        // A collaborator listed by numeric id is id-only: an unrelated user whose
        // LOGIN is those same digits must NOT be admitted (the entry_matches guard).
        let reg = reg(7, &["4242"]);
        assert!(is_work_author_allowed(&reg, &[], 4242, "some-login"));
        assert!(!is_work_author_allowed(&reg, &[], 999, "4242"));
    }

    // ---- WorkAuthz three-state decision -------------------------------------

    #[test]
    fn authz_off_allows_every_author() {
        // Enforcement off = legacy permissive: even a total stranger is allowed.
        let reg = reg(7, &["bob"]);
        let authz = WorkAuthz::off();
        assert!(!authz.enforce);
        assert!(authz.allows(&reg, 999, "mallory"));
    }

    #[test]
    fn authz_enforcing_with_admins_allows_the_admin_tier() {
        let reg = reg(7, &[]);
        let authz = WorkAuthz::enforcing(vec![admin(500, "octo-admin")]);
        assert!(authz.enforce);
        assert!(authz.allows(&reg, 500, "octo-admin"));
        assert!(!authz.allows(&reg, 999, "mallory"));
    }

    #[test]
    fn authz_enforcing_with_empty_admins_still_rejects_strangers() {
        // The admin-lookup-failed-this-pass state: admins empty, but author +
        // collaborators are STILL enforced and a stranger is STILL rejected (never a
        // full fail-open).
        let reg = reg(7, &["bob"]);
        let authz = WorkAuthz::enforcing(Vec::new());
        assert!(authz.enforce);
        assert!(
            authz.allows(&reg, 7, "author-login"),
            "author still allowed"
        );
        assert!(authz.allows(&reg, 999, "bob"), "collaborator still allowed");
        assert!(
            !authz.allows(&reg, 1234, "mallory"),
            "stranger still rejected when the admin tier is unavailable"
        );
    }
}
