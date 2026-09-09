//! The authorization facts one session carries, and the borrowed view every
//! capability decision is made from.
//!
//! [`SessionAccessContext`] is the registry's value type: a projection of the
//! trigger issue's *already parsed* authorization metadata. It deliberately holds
//! no issue body, no title, no comment text, no environment content, no runtime
//! response, and no credential — only the correlation keys plus the three
//! identity lists a decision needs.
//!
//! [`SessionAuthorizationFacts`] is the borrowed shape the pure policy consumes.
//! It exists so the reconciler's live
//! [`SessionRegistration`](crate::reconcile::desired::SessionRegistration) and the
//! registry's stored context can feed ONE decision function without either side
//! allocating a copy of the other's representation — which is what keeps work
//! authority and operations visibility from drifting apart.

use crate::models::RepoRef;
use crate::reconcile::creator::SessionCreator;

/// Everything a session-scoped authorization decision may consider.
///
/// Correlation fields (`installation_id`, `repo`, `trigger_issue`) are display
/// and traceability data. They are NOT authorization inputs: repository
/// visibility, repository role, and trigger readability are deliberately not
/// tiers (epic `AUTH-04`).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAccessContext {
    /// The GitHub App installation the session belongs to.
    pub installation_id: i64,
    /// The `owner/name` repository the session works.
    pub repo: RepoRef,
    /// The trigger issue the session was launched from.
    pub trigger_issue: i64,
    /// The effective human creator. A human-authored trigger carries the
    /// author's immutable id; an App-authored trigger carries only the sole
    /// assignee's login, and the missing id is preserved as `None` rather than
    /// backfilled from the trigger author.
    pub creator: SessionCreator,
    /// The frozen `### Session Collaborators` entries (logins or numeric ids).
    pub collaborators: Vec<String>,
    /// The frozen `### FKST Contributors` / `### Log Access Allowlist` entries.
    pub log_access: Vec<String>,
}

impl SessionAccessContext {
    /// The borrowed view the pure policy evaluates.
    pub fn facts(&self) -> SessionAuthorizationFacts<'_> {
        SessionAuthorizationFacts {
            creator_id: self.creator.id,
            creator_login: &self.creator.login,
            collaborators: &self.collaborators,
            log_access: &self.log_access,
        }
    }

    /// Whether this context belongs to `(installation_id, repo)`.
    ///
    /// Used by the per-repository replacement path: a successful sweep of one
    /// repository must drop exactly that repository's retired sessions and touch
    /// no other repository's entries.
    pub(crate) fn belongs_to(&self, installation_id: i64, repo: &RepoRef) -> bool {
        self.installation_id == installation_id && &self.repo == repo
    }
}

/// The borrowed authorization facts of one session.
///
/// Lifetimes rather than owned data: this is constructed per decision, on the
/// reconcile hot path as well as the request path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionAuthorizationFacts<'a> {
    /// The effective creator's immutable id, or `None` for an assignee-derived
    /// creator whose id GitHub's issue metadata never exposed.
    pub creator_id: Option<i64>,
    /// The effective creator's login. Only consulted when `creator_id` is `None`.
    pub creator_login: &'a str,
    /// `### Session Collaborators` entries.
    pub collaborators: &'a [String],
    /// `### FKST Contributors` / `### Log Access Allowlist` entries.
    pub log_access: &'a [String],
}

impl<'a> SessionAuthorizationFacts<'a> {
    /// Whether the verified caller is the effective creator.
    ///
    /// Strictly id-first: when the session recorded an immutable creator id, only
    /// that id authorizes — a stale login snapshot belonging to a *different*
    /// account must never inherit the session. The login fallback exists solely
    /// for assignee-derived sessions, where GitHub gave us no id to compare.
    pub fn creator_matches(&self, caller_id: i64, caller_login: &str) -> bool {
        match self.creator_id {
            Some(creator_id) => caller_id == creator_id,
            None => {
                !self.creator_login.trim().is_empty()
                    && caller_login.eq_ignore_ascii_case(self.creator_login)
            }
        }
    }
}

#[cfg(test)]
#[path = "context_tests.rs"]
mod tests;
