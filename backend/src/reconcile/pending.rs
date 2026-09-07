//! Assignee-scoped spawn/idle gate.
//!
//! Each work label uses a cheap Search count first. Only a positive count pays
//! for issue enumeration, where the exactly-one-assignee routing rule and the
//! always-on author authority predicate are applied to metadata.

use async_trait::async_trait;
use secrecy::SecretString;

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::github_app::listing::GithubListing;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::routing::{route_work_issue, WorkRouting};
use crate::reconcile::work_authz::is_work_author_allowed_with_bot;

#[async_trait]
pub trait PendingWork: Send + Sync {
    /// True when at least one open issue is routed to `reg` through its sole
    /// assignee and its author belongs to an allowed authority tier.
    async fn has_pending(
        &self,
        installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
        reg: &SessionRegistration,
        global_admins: &AccessPolicy,
    ) -> Result<bool, AppError>;
}

/// Production implementation over one already-minted repo installation token.
pub struct LabelCountPending<'a> {
    listing: &'a dyn GithubListing,
    token: &'a SecretString,
    github_bot_login: Option<&'a str>,
}

impl<'a> LabelCountPending<'a> {
    pub fn new(listing: &'a dyn GithubListing, token: &'a SecretString) -> Self {
        Self {
            listing,
            token,
            github_bot_login: None,
        }
    }

    pub fn new_with_bot_login(
        listing: &'a dyn GithubListing,
        token: &'a SecretString,
        github_bot_login: Option<&'a str>,
    ) -> Self {
        Self {
            listing,
            token,
            github_bot_login,
        }
    }
}

#[async_trait]
impl PendingWork for LabelCountPending<'_> {
    async fn has_pending(
        &self,
        _installation_id: i64,
        repo: &RepoRef,
        work_labels: &[String],
        reg: &SessionRegistration,
        global_admins: &AccessPolicy,
    ) -> Result<bool, AppError> {
        for label in work_labels {
            let count = self
                .listing
                .count_open_issues_with_label_assignee(
                    self.token,
                    &repo.owner,
                    &repo.name,
                    label,
                    &reg.creator_login,
                )
                .await?;
            if count == 0 {
                continue;
            }

            let issues = self
                .listing
                .list_issues_by_label_assignee(
                    self.token,
                    &repo.owner,
                    &repo.name,
                    label,
                    &reg.creator_login,
                )
                .await?;
            if issues.iter().map(|issue| issue.metadata()).any(|meta| {
                route_work_issue(&meta, &reg.creator_login) == WorkRouting::Routed
                    && is_work_author_allowed_with_bot(
                        reg,
                        global_admins,
                        meta.user_id,
                        &meta.user_login,
                        self.github_bot_login,
                    )
            }) {
                return Ok(true);
            }
        }
        Ok(false)
    }
}

#[cfg(test)]
#[path = "pending_tests.rs"]
mod tests;
