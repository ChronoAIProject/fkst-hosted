//! GitHub App listing transport (Model B, issue #359 §9 / PR1): the read-side
//! calls the Model B reconciler needs to enumerate work — issues by label, an
//! open-issue count, the App's installations, and an installation's repos.
//!
//! WHY a sibling of `api.rs`: `api.rs` owns the token-minting transport (App JWT
//! for `/app/*`, installation token for repo calls) and is already at its size
//! budget. These read calls share the SAME auth + error-classification
//! discipline (Bearer auth; 401/403 → auth-vs-rate-limit disambiguation; typed
//! [`GithubAppError`] variants), so they mirror `HttpGithubApi` (injected
//! `api_base`, 20s timeout, `fkst-hosted-api` user-agent) and reuse the `api`
//! module's rate-limit helpers — exactly as `contents.rs` does — rather than
//! duplicating them.
//!
//! Purely additive: nothing calls these yet. The reconciler (a later PR) will
//! hold a [`GithubListing`] and drive it; the trait is the injectable seam so
//! that reconciler is unit-testable against a fake, mirroring [`super::api::GithubApi`].

use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use super::api::{is_rate_limited, reset_seconds};
use super::GithubAppError;
use crate::models::RepoRef;

/// Request timeout for every listing call (mirrors `api.rs`).
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// A trimmed GitHub issue carrying only the fields the Model B reconciler needs
/// (number, labels, state, assignees, author). GitHub's issues endpoint returns
/// pull requests too; those are filtered out before an [`IssueSummary`] is built
/// (see [`list_issues_by_label`](GithubListing::list_issues_by_label)).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IssueSummary {
    pub number: i64,
    pub title: String,
    /// The raw issue body (Markdown). The Model B reconciler parses this into a
    /// launch spec (see `crate::reconcile::registry::parse_registration`); GitHub
    /// omits it entirely for a body-less issue, so it defaults to empty.
    pub body: String,
    /// Label NAMES (GitHub returns `[{ "name": ... }]`; mapped to the names).
    pub labels: Vec<String>,
    pub state: String,
    /// Assignee login names.
    pub assignees: Vec<String>,
    pub user_login: String,
    pub user_id: i64,
}

/// One GitHub App installation, trimmed to the id + account login the reconciler
/// enumerates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstallationSummary {
    pub id: i64,
    pub account_login: String,
}

// ---------------------------------------------------------------------------
// Raw wire shapes (private): decoded straight from GitHub, then mapped to the
// trimmed public summaries above.
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
struct RawLabel {
    name: String,
}

/// A `{ "login": ... }` object — GitHub's shape for an issue assignee and an
/// installation account.
#[derive(Deserialize)]
struct RawLogin {
    login: String,
}

#[derive(Deserialize)]
struct RawUser {
    login: String,
    id: i64,
}

#[derive(Deserialize)]
struct RawIssue {
    number: i64,
    #[serde(default)]
    title: String,
    /// GitHub sends `"body": null` (or omits it) for a body-less issue; treat
    /// both as an empty body rather than erroring the whole page decode.
    #[serde(default, deserialize_with = "deserialize_null_default")]
    body: String,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    state: String,
    #[serde(default)]
    assignees: Vec<RawLogin>,
    user: RawUser,
    /// Present ONLY when this "issue" is actually a pull request. The issues
    /// endpoint returns PRs too; presence of this field is how they are told
    /// apart and filtered out.
    pull_request: Option<serde_json::Value>,
}

impl RawIssue {
    /// Map the wire shape to the trimmed summary: label objects → names,
    /// assignee objects → logins, author → `user_login`/`user_id`.
    fn into_summary(self) -> IssueSummary {
        IssueSummary {
            number: self.number,
            title: self.title,
            body: self.body,
            labels: self.labels.into_iter().map(|l| l.name).collect(),
            state: self.state,
            assignees: self.assignees.into_iter().map(|a| a.login).collect(),
            user_login: self.user.login,
            user_id: self.user.id,
        }
    }
}

/// Deserialize a possibly-`null` value into `T::default()`. GitHub renders an
/// empty issue body as JSON `null` (not an omitted field), which a plain
/// `#[serde(default)]` would reject for a non-`Option` `String`; this coerces both
/// `null` and a present value uniformly.
fn deserialize_null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    let opt = Option::<T>::deserialize(deserializer)?;
    Ok(opt.unwrap_or_default())
}

#[derive(Deserialize)]
struct RawInstallation {
    id: i64,
    account: RawLogin,
}

#[derive(Deserialize)]
struct RawRepo {
    name: String,
    owner: RawLogin,
}

/// `GET /installation/repositories` wraps its repos in a `repositories` array
/// (alongside a `total_count`); only the array is needed.
#[derive(Deserialize)]
struct RepoPage {
    #[serde(default)]
    repositories: Vec<RawRepo>,
}

/// `GET /search/issues` returns a `total_count` alongside the (unused) `items`.
#[derive(Deserialize)]
struct SearchCount {
    total_count: u64,
}

// ---------------------------------------------------------------------------
// Transport trait + HTTP implementation
// ---------------------------------------------------------------------------

/// Read-side GitHub App transport for the Model B reconciler. Injected so the
/// reconciler is unit-testable against a fake, mirroring [`super::api::GithubApi`].
///
/// Every method takes the Bearer credential explicitly: repo/search/installation
/// calls take an installation `token`; `list_installations` takes the App JWT.
/// (The reconciler mints those tokens through the existing
/// [`super::GithubAppTokens`] path and passes them in.)
#[async_trait]
pub trait GithubListing: Send + Sync {
    /// `GET /repos/{owner}/{repo}/issues?labels=<label>&state=open&per_page=100`,
    /// following `Link` pagination to exhaustion. Pull requests (which the issues
    /// endpoint also returns) are excluded. Installation-token auth.
    async fn list_issues_by_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError>;

    /// Count open issues carrying `label` via the Search API in ONE call
    /// (`GET /search/issues?...&per_page=1`, reading `total_count`; no
    /// pagination). The label is URL-encoded. Installation-token auth.
    async fn count_open_issues_with_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<u64, GithubAppError>;

    /// `GET /app/installations?per_page=100` with App-JWT auth, following `Link`
    /// pagination to exhaustion.
    async fn list_installations(
        &self,
        app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError>;

    /// `GET /installation/repositories?per_page=100` with installation-token
    /// auth, following `Link` pagination to exhaustion.
    async fn list_installation_repos(
        &self,
        token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError>;
}

/// Production HTTP transport backed by reqwest (mirrors [`super::api::HttpGithubApi`]).
pub struct HttpGithubListing {
    api_base: String,
    client: reqwest::Client,
}

impl std::fmt::Debug for HttpGithubListing {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("HttpGithubListing")
            .field("api_base", &self.api_base)
            .finish()
    }
}

impl HttpGithubListing {
    pub fn new(api_base: &str) -> Result<Self, GithubAppError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted-api")
            .build()
            .map_err(|e| GithubAppError::Http(format!("listing client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// Perform one page GET: send with Bearer `auth`, classify the response, then
    /// decode the body into `T` and return it alongside the `rel="next"` URL (if
    /// any). Centralises the auth + error-classification discipline every listing
    /// method shares. `query` is applied only when `Some` — the first page passes
    /// the query params; a followed `next` URL already carries the encoded query,
    /// so subsequent pages pass `None`.
    async fn get_page<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        auth: &SecretString,
        query: Option<&[(&str, &str)]>,
        resource: &str,
    ) -> Result<(T, Option<String>), GithubAppError> {
        let mut req = self
            .client
            .get(url)
            .header("accept", "application/vnd.github+json")
            .bearer_auth(auth.expose_secret());
        if let Some(q) = query {
            req = req.query(q);
        }
        let response = req
            .send()
            .await
            .map_err(|e| GithubAppError::Http(format!("{resource}: {e}")))?;

        let status = response.status();
        if let Some(err) = classify_error(status, response.headers(), resource) {
            return Err(err);
        }
        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            return Err(GithubAppError::Http(format!(
                "{resource} status {status}: {body}"
            )));
        }

        // Read the next-page link BEFORE consuming the body (both borrow the
        // response; `.json()` takes it by value).
        let next = next_page_url(response.headers());
        let page: T = response
            .json()
            .await
            .map_err(|e| GithubAppError::Http(format!("{resource} body: {e}")))?;
        Ok((page, next))
    }
}

#[async_trait]
impl GithubListing for HttpGithubListing {
    async fn list_issues_by_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<IssueSummary>, GithubAppError> {
        let mut url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        // Applied to the first page only; followed `next` URLs carry it already.
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![
            ("labels", label),
            ("state", "open"),
            ("per_page", "100"),
        ]);
        let resource = "list_issues_by_label";
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawIssue>, _) = self
                .get_page(&url, token, query.as_deref(), resource)
                .await?;
            // Skip PRs (the issues endpoint returns them with a `pull_request`).
            out.extend(
                page.into_iter()
                    .filter(|raw| raw.pull_request.is_none())
                    .map(RawIssue::into_summary),
            );
            match next {
                Some(n) => {
                    url = n;
                    query = None;
                }
                None => break,
            }
        }
        Ok(out)
    }

    async fn count_open_issues_with_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<u64, GithubAppError> {
        // The label is embedded in the search qualifier and URL-encoded by
        // reqwest's query serializer (spaces/quotes → percent/`+` escapes).
        let q = format!("repo:{owner}/{repo} type:issue state:open label:\"{label}\"");
        let url = format!("{}/search/issues", self.api_base);
        let (page, _next): (SearchCount, _) = self
            .get_page(
                &url,
                token,
                Some(&[("q", q.as_str()), ("per_page", "1")]),
                "count_open_issues_with_label",
            )
            .await?;
        Ok(page.total_count)
    }

    async fn list_installations(
        &self,
        app_jwt: &SecretString,
    ) -> Result<Vec<InstallationSummary>, GithubAppError> {
        let mut url = format!("{}/app/installations", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let resource = "list_installations";
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawInstallation>, _) = self
                .get_page(&url, app_jwt, query.as_deref(), resource)
                .await?;
            out.extend(page.into_iter().map(|raw| InstallationSummary {
                id: raw.id,
                account_login: raw.account.login,
            }));
            match next {
                Some(n) => {
                    url = n;
                    query = None;
                }
                None => break,
            }
        }
        Ok(out)
    }

    async fn list_installation_repos(
        &self,
        token: &SecretString,
    ) -> Result<Vec<RepoRef>, GithubAppError> {
        let mut url = format!("{}/installation/repositories", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let resource = "list_installation_repos";
        let mut out = Vec::new();
        loop {
            let (page, next): (RepoPage, _) = self
                .get_page(&url, token, query.as_deref(), resource)
                .await?;
            out.extend(page.repositories.into_iter().map(|raw| RepoRef {
                owner: raw.owner.login,
                name: raw.name,
            }));
            match next {
                Some(n) => {
                    url = n;
                    query = None;
                }
                None => break,
            }
        }
        Ok(out)
    }
}

// ---------------------------------------------------------------------------
// Shared classification / pagination helpers
// ---------------------------------------------------------------------------

/// Classify a GitHub response for the listing endpoints, mirroring the `api`
/// module's disambiguation: 401 / plain 403 → [`GithubAppError::AppAuth`];
/// 403 with rate-limit evidence → [`GithubAppError::RateLimited`]; 404 →
/// [`GithubAppError::NotFound`] (so a caller can tell a missing resource apart
/// from a transport error). Returns `None` for any other status (including
/// success), leaving the generic `Http` fallback to the caller.
fn classify_error(
    status: reqwest::StatusCode,
    headers: &reqwest::header::HeaderMap,
    resource: &str,
) -> Option<GithubAppError> {
    match status {
        reqwest::StatusCode::UNAUTHORIZED => Some(GithubAppError::AppAuth),
        reqwest::StatusCode::FORBIDDEN => {
            if is_rate_limited(headers) {
                Some(GithubAppError::RateLimited(reset_seconds(headers)))
            } else {
                Some(GithubAppError::AppAuth)
            }
        }
        reqwest::StatusCode::NOT_FOUND => Some(GithubAppError::NotFound {
            owner_repo: resource.to_string(),
            path: String::new(),
        }),
        _ => None,
    }
}

/// Extract the `rel="next"` URL from a GitHub `Link` header, if present.
///
/// GitHub paginates with a header like
/// `<https://api.github.com/...&page=2>; rel="next", <...>; rel="last"`. The
/// returned URL is absolute and already carries the encoded query, so the caller
/// follows it verbatim. Returns `None` on the last page (no `rel="next"`).
fn next_page_url(headers: &reqwest::header::HeaderMap) -> Option<String> {
    let link = headers.get(reqwest::header::LINK)?.to_str().ok()?;
    for part in link.split(',') {
        let segments: Vec<&str> = part.split(';').map(str::trim).collect();
        if !segments.contains(&"rel=\"next\"") {
            continue;
        }
        if let Some(target) = segments.first() {
            let url = target.trim_start_matches('<').trim_end_matches('>');
            if !url.is_empty() {
                return Some(url.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
#[path = "listing_tests.rs"]
mod tests;
