//! Canvas-specific GitHub calls, added onto [`DashboardGithub`] as a sibling
//! `impl` block: the org-membership role read the overview's owner detection
//! needs, the repo pull-request listing, and the USER-token issue writes the
//! create/stop-session endpoints act with (the human stays the issue author —
//! and thus the session authz owner). Follows the dashboard client's raw-DTO +
//! wiremock-tested pattern (same transport, same error mapping); lives in its
//! own file so neither module grows past the file-size budget.

use secrecy::{ExposeSecret, SecretString};
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

/// The issue created by [`DashboardGithub::create_issue`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CreatedIssue {
    pub number: i64,
    pub html_url: String,
}

/// Pull GitHub's own `message` out of an error response body (it names the
/// real cause: permission missing, issues disabled, validation detail) without
/// leaking anything else; falls back to the bare status.
async fn github_error_message(response: reqwest::Response) -> String {
    let status = response.status();
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("github returned status {status}"))
}

/// Map a failed GitHub issue-write status onto the API error surface. GitHub
/// answers 404 for both "no such repo" and "no access" (deliberate
/// anti-enumeration), so both surface as not-found here.
fn issue_write_error(op: &str, status: reqwest::StatusCode, message: String) -> AppError {
    match status.as_u16() {
        401 => AppError::Unauthorized(format!("github rejected the token: {message}")),
        403 => AppError::Forbidden(format!("GitHub refused {op}: {message}")),
        404 => AppError::NotFound(format!("github {op}: {message}")),
        // 410 Gone = issues are disabled on the repo.
        410 => AppError::Unprocessable(format!("github {op}: {message}")),
        422 => AppError::Validation(format!("GitHub rejected {op}: {message}")),
        _ => AppError::Unavailable(format!("github {op} returned status {status}")),
    }
}

/// One issue as fetched by [`DashboardGithub::get_issue`]: just what the
/// stop-session pre-flight gate needs — the label names, and whether the
/// "issue" is actually a pull request (GitHub's issues API serves PRs too).
#[derive(Debug, Clone)]
pub(crate) struct FetchedIssue {
    pub labels: Vec<String>,
    pub is_pull_request: bool,
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

    /// `POST /repos/{owner}/{repo}/issues` with the USER token — the trigger
    /// issue is created AS the signed-in human, who thereby becomes the
    /// session's authz owner (the reconciler trusts the issue author). The
    /// body/labels are the caller's responsibility (rendered + round-trip
    /// validated before this is ever called).
    pub(crate) async fn create_issue(
        &self,
        user_token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<CreatedIssue, AppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        let response = self
            .client
            .post(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .json(&serde_json::json!({ "title": title, "body": body, "labels": labels }))
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github create-issue transport error");
                AppError::Unavailable("github create-issue request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            #[derive(Deserialize)]
            struct RawCreated {
                number: i64,
                #[serde(default)]
                html_url: String,
            }
            let raw: RawCreated = response.json().await.map_err(|e| {
                tracing::warn!(error = %e, "github create-issue response did not parse");
                AppError::Upstream("github create-issue response was malformed".to_string())
            })?;
            return Ok(CreatedIssue {
                number: raw.number,
                html_url: raw.html_url,
            });
        }
        let message = github_error_message(response).await;
        Err(issue_write_error("create_issue", status, message))
    }

    /// `GET /repos/{owner}/{repo}/issues/{number}` with the USER token — the
    /// stop-session pre-flight read. GitHub answers 404 for both "no such
    /// issue" and "no access" (anti-enumeration), mapped to not-found here.
    pub(crate) async fn get_issue(
        &self,
        user_token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<FetchedIssue, AppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .get(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github get-issue transport error");
                AppError::Unavailable("github get-issue request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            #[derive(Deserialize)]
            struct RawIssueLabel {
                name: String,
            }
            #[derive(Deserialize)]
            struct RawFetchedIssue {
                #[serde(default)]
                labels: Vec<RawIssueLabel>,
                /// Present only when this "issue" is actually a PR.
                pull_request: Option<serde_json::Value>,
            }
            let raw: RawFetchedIssue = response.json().await.map_err(|e| {
                tracing::warn!(error = %e, "github get-issue response did not parse");
                AppError::Upstream("github get-issue response was malformed".to_string())
            })?;
            return Ok(FetchedIssue {
                labels: raw.labels.into_iter().map(|label| label.name).collect(),
                is_pull_request: raw.pull_request.is_some(),
            });
        }
        let message = github_error_message(response).await;
        Err(issue_write_error("get_issue", status, message))
    }

    /// `PATCH /repos/{owner}/{repo}/issues/{number}` `state=closed` with the
    /// USER token — closing the trigger issue IS the stop/retire contract, and
    /// GitHub natively enforces whether THIS caller may close it.
    pub(crate) async fn close_issue(
        &self,
        user_token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<(), AppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .patch(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .json(&serde_json::json!({ "state": "closed" }))
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github close-issue transport error");
                AppError::Unavailable("github close-issue request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let message = github_error_message(response).await;
        Err(issue_write_error("close_issue", status, message))
    }
}

#[cfg(test)]
#[path = "github_tests.rs"]
mod tests;
