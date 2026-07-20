//! The spawn/idle gate: "does this session still have pending work?" (issue #359
//! §4.2, PR5b).
//!
//! The pure planner ([`crate::reconcile::desired::plan_repo`]) spawns a session
//! only while it reports itself PENDING, and idle-kills it once it stops. That
//! "pending?" signal is the count of open issues carrying the session's work
//! label: `> 0` means there is work to claim. This module is the injectable seam
//! for that check so the per-repo driver + its tests can swap a fake in for the
//! live GitHub Search call.
//!
//! Secret hygiene: the installation token is held by reference and never logged.

use async_trait::async_trait;
use secrecy::SecretString;

use crate::error::AppError;
use crate::github_app::listing::GithubListing;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::work_authz::is_work_author_allowed;

/// The spawn/idle gate. Returns whether a session's work label still has open
/// issues to claim (its "pending" signal). Injected so the driver is unit-testable
/// against a fake, mirroring [`crate::github_app::listing::GithubListing`].
#[async_trait]
pub trait PendingWork: Send + Sync {
    /// True when at least one OPEN issue in `repo` carries ANY of `work_labels` —
    /// the session has claimable work and is "pending". The set is the trigger's
    /// explicit work label plus every label its packages auto-declare
    /// ([`crate::reconcile::work_labels`]); an empty set is never pending.
    /// `installation_id` identifies the installation the caller minted its token
    /// from (an impl backed by a pre-scoped token ignores it).
    async fn has_pending(
        &self,
        installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
    ) -> Result<bool, AppError>;

    /// Author-FILTERED pending gate (R3 authority gate, epic #572). Like
    /// [`has_pending`](Self::has_pending), but a session is pending only while at
    /// least one OPEN work-label issue was raised by an author AUTHORIZED to raise
    /// work for it (the session's author ∪ Session Collaborators ∪ repo `admins`,
    /// per [`is_work_author_allowed`]). Because GitHub Search cannot filter by
    /// author, this ENUMERATES the label's open issues (not a blind count) so each
    /// author can be checked; an empty label set is never pending. Used only when
    /// the operator opts into enforcement — the driver keeps calling the cheap
    /// blind [`has_pending`](Self::has_pending) when the flag is off.
    async fn has_pending_authorized(
        &self,
        installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
        reg: &SessionRegistration,
        admins: &[GithubActor],
    ) -> Result<bool, AppError>;
}

/// Production gate: a session is pending iff its work label has `> 0` open issues,
/// counted in ONE Search API call ([`GithubListing::count_open_issues_with_label`]).
///
/// Holds the repo-scoped installation `token` + the [`GithubListing`] transport by
/// reference: the driver mints one token per repo, lists the trigger issues, then
/// reuses that same token to gate each of the repo's registrations, so no per-check
/// re-mint is needed.
pub struct LabelCountPending<'a> {
    listing: &'a dyn GithubListing,
    token: &'a SecretString,
}

impl<'a> LabelCountPending<'a> {
    /// Build the gate over a repo-scoped installation `token` + a listing transport.
    pub fn new(listing: &'a dyn GithubListing, token: &'a SecretString) -> Self {
        Self { listing, token }
    }
}

#[async_trait]
impl PendingWork for LabelCountPending<'_> {
    async fn has_pending(
        &self,
        _installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
    ) -> Result<bool, AppError> {
        // The token already encodes the installation, so `installation_id` is
        // unused here; it stays on the trait for impls that mint per-check.
        // GitHub Search ANDs multiple `label:` qualifiers, so a single OR query
        // isn't available — count each label (cheap, `per_page=1`) and
        // short-circuit on the first hit.
        for label in work_labels {
            let count = self
                .listing
                .count_open_issues_with_label(self.token, &repo.owner, &repo.name, label)
                .await?;
            if count > 0 {
                return Ok(true);
            }
        }
        Ok(false)
    }

    async fn has_pending_authorized(
        &self,
        _installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
        reg: &SessionRegistration,
        admins: &[GithubActor],
    ) -> Result<bool, AppError> {
        // Enumerate rather than count: the authorization decision needs each open
        // issue's AUTHOR, which the Search count does not expose. Short-circuit on
        // the first issue by an authorized author (labels OR'd, like `has_pending`).
        for label in work_labels {
            let issues = self
                .listing
                .list_issues_by_label(self.token, &repo.owner, &repo.name, label)
                .await?;
            if issues
                .iter()
                .any(|issue| is_work_author_allowed(reg, admins, issue.user_id, &issue.user_login))
            {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
