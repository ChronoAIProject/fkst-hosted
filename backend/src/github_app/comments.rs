//! Author-and-timestamp-aware issue-comment reads.
//!
//! `GithubApi::list_issue_comments` returns BODIES only, which is all the
//! config-immutability check ever needed: it looks for one marker written by the
//! control plane on an issue the control plane owns.
//!
//! The schedule pass needs more. A run record is durable state recovered from a
//! comment on an issue any repository collaborator can comment on, so it must be
//! able to say WHO wrote a marker — a hand-written `fkst-cron-run:v1` comment
//! claiming a slot completed would otherwise let anyone silence a schedule or
//! forge its history. It also needs comment timestamps for the run projection.
//!
//! ## Why this reads the NEWEST comments rather than the first page
//!
//! GitHub returns issue comments oldest-first. A schedule firing hourly accrues
//! two records an hour, so a single `per_page=100` page would, within days, show
//! only the ORIGINAL comments and none of the recent run records — the pass would
//! recover an empty cursor and re-fire the anchor slot forever. This walks the
//! `Link` header to the last page and reads backwards from there, which is both
//! correct and usually one request.

use async_trait::async_trait;
use k8s_openapi::chrono::{DateTime, Utc};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::api::{is_rate_limited, reset_seconds};
use super::{GithubAppError, GithubAppTokens};

/// Request timeout, mirroring the sibling transports.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Comments per page. GitHub's maximum, so the common case is one request.
const PER_PAGE: u32 = 100;

/// One issue comment with the provenance the schedule pass authorizes against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IssueComment {
    pub body: String,
    /// The comment author's login. Only the configured App identity is trusted as
    /// a run-record writer.
    pub user_login: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Deserialize)]
struct RawComment {
    #[serde(default)]
    body: String,
    user: Option<RawUser>,
    created_at: Option<DateTime<Utc>>,
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
}

/// Read-side transport for comment provenance. Injected so the schedule pass is
/// unit-testable against a fake, mirroring [`super::listing::GithubListing`].
#[async_trait]
pub trait IssueCommentReader: Send + Sync {
    /// The most recent comments on `number`, oldest-first within the window,
    /// bounded to the last `max_pages` pages.
    ///
    /// A 404 yields an empty list: a vanished issue has no history, and failing
    /// the whole repository pass over one deleted issue would be a worse trade.
    async fn list_recent_issue_comments(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<IssueComment>, GithubAppError>;
}

/// Production HTTP transport.
pub struct HttpIssueCommentReader {
    api_base: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpIssueCommentReader {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpIssueCommentReader")
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl HttpIssueCommentReader {
    pub fn new(api_base: &str) -> Result<Self, GithubAppError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| GithubAppError::Http(format!("comment reader client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Fetch one page, returning its comments plus the last-page number the `Link`
    /// header advertises (present only on the first request of a paginated issue).
    async fn page(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        page: u32,
    ) -> Result<(Vec<IssueComment>, Option<u32>), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/comments",
            self.api_base
        );
        let response = self
            .client
            .get(&url)
            .query(&[
                ("per_page", PER_PAGE.to_string()),
                ("page", page.to_string()),
            ])
            .header("accept", "application/vnd.github+json")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_recent_issue_comments: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok((Vec::new(), None));
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            return Err(if is_rate_limited(response.headers()) {
                GithubAppError::RateLimited(reset_seconds(response.headers()))
            } else {
                GithubAppError::AppAuth
            });
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "list_recent_issue_comments status {status}: {body}"
            )));
        }

        let last = last_page(response.headers());
        let raw: Vec<RawComment> = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_recent_issue_comments body: {e}")))?;
        let comments = raw
            .into_iter()
            .map(|comment| IssueComment {
                body: comment.body,
                user_login: comment.user.map(|user| user.login).unwrap_or_default(),
                // An absent timestamp degrades to the epoch: it only orders the
                // projection's display, never a scheduling decision (those read the
                // slot out of the marker itself).
                created_at: comment.created_at.unwrap_or(DateTime::UNIX_EPOCH),
            })
            .collect();
        Ok((comments, last))
    }
}

#[async_trait]
impl IssueCommentReader for HttpIssueCommentReader {
    async fn list_recent_issue_comments(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<IssueComment>, GithubAppError> {
        let max_pages = max_pages.max(1);
        let (first, last) = self.page(token, owner, repo, number, 1).await?;
        // One page of comments: the common case, and already complete.
        let Some(last) = last.filter(|last| *last > 1) else {
            return Ok(first);
        };

        let start = last.saturating_sub(max_pages - 1).max(1);
        let mut out = if start == 1 { first } else { Vec::new() };
        for page in start.max(2)..=last {
            let (comments, _) = self.page(token, owner, repo, number, page).await?;
            out.extend(comments);
        }
        Ok(out)
    }
}

/// The `page=N` of the `rel="last"` link, if GitHub paginated the response.
fn last_page(headers: &reqwest::header::HeaderMap) -> Option<u32> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in link.split(',') {
        let segments: Vec<&str> = part.split(';').map(str::trim).collect();
        if !segments.contains(&"rel=\"last\"") {
            continue;
        }
        let url = segments
            .first()?
            .trim_start_matches('<')
            .trim_end_matches('>');
        let page = url
            .split(['?', '&'])
            .find_map(|param| param.strip_prefix("page="))?;
        return page.parse().ok();
    }
    None
}

impl GithubAppTokens {
    /// Mint an installation token for `owner_repo` and read its issue's most
    /// recent comments with provenance.
    pub async fn list_recent_issue_comments(
        &self,
        reader: &dyn IssueCommentReader,
        owner_repo: &str,
        number: u64,
        max_pages: u32,
    ) -> Result<Vec<IssueComment>, GithubAppError> {
        let (owner, repo) = owner_repo
            .split_once('/')
            .ok_or(GithubAppError::InvalidRepoRef)?;
        let token = self.token_for_repo(owner_repo, None).await?;
        reader
            .list_recent_issue_comments(&token, owner, repo, number, max_pages)
            .await
    }
}

#[cfg(test)]
#[path = "comments_tests.rs"]
mod tests;
