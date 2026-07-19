//! GitHub API transport: the `GithubApi` trait and its `HttpGithubApi` implementation.
//!
//! Two endpoints are used:
//!   1. `GET {base}/repos/{owner}/{repo}/installation` -- resolve the installation
//!      covering a repo (404 = not installed).
//!   2. `POST {base}/app/installations/{id}/access_tokens` -- mint a 1-hour
//!      installation token scoped to specific repos and a permissions subset.
//!
//! HTTP client patterns mirror `src/journal/github.rs`: injected `api_base`,
//! 20s timeout, user-agent `fkst-hosted-api`, rate-limit / auth disambiguation.

use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Serialize;

use super::GithubAppError;

/// Request timeout for every GitHub API call.
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// Hard page cap for [`GithubApi::list_pull_files`] (100 files/page → ~300
/// files). The canvas outcomes surface lists a PR's changed files, not an
/// unbounded mega-PR, so a fixed cap bounds the fan-out.
const MAX_PULL_FILE_PAGES: u32 = 3;

/// Opaque installation ID resolved from the GitHub API.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct InstallationId(pub u64);

/// Permission subset requested for an installation token. Values are "read" or
/// "write". Omitted fields mean "no permission requested".
#[derive(Debug, Clone, Default, PartialEq, Eq, Hash, Serialize)]
pub struct TokenPermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub contents: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issues: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pull_requests: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administration: Option<String>,
}

/// Token creation request body.
#[derive(Serialize)]
pub struct InstallationTokenRequest {
    /// Bare repo names (NOT `owner/name`).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub repositories: Vec<String>,
    /// Permission subset; `None` requests the installation's default.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permissions: Option<TokenPermissions>,
}

/// A minted installation token with its expiry.
pub struct InstallationToken {
    pub token: SecretString,
    pub expires_at: SystemTime,
}

/// A single file read from the Contents API: its blob SHA (required to UPDATE it
/// via `PUT /contents`) and its raw base64 `content` (GitHub returns base64 with
/// embedded newlines — decode after stripping whitespace).
#[derive(Debug, Clone)]
pub struct RemoteFile {
    pub sha: String,
    pub content_base64: String,
}

/// One OPEN pull request, trimmed to what the auto-merge step needs.
#[derive(Debug, Clone)]
pub struct PullRequestSummary {
    pub number: u64,
    /// The PR author's login (matched against the configured bot login).
    pub author_login: String,
    pub head_sha: String,
    /// The PR's head branch name (GitHub `head.ref`). The devloop bot encodes the
    /// work-issue number in it (`devloop/issue/<owner>/<repo>/<N>/…`), so the
    /// auto-merge step parses it to close the linked issue after a merge.
    pub head_ref: String,
    /// The PR title (GitHub `title`). A fallback source for the work-issue number
    /// (`… for #<N>` / `… for issue #<N>`) when the branch name does not carry it.
    pub title: String,
}

/// One changed file of a pull request (`GET /repos/{o}/{r}/pulls/{n}/files`),
/// trimmed to what the canvas outcomes surface renders. The `sha` is the
/// file's BLOB sha at the PR head — the handle the blob-stream endpoint reads
/// bytes with. `previous_filename` is set only for a rename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PullFileMeta {
    pub filename: String,
    /// GitHub's file status: `added`/`modified`/`removed`/`renamed`/`copied`/`changed`.
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    pub changes: i64,
    /// The file's blob sha at the PR head.
    pub sha: String,
    /// The prior path, present only when `status == "renamed"`.
    pub previous_filename: Option<String>,
}

// Hand-written: the token must never appear in Debug.
impl fmt::Debug for InstallationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InstallationToken")
            .field("token", &"<redacted>")
            .field("expires_at", &self.expires_at)
            .finish()
    }
}

/// Abstract GitHub API transport. `HttpGithubApi` is the production impl;
/// tests inject a fake.
#[async_trait]
pub trait GithubApi: Send + Sync {
    /// `GET {base}/repos/{owner}/{repo}/installation` with Bearer = app JWT.
    /// 404 -> [`GithubAppError::NotInstalled`].
    async fn installation_for_repo(
        &self,
        app_jwt: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<InstallationId, GithubAppError>;

    /// `POST {base}/app/installations/{id}/access_tokens`.
    /// 404 -> [`GithubAppError::InstallationGone`].
    /// 422 -> [`GithubAppError::TokenRequestRejected`].
    async fn create_installation_token(
        &self,
        app_jwt: &SecretString,
        id: InstallationId,
        req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError>;

    /// Post an issue comment with an installation `token`. Only the HTTP
    /// transport implements this; fakes inherit the default.
    async fn create_issue_comment(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, number, body);
        unimplemented!("create_issue_comment is only implemented by the HTTP transport")
    }

    /// Create a new issue (`POST /repos/{o}/{r}/issues`) with `title`, `body`, and
    /// `labels`; returns its number. Default panics (only the HTTP transport
    /// implements it).
    async fn create_issue(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, GithubAppError> {
        let _ = (token, owner, repo, title, body, labels);
        unimplemented!("create_issue is only implemented by the HTTP transport")
    }

    /// The numbers of OPEN issues carrying `label` (the seed-issue idempotency
    /// probe; excludes pull requests). Default panics.
    async fn open_issues_with_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<u64>, GithubAppError> {
        let _ = (token, owner, repo, label);
        unimplemented!("open_issues_with_label is only implemented by the HTTP transport")
    }

    /// Add labels to an issue (additive; preserves existing). Default panics.
    async fn add_issue_labels(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, number, labels);
        unimplemented!("add_issue_labels is only implemented by the HTTP transport")
    }

    /// Remove ONE label from an issue (404-tolerant). Default panics.
    async fn remove_issue_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, number, label);
        unimplemented!("remove_issue_label is only implemented by the HTTP transport")
    }

    /// `PATCH {base}/repos/{owner}/{repo}/issues/{number}` with `{"state":"closed"}`
    /// closing the issue (needs `issues:write`). Used to complete an auto-merge by
    /// closing the merged PR's linked work issue. Default panics.
    async fn close_issue(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, number);
        unimplemented!("close_issue is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/issues/{number}` → the issue's current
    /// label NAMES. Used by the session-health scrape to dedupe its degraded flag
    /// (only post a comment on the FIRST transition). A 404 (issue gone) yields an
    /// empty set. Default panics.
    async fn get_issue_labels(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        let _ = (token, owner, repo, number);
        unimplemented!("get_issue_labels is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/issues/{number}/comments?per_page=100` →
    /// each comment's raw markdown BODY (author order). Used by the config-immutability
    /// check to recover the original `full_config_hash` latched (as a hidden marker) in
    /// the one-time session-announcement comment. A 404 (issue gone) yields an empty
    /// list. Default panics.
    async fn list_issue_comments(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        let _ = (token, owner, repo, number);
        unimplemented!("list_issue_comments is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/contents/{path}` (optionally `?ref=…`)
    /// returning the file's blob SHA + base64 content. A 404 yields `Ok(None)`
    /// (missing file — installed template v0 / the CREATE path). Default panics.
    async fn content_file(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: Option<&str>,
    ) -> Result<Option<RemoteFile>, GithubAppError> {
        let _ = (token, owner, repo, path, git_ref);
        unimplemented!("content_file is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}` -> the repo's `default_branch`. Default panics.
    async fn repo_default_branch(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<String, GithubAppError> {
        let _ = (token, owner, repo);
        unimplemented!("repo_default_branch is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/git/ref/heads/{branch}` -> the branch
    /// head commit SHA. Default panics.
    async fn branch_head_sha(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<String, GithubAppError> {
        let _ = (token, owner, repo, branch);
        unimplemented!("branch_head_sha is only implemented by the HTTP transport")
    }

    /// `POST {base}/repos/{owner}/{repo}/git/refs` creating `refs/heads/{branch}`
    /// at `sha`. A 422 (ref already exists) maps to
    /// [`GithubAppError::RefExists`]. Default panics.
    async fn create_ref(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, branch, sha);
        unimplemented!("create_ref is only implemented by the HTTP transport")
    }

    /// `PUT {base}/repos/{owner}/{repo}/contents/{path}` creating or updating a
    /// file on `branch`. `sha` is the existing blob SHA for an UPDATE and is
    /// omitted for a CREATE. Default panics.
    #[allow(clippy::too_many_arguments)]
    async fn put_file(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        path: &str,
        message: &str,
        content_base64: &str,
        branch: &str,
        sha: Option<&str>,
    ) -> Result<(), GithubAppError> {
        let _ = (
            token,
            owner,
            repo,
            path,
            message,
            content_base64,
            branch,
            sha,
        );
        unimplemented!("put_file is only implemented by the HTTP transport")
    }

    /// `POST {base}/repos/{owner}/{repo}/pulls` opening a PR, returning its
    /// number. Default panics.
    #[allow(clippy::too_many_arguments)]
    async fn create_pull_request(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> Result<u64, GithubAppError> {
        let _ = (token, owner, repo, title, head, base, body);
        unimplemented!("create_pull_request is only implemented by the HTTP transport")
    }

    /// `PUT {base}/repos/{owner}/{repo}/pulls/{number}/merge` merging a PR.
    /// Default panics.
    async fn merge_pull_request(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        commit_title: &str,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, number, commit_title);
        unimplemented!("merge_pull_request is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/pulls?state=open&per_page=100` → the open
    /// PRs (number + author login + head sha). Single page (v1). Default panics.
    async fn list_open_pulls(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<PullRequestSummary>, GithubAppError> {
        let _ = (token, owner, repo);
        unimplemented!("list_open_pulls is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/pulls/{number}` → GitHub's `mergeable`
    /// tri-state: `Some(true)` mergeable, `Some(false)` conflict, `None` not yet
    /// computed (JSON `null`/absent → retry next reconcile). Default panics.
    async fn pull_request_mergeable(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Option<bool>, GithubAppError> {
        let _ = (token, owner, repo, number);
        unimplemented!("pull_request_mergeable is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/pulls/{number}/files?per_page=100` — the
    /// PR's changed files, paginated up to [`MAX_PULL_FILE_PAGES`] pages (a hard
    /// ~300-file cap: the canvas outcomes surface lists a PR's files, not an
    /// unbounded mega-PR). Default panics. Works with any bearer token.
    async fn list_pull_files(
        &self,
        installation_token: &str,
        owner: &str,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<PullFileMeta>, GithubAppError> {
        let _ = (installation_token, owner, repo, pull_number);
        unimplemented!("list_pull_files is only implemented by the HTTP transport")
    }

    /// `GET {base}/repos/{owner}/{repo}/git/blobs/{sha}` with
    /// `Accept: application/vnd.github.raw` — the file's RAW bytes. Capped to
    /// `max_bytes`: a blob larger than that yields [`GithubAppError::BlobTooLarge`]
    /// (the caller renders "too large, open on GitHub") rather than buffering it.
    /// Default panics.
    async fn get_blob_raw(
        &self,
        installation_token: &str,
        owner: &str,
        repo: &str,
        blob_sha: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, GithubAppError> {
        let _ = (installation_token, owner, repo, blob_sha, max_bytes);
        unimplemented!("get_blob_raw is only implemented by the HTTP transport")
    }

    /// `DELETE {base}/repos/{owner}/{repo}/git/refs/heads/{branch}` deleting a
    /// branch (404/422 tolerated). Default panics.
    async fn delete_ref(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<(), GithubAppError> {
        let _ = (token, owner, repo, branch);
        unimplemented!("delete_ref is only implemented by the HTTP transport")
    }
}

/// Production HTTP transport backed by reqwest.
pub struct HttpGithubApi {
    api_base: String,
    client: reqwest::Client,
}

impl fmt::Debug for HttpGithubApi {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HttpGithubApi")
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl HttpGithubApi {
    pub fn new(api_base: &str) -> Result<Self, GithubAppError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| GithubAppError::Http(format!("client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }
}

/// Seconds until the rate-limit reset, from `retry-after` or
/// `x-ratelimit-reset`. Defaults to 60s when unparseable.
///
/// `pub(super)` so the Contents READ helper (#179) reuses the same rate-limit
/// classification as the token/installation transport.
pub(super) fn reset_seconds(headers: &reqwest::header::HeaderMap) -> u64 {
    if let Some(retry_after) = headers
        .get("retry-after")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        return retry_after;
    }
    if let Some(reset_epoch) = headers
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
    {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        return reset_epoch.saturating_sub(now);
    }
    60
}

/// True when a 403 carries rate-limit evidence.
///
/// `pub(super)` so the Contents READ helper (#179) shares the same 403
/// disambiguation (rate-limit vs auth failure).
pub(super) fn is_rate_limited(headers: &reqwest::header::HeaderMap) -> bool {
    let remaining_zero = headers
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim() == "0")
        .unwrap_or(false);
    remaining_zero || headers.contains_key("retry-after")
}

/// Classify a 401/403 into the shared auth/rate-limit error, mirroring the
/// disambiguation the token/installation transport uses. Returns `Some(err)` for
/// 401 (auth) and 403 (rate-limit when the headers say so, else auth); `None`
/// otherwise so the caller can continue its own status handling. Used by the
/// template-reconcile write methods so they share ONE classification.
fn classify_auth_status(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
) -> Option<GithubAppError> {
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Some(GithubAppError::AppAuth);
    }
    if status == reqwest::StatusCode::FORBIDDEN {
        if is_rate_limited(headers) {
            return Some(GithubAppError::RateLimited(reset_seconds(headers)));
        }
        return Some(GithubAppError::AppAuth);
    }
    None
}

#[async_trait]
impl GithubApi for HttpGithubApi {
    async fn installation_for_repo(
        &self,
        app_jwt: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<InstallationId, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/installation", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(app_jwt.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("installation_for_repo: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::NotInstalled {
                owner_repo: format!("{owner}/{repo}"),
                install_url: None,
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            if is_rate_limited(response.headers()) {
                return Err(GithubAppError::RateLimited(reset_seconds(
                    response.headers(),
                )));
            }
            return Err(GithubAppError::AppAuth);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "installation_for_repo status {status}: {body}"
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("installation_for_repo body: {e}")))?;
        let id = body["id"]
            .as_u64()
            .ok_or_else(|| GithubAppError::Http("installation_for_repo: missing id".to_string()))?;
        Ok(InstallationId(id))
    }

    async fn create_installation_token(
        &self,
        app_jwt: &SecretString,
        id: InstallationId,
        req: &InstallationTokenRequest,
    ) -> Result<InstallationToken, GithubAppError> {
        let url = format!("{}/app/installations/{}/access_tokens", self.api_base, id.0);
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(app_jwt.expose_secret())
            .json(req)
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_installation_token: {e}")))?;

        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::InstallationGone {
                owner_repo: String::new(),
            });
        }
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::FORBIDDEN {
            if is_rate_limited(response.headers()) {
                return Err(GithubAppError::RateLimited(reset_seconds(
                    response.headers(),
                )));
            }
            return Err(GithubAppError::AppAuth);
        }
        if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::TokenRequestRejected(body));
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_installation_token status {status}: {body}"
            )));
        }

        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_installation_token body: {e}")))?;

        let token_str = body["token"]
            .as_str()
            .ok_or_else(|| {
                GithubAppError::Http("create_installation_token: missing token".to_string())
            })?
            .to_string();

        let expires_str = body["expires_at"].as_str().ok_or_else(|| {
            GithubAppError::Http("create_installation_token: missing expires_at".to_string())
        })?;

        let expires_dt = bson::DateTime::parse_rfc3339_str(expires_str).map_err(|e| {
            GithubAppError::Http(format!("create_installation_token: bad expires_at: {e}"))
        })?;

        let expires_at = SystemTime::UNIX_EPOCH
            + std::time::Duration::from_millis(expires_dt.timestamp_millis() as u64);

        Ok(InstallationToken {
            token: SecretString::from(token_str),
            expires_at,
        })
    }

    async fn create_issue_comment(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        body: &str,
    ) -> Result<(), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/comments",
            self.api_base
        );
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({ "body": body }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_issue_comment: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_issue_comment status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn create_issue(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        body: &str,
        labels: &[String],
    ) -> Result<u64, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({ "title": title, "body": body, "labels": labels }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_issue: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_issue status {status}: {body}"
            )));
        }
        let value: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_issue parse: {e}")))?;
        value.get("number").and_then(|n| n.as_u64()).ok_or_else(|| {
            GithubAppError::Http("create_issue: response missing number".to_string())
        })
    }

    async fn open_issues_with_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<u64>, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .query(&[("state", "open"), ("labels", label), ("per_page", "100")])
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("open_issues_with_label: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "open_issues_with_label status {status}: {body}"
            )));
        }
        let items: Vec<serde_json::Value> = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("open_issues_with_label parse: {e}")))?;
        // The issues endpoint also returns PRs (they carry a `pull_request` object);
        // exclude them so a PR never masquerades as an existing trigger issue.
        Ok(items
            .iter()
            .filter(|it| it.get("pull_request").is_none())
            .filter_map(|it| it.get("number").and_then(|n| n.as_u64()))
            .collect())
    }

    async fn add_issue_labels(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        labels: &[String],
    ) -> Result<(), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/labels",
            self.api_base
        );
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({ "labels": labels }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("add_issue_labels: {e}")))?;
        let status = response.status();
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "add_issue_labels status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn remove_issue_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        label: &str,
    ) -> Result<(), GithubAppError> {
        let enc = label.replace(' ', "%20");
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/labels/{enc}",
            self.api_base
        );
        let response = self
            .client
            .delete(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("remove_issue_label: {e}")))?;
        let status = response.status();
        // 404 just means the label was not present — tolerate it.
        if !status.is_success() && status != reqwest::StatusCode::NOT_FOUND {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "remove_issue_label status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn close_issue(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<(), GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .patch(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({ "state": "closed" }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("close_issue: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "close_issue status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn get_issue_labels(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_issue_labels: {e}")))?;
        let status = response.status();
        // A vanished issue carries no labels — treat it as an empty set so the
        // caller neither flags nor clears a non-existent issue.
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "get_issue_labels status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_issue_labels body: {e}")))?;
        let labels = body
            .get("labels")
            .and_then(|l| l.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|l| l.get("name").and_then(|n| n.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(labels)
    }

    async fn list_issue_comments(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Vec<String>, GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/issues/{number}/comments?per_page=100",
            self.api_base
        );
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_issue_comments: {e}")))?;
        let status = response.status();
        // A vanished issue carries no comments — treat it as an empty list so the
        // caller latches no original hash for it (mirrors get_issue_labels' 404).
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(Vec::new());
        }
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "list_issue_comments status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_issue_comments body: {e}")))?;
        let comments = body
            .as_array()
            .map(|arr| {
                arr.iter()
                    .filter_map(|c| c.get("body").and_then(|b| b.as_str()))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        Ok(comments)
    }

    async fn content_file(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        path: &str,
        git_ref: Option<&str>,
    ) -> Result<Option<RemoteFile>, GithubAppError> {
        let mut url = format!(
            "{}/repos/{owner}/{repo}/contents/{}",
            self.api_base,
            path.trim_start_matches('/')
        );
        if let Some(git_ref) = git_ref {
            url.push_str(&format!("?ref={git_ref}"));
        }
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("content_file: {e}")))?;
        let status = response.status();
        // A missing file is the create-path / installed-v0 signal, not an error.
        if status == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "content_file status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("content_file body: {e}")))?;
        let sha = body["sha"]
            .as_str()
            .ok_or_else(|| GithubAppError::Http("content_file: missing sha".to_string()))?
            .to_string();
        // A file object always carries `content`; a directory (array) would not
        // deserialize here — the caller only ever requests concrete file paths.
        let content_base64 = body["content"].as_str().unwrap_or_default().to_string();
        Ok(Some(RemoteFile {
            sha,
            content_base64,
        }))
    }

    async fn repo_default_branch(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<String, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("repo_default_branch: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "repo_default_branch status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("repo_default_branch body: {e}")))?;
        body["default_branch"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| {
                GithubAppError::Http("repo_default_branch: missing default_branch".to_string())
            })
    }

    async fn branch_head_sha(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<String, GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/ref/heads/{branch}",
            self.api_base
        );
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("branch_head_sha: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "branch_head_sha status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("branch_head_sha body: {e}")))?;
        body["object"]["sha"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| GithubAppError::Http("branch_head_sha: missing object.sha".to_string()))
    }

    async fn create_ref(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
        sha: &str,
    ) -> Result<(), GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/git/refs", self.api_base);
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({
                "ref": format!("refs/heads/{branch}"),
                "sha": sha,
            }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_ref: {e}")))?;
        let status = response.status();
        // 422 => the ref already exists (a stale branch from a prior failed run).
        if status == reqwest::StatusCode::UNPROCESSABLE_ENTITY {
            return Err(GithubAppError::RefExists);
        }
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_ref status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn put_file(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        path: &str,
        message: &str,
        content_base64: &str,
        branch: &str,
        sha: Option<&str>,
    ) -> Result<(), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/contents/{}",
            self.api_base,
            path.trim_start_matches('/')
        );
        // The create-vs-update distinction is exactly the presence of `sha`: a
        // CREATE omits it, an UPDATE carries the existing blob SHA.
        let mut body = serde_json::Map::new();
        body.insert("message".to_string(), serde_json::json!(message));
        body.insert("content".to_string(), serde_json::json!(content_base64));
        body.insert("branch".to_string(), serde_json::json!(branch));
        if let Some(sha) = sha {
            body.insert("sha".to_string(), serde_json::json!(sha));
        }
        let response = self
            .client
            .put(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::Value::Object(body))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("put_file: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "put_file status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn create_pull_request(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        title: &str,
        head: &str,
        base: &str,
        body: &str,
    ) -> Result<u64, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls", self.api_base);
        let response = self
            .client
            .post(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({
                "title": title,
                "head": head,
                "base": base,
                "body": body,
            }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_pull_request: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "create_pull_request status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("create_pull_request body: {e}")))?;
        body["number"]
            .as_u64()
            .ok_or_else(|| GithubAppError::Http("create_pull_request: missing number".to_string()))
    }

    async fn merge_pull_request(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
        commit_title: &str,
    ) -> Result<(), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls/{number}/merge",
            self.api_base
        );
        let response = self
            .client
            .put(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .json(&serde_json::json!({
                "merge_method": "merge",
                "commit_title": commit_title,
            }))
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("merge_pull_request: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "merge_pull_request status {status}: {body}"
            )));
        }
        Ok(())
    }

    async fn list_open_pulls(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
    ) -> Result<Vec<PullRequestSummary>, GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/pulls?state=open&per_page=100",
            self.api_base
        );
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_open_pulls: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "list_open_pulls status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("list_open_pulls body: {e}")))?;
        let arr = body.as_array().cloned().unwrap_or_default();
        Ok(arr
            .iter()
            .filter_map(|pr| {
                Some(PullRequestSummary {
                    number: pr["number"].as_u64()?,
                    author_login: pr["user"]["login"].as_str().unwrap_or_default().to_string(),
                    head_sha: pr["head"]["sha"].as_str().unwrap_or_default().to_string(),
                    head_ref: pr["head"]["ref"].as_str().unwrap_or_default().to_string(),
                    title: pr["title"].as_str().unwrap_or_default().to_string(),
                })
            })
            .collect())
    }

    async fn pull_request_mergeable(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<Option<bool>, GithubAppError> {
        let url = format!("{}/repos/{owner}/{repo}/pulls/{number}", self.api_base);
        let response = self
            .client
            .get(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("pull_request_mergeable: {e}")))?;
        let status = response.status();
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "pull_request_mergeable status {status}: {body}"
            )));
        }
        let body: serde_json::Value = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("pull_request_mergeable body: {e}")))?;
        // `mergeable` is `null` until GitHub computes it — `as_bool()` maps that
        // (and an absent field) to `None`, the "retry next reconcile" signal.
        Ok(body["mergeable"].as_bool())
    }

    async fn list_pull_files(
        &self,
        installation_token: &str,
        owner: &str,
        repo: &str,
        pull_number: i64,
    ) -> Result<Vec<PullFileMeta>, GithubAppError> {
        let mut out = Vec::new();
        // Positional `?page=N` paging (bounded, no Link-header parse): stop as
        // soon as a page comes back short — a full 100 means "there may be more".
        for page in 1..=MAX_PULL_FILE_PAGES {
            let url = format!(
                "{}/repos/{owner}/{repo}/pulls/{pull_number}/files",
                self.api_base
            );
            let response = self
                .client
                .get(&url)
                .header("accept", "application/vnd.github+json")
                .header("user-agent", "fkst-hosted")
                .bearer_auth(installation_token)
                .query(&[("per_page", "100".to_string()), ("page", page.to_string())])
                .send()
                .await
                .map_err(|e| GithubAppError::Http(format!("list_pull_files: {e}")))?;
            let status = response.status();
            if let Some(err) = classify_auth_status(status, response.headers()) {
                return Err(err);
            }
            if !status.is_success() {
                let body = response.text().await.unwrap_or_default();
                return Err(GithubAppError::Http(format!(
                    "list_pull_files status {status}: {body}"
                )));
            }
            let body: serde_json::Value = response
                .json()
                .await
                .map_err(|e| GithubAppError::Http(format!("list_pull_files body: {e}")))?;
            let arr = body.as_array().cloned().unwrap_or_default();
            let page_len = arr.len();
            for file in &arr {
                out.push(PullFileMeta {
                    filename: file["filename"].as_str().unwrap_or_default().to_string(),
                    status: file["status"].as_str().unwrap_or_default().to_string(),
                    additions: file["additions"].as_i64().unwrap_or_default(),
                    deletions: file["deletions"].as_i64().unwrap_or_default(),
                    changes: file["changes"].as_i64().unwrap_or_default(),
                    sha: file["sha"].as_str().unwrap_or_default().to_string(),
                    previous_filename: file["previous_filename"].as_str().map(str::to_string),
                });
            }
            // A short page (fewer than the per-page ceiling) is the last page.
            if page_len < 100 {
                break;
            }
        }
        Ok(out)
    }

    async fn get_blob_raw(
        &self,
        installation_token: &str,
        owner: &str,
        repo: &str,
        blob_sha: &str,
        max_bytes: usize,
    ) -> Result<Vec<u8>, GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/blobs/{blob_sha}",
            self.api_base
        );
        let response = self
            .client
            .get(&url)
            // The `raw` media type makes GitHub return the file bytes verbatim
            // (rather than the base64 JSON envelope).
            .header("accept", "application/vnd.github.raw")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(installation_token)
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_blob_raw: {e}")))?;
        let status = response.status();
        if status == reqwest::StatusCode::NOT_FOUND {
            return Err(GithubAppError::NotFound {
                owner_repo: format!("{owner}/{repo}"),
                path: format!("git/blobs/{blob_sha}"),
            });
        }
        if let Some(err) = classify_auth_status(status, response.headers()) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "get_blob_raw status {status}: {body}"
            )));
        }
        // Reject an over-cap blob up front by its advertised length so we never
        // buffer the whole thing into memory just to discard it.
        if let Some(len) = response.content_length() {
            if len > max_bytes as u64 {
                return Err(GithubAppError::BlobTooLarge);
            }
        }
        let bytes = response
            .bytes()
            .await
            .map_err(|e| GithubAppError::Http(format!("get_blob_raw body: {e}")))?;
        // Defence in depth for a chunked response with no Content-Length.
        if bytes.len() > max_bytes {
            return Err(GithubAppError::BlobTooLarge);
        }
        Ok(bytes.to_vec())
    }

    async fn delete_ref(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        branch: &str,
    ) -> Result<(), GithubAppError> {
        let url = format!(
            "{}/repos/{owner}/{repo}/git/refs/heads/{branch}",
            self.api_base
        );
        let response = self
            .client
            .delete(&url)
            .header("accept", "application/vnd.github+json")
            .header("user-agent", "fkst-hosted")
            .bearer_auth(token.expose_secret())
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("delete_ref: {e}")))?;
        let status = response.status();
        // Best-effort cleanup: a already-gone branch (404) or a 422 (ref does not
        // exist) is tolerated so a partial prior run never wedges the next.
        if !status.is_success()
            && status != reqwest::StatusCode::NOT_FOUND
            && status != reqwest::StatusCode::UNPROCESSABLE_ENTITY
        {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "delete_ref status {status}: {body}"
            )));
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "api_tests.rs"]
mod tests;
