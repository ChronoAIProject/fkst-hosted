//! Shared canvas DTOs: the issue projection both the sessions endpoint's
//! trigger and work-issue lists render.

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::goals::trigger_parse::PackageRef;
use crate::routes::dashboard::IssueWithMeta;

/// Render a parsed package reference back to its canonical
/// `owner/repo@ref:path` form (the exact grammar the trigger parser accepts).
pub(super) fn render_package_ref(package: &PackageRef) -> String {
    format!(
        "{}/{}@{}:{}",
        package.owner, package.repo, package.git_ref, package.path
    )
}

/// A GitHub issue as the canvas renders it: the trimmed dashboard view plus the
/// link + ISO-8601 timestamps the level-2 detail panel shows.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct IssueDetail {
    pub number: i64,
    pub title: String,
    /// `open` or `closed`.
    pub state: String,
    /// The issue author's GitHub login.
    pub author: String,
    /// Label NAMES on the issue.
    pub labels: Vec<String>,
    /// The issue's github.com URL.
    pub html_url: String,
    /// ISO-8601 creation time.
    pub created_at: String,
    /// ISO-8601 last-update time.
    pub updated_at: String,
    /// ISO-8601 close time; null while the issue is open.
    pub closed_at: Option<String>,
}

impl From<&IssueWithMeta> for IssueDetail {
    fn from(issue: &IssueWithMeta) -> Self {
        IssueDetail {
            number: issue.summary.number,
            title: issue.summary.title.clone(),
            state: issue.summary.state.clone(),
            author: issue.summary.user_login.clone(),
            labels: issue.summary.labels.clone(),
            html_url: issue.html_url.clone(),
            created_at: issue.created_at.clone(),
            updated_at: issue.updated_at.clone(),
            closed_at: issue.closed_at.clone(),
        }
    }
}

#[cfg(test)]
#[path = "types_tests.rs"]
mod tests;
