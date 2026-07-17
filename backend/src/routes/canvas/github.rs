//! Canvas-specific GitHub calls, added onto [`DashboardGithub`] as a sibling
//! `impl` block: the org-membership role read the overview's owner detection
//! needs. Follows the dashboard client's raw-DTO + paged `get_page` pattern
//! (same transport, same error mapping); lives in its own file so neither
//! module grows past the file-size budget.

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;
use crate::routes::dashboard::DashboardGithub;

#[derive(Deserialize)]
struct RawOrgLogin {
    login: String,
}

/// One element of the bare-array `GET /user/memberships/orgs` response.
#[derive(Deserialize)]
struct RawMembership {
    /// `admin` (org owner) or `member`.
    #[serde(default)]
    role: String,
    organization: RawOrgLogin,
}

/// The caller's active membership in one organization: the org login plus the
/// caller's role (`admin` = org owner; anything else = plain member).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct OrgMembership {
    pub org: String,
    pub role: String,
}

#[derive(Deserialize)]
struct RawPullUser {
    #[serde(default)]
    login: String,
}

#[derive(Deserialize)]
struct RawPullHead {
    /// The head BRANCH name (`ref` is a Rust keyword).
    #[serde(rename = "ref", default)]
    head_ref: String,
}

/// One element of the bare-array `GET /repos/{owner}/{repo}/pulls` response.
#[derive(Deserialize)]
struct RawPull {
    number: i64,
    #[serde(default)]
    title: String,
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    state: String,
    /// Set only once the PR merged (the list endpoint has no `merged` bool).
    #[serde(default)]
    merged_at: Option<String>,
    user: Option<RawPullUser>,
    head: Option<RawPullHead>,
}

/// One pull request as the canvas sessions endpoint consumes it.
#[derive(Debug, Clone)]
pub(crate) struct RepoPull {
    pub number: i64,
    pub title: String,
    pub html_url: String,
    /// `open` or `closed`.
    pub state: String,
    /// Merged (derived from `merged_at`); a closed-unmerged PR stays false.
    pub merged: bool,
    /// The PR author's login (the devloop-bot filter keys on it).
    pub author: String,
    /// The head branch name (the devloop issue-number parse keys on it).
    pub head_ref: String,
}

impl DashboardGithub {
    /// `GET /user/memberships/orgs?state=active` (user token), paginated — the
    /// caller's ACTIVE org memberships with their role. The overview uses
    /// `role == "admin"` as the org-owner signal; `state=active` excludes
    /// pending invitations (an invitee is not an owner of anything yet).
    pub(crate) async fn user_org_memberships(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<OrgMembership>, AppError> {
        let mut url = format!("{}/user/memberships/orgs", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> =
            Some(vec![("state", "active"), ("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawMembership>, _) = self
                .get_page(&url, user_token, query.as_deref(), "user_org_memberships")
                .await?;
            out.extend(page.into_iter().map(|m| OrgMembership {
                org: m.organization.login,
                role: m.role,
            }));
            match next {
                Some(next_url) => {
                    url = next_url;
                    query = None;
                }
                None => return Ok(out),
            }
        }
    }

    /// `GET /repos/{owner}/{repo}/pulls?state=all` — the repo's newest 100 pull
    /// requests (ONE page, deliberately unpaginated: the canvas links recent
    /// devloop PRs, not the repo's full history). Works with either a user or
    /// an installation token.
    pub(crate) async fn list_pulls_all(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<RepoPull>, AppError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.api_base);
        let query: &[(&str, &str)] = &[
            ("state", "all"),
            ("per_page", "100"),
            ("sort", "created"),
            ("direction", "desc"),
        ];
        let (page, _next): (Vec<RawPull>, _) = self
            .get_page(&url, token, Some(query), "list_pulls_all")
            .await?;
        Ok(page
            .into_iter()
            .map(|raw| RepoPull {
                number: raw.number,
                title: raw.title,
                html_url: raw.html_url,
                state: raw.state,
                merged: raw.merged_at.is_some(),
                author: raw.user.map(|u| u.login).unwrap_or_default(),
                head_ref: raw.head.map(|h| h.head_ref).unwrap_or_default(),
            })
            .collect())
    }
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
