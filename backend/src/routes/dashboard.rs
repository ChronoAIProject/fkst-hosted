//! Shared GitHub read/write client for the user-facing session surface.
//!
//! [`DashboardGithub`] is the small GitHub client the canvas dashboard
//! ([`crate::routes::canvas`]) and the repo endpoints ([`crate::routes::repos`])
//! share: the user-token installation/repo/org enumeration plus the `state=all`
//! issue reads that the reconciler's `GithubListing` trait does not expose
//! (`list_issues_by_label` is `state=open` and carries test doubles). Keeping it
//! here — rather than extending that trait — keeps the reconciler's listing lean.
//!
//! Alongside the client this module hosts two tiny helpers the same consumers
//! reuse: [`bearer_token`] (pull the caller's `Authorization: Bearer` out of the
//! request) and [`status_labels`] (the `fkst-*` control-plane markers on an
//! issue).
//!
//! A GitHub-App user token only ever sees THIS app's installations, so every
//! user-token read here is already scoped to the signed-in user; installed-repo
//! issue reads use an APP installation token (minted via
//! [`crate::github_app::GithubAppTokens`]) at the call sites.

use axum::http::{header, HeaderMap};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;

use crate::error::AppError;
use crate::github_app::listing::IssueSummary;
use crate::models::RepoRef;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The `fkst-*` labels on a trigger issue (control-plane status markers).
/// `pub(crate)`: the canvas sessions endpoint renders the same projection.
pub(crate) fn status_labels(issue: &IssueSummary) -> Vec<String> {
    issue
        .labels
        .iter()
        .filter(|l| l.starts_with("fkst-"))
        .cloned()
        .collect()
}

// ---- GitHub reads (user token + installation token) -------------------------

/// A minimal GitHub read client for the dashboard: the user-token installation
/// enumeration + `state=all` issue reads that the reconciler's `GithubListing`
/// does not expose. Fields are `pub(crate)` because the canvas endpoints
/// ([`crate::routes::canvas`]) extend this client with their own methods in a
/// sibling module (same crate, separate file to respect the file-size budget).
pub(crate) struct DashboardGithub {
    pub(crate) api_base: String,
    pub(crate) client: reqwest::Client,
}

#[derive(Deserialize)]
struct RawLogin {
    login: String,
    /// `User` or `Organization` when GitHub includes account type.
    #[serde(rename = "type", default)]
    kind: String,
}
#[derive(Deserialize)]
struct RawInstallation {
    id: i64,
    account: RawLogin,
    /// `"all"` or `"selected"` — whether the installation covers every repo.
    #[serde(default)]
    repository_selection: String,
}
#[derive(Deserialize)]
struct InstallationsPage {
    #[serde(default)]
    installations: Vec<RawInstallation>,
}
#[derive(Deserialize)]
struct RawRepo {
    #[serde(default)]
    id: i64,
    name: String,
    owner: RawLogin,
    #[serde(default)]
    private: bool,
}
#[derive(Deserialize)]
struct ReposPage {
    #[serde(default)]
    repositories: Vec<RawRepo>,
}
#[derive(Deserialize)]
struct RawRepoOwner {
    login: String,
    /// `"User"` or `"Organization"`.
    #[serde(rename = "type", default)]
    kind: String,
}
#[derive(Deserialize)]
struct RawRepoPerms {
    #[serde(default)]
    admin: bool,
}
/// One element of the bare-array `GET /user/repos` response.
#[derive(Deserialize)]
struct RawUserRepo {
    id: i64,
    name: String,
    owner: RawRepoOwner,
    #[serde(default)]
    private: bool,
    /// Present for authenticated requests; defaults closed when absent.
    permissions: Option<RawRepoPerms>,
}

/// A repo the user can access, as the repo-listing endpoint consumes it.
#[derive(Debug, Clone)]
pub(crate) struct UserRepo {
    pub id: i64,
    pub owner: String,
    pub name: String,
    pub private: bool,
    pub org: bool,
    pub admin: bool,
}

#[derive(Deserialize)]
struct RawLabel {
    name: String,
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
    #[serde(default, deserialize_with = "null_default")]
    body: String,
    #[serde(default)]
    labels: Vec<RawLabel>,
    #[serde(default)]
    state: String,
    user: RawUser,
    /// Present only when this "issue" is actually a PR (filtered out).
    pull_request: Option<serde_json::Value>,
    // Presentation metadata the canvas dashboard renders (issue links + ISO
    // timestamps). Defaulted defensively — GitHub always sends them, but a
    // missing field must never fail the whole listing.
    #[serde(default)]
    html_url: String,
    #[serde(default)]
    created_at: String,
    #[serde(default)]
    updated_at: String,
    #[serde(default)]
    closed_at: Option<String>,
}

/// One issue from a `state=all` label read: the reconciler-shaped
/// [`IssueSummary`] (what `parse_registration` consumes) PLUS the presentation
/// metadata GitHub sends alongside it (link + ISO-8601 timestamps) that the
/// canvas dashboard renders. Kept as a wrapper — not extra `IssueSummary`
/// fields — so the reconciler's widely-constructed summary type stays lean.
#[derive(Debug, Clone)]
pub(crate) struct IssueWithMeta {
    pub summary: IssueSummary,
    pub html_url: String,
    pub created_at: String,
    pub updated_at: String,
    /// `None` while the issue is open.
    pub closed_at: Option<String>,
}

/// Coerce a possibly-`null` JSON value into `T::default()` (GitHub sends
/// `"body": null` for a body-less issue).
fn null_default<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Default + Deserialize<'de>,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// One installation the user can access (this app only).
#[derive(Debug, Clone)]
pub(crate) struct InstallationRef {
    pub id: i64,
    pub account: String,
    /// `personal` or `org` (empty/unknown values fail closed to `personal`).
    pub account_kind: String,
    /// `"all"` or `"selected"` (empty when GitHub omits it).
    pub repository_selection: String,
}

/// Normalize GitHub's account discriminator into the canvas wire contract.
/// Unknown/omitted values are treated as personal: only an explicit
/// `Organization` may claim the organization presentation.
fn github_account_kind(kind: &str) -> &'static str {
    if kind.eq_ignore_ascii_case("organization") {
        "org"
    } else {
        "personal"
    }
}

impl DashboardGithub {
    pub(crate) fn new(api_base: &str) -> Result<Self, AppError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .user_agent("fkst-hosted")
            .build()
            .map_err(|e| AppError::Internal(anyhow::anyhow!("dashboard client build: {e}")))?;
        Ok(Self {
            api_base: api_base.trim_end_matches('/').to_string(),
            client,
        })
    }

    /// GET a page with Bearer `auth`; return the decoded body + the `rel="next"` URL.
    /// `pub(crate)` so the canvas extension methods reuse the same paging +
    /// error-mapping transport instead of duplicating it.
    pub(crate) async fn get_page<T: serde::de::DeserializeOwned>(
        &self,
        url: &str,
        auth: &SecretString,
        query: Option<&[(&str, &str)]>,
        resource: &str,
    ) -> Result<(T, Option<String>), AppError> {
        let mut req = self
            .client
            .get(url)
            .header(header::ACCEPT, "application/vnd.github+json")
            .bearer_auth(auth.expose_secret());
        if let Some(q) = query {
            req = req.query(q);
        }
        let response = req.send().await.map_err(|e| {
            tracing::warn!(resource, error = %e, "dashboard github request failed");
            AppError::Unavailable(format!("github request failed ({resource})"))
        })?;
        let status = response.status();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            return Err(AppError::Unauthorized("github token rejected".to_string()));
        }
        if status == reqwest::StatusCode::FORBIDDEN
            && response
                .headers()
                .get("x-ratelimit-remaining")
                .and_then(|v| v.to_str().ok())
                == Some("0")
        {
            return Err(AppError::Unavailable(
                "github rate limit reached; try again shortly".to_string(),
            ));
        }
        if !status.is_success() {
            return Err(AppError::Upstream(format!(
                "github {resource} status {status}"
            )));
        }
        let next = next_page_url(response.headers());
        let page: T = response
            .json()
            .await
            .map_err(|e| AppError::Upstream(format!("github {resource} body: {e}")))?;
        Ok((page, next))
    }

    /// `GET /user/repos` (user token) — EVERY repo the user can access (private,
    /// public, and organization repos), paginated. Powers the repo-listing
    /// endpoint (issue #499); `affiliation` covers owned, collaborator, and
    /// org-member repos, `visibility=all` includes private ones.
    pub(crate) async fn user_all_repos(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<UserRepo>, AppError> {
        let mut url = format!("{}/user/repos", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![
            ("per_page", "100"),
            ("affiliation", "owner,collaborator,organization_member"),
            ("visibility", "all"),
        ]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawUserRepo>, _) = self
                .get_page(&url, user_token, query.as_deref(), "user_repos")
                .await?;
            out.extend(page.into_iter().map(|r| UserRepo {
                id: r.id,
                owner: r.owner.login,
                name: r.name,
                private: r.private,
                org: r.owner.kind == "Organization",
                admin: r.permissions.map(|p| p.admin).unwrap_or(false),
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

    /// `GET /user/orgs` (user token) — the organizations the user belongs to,
    /// paginated. Powers the create-repo owner picker (issue #503).
    pub(crate) async fn user_orgs(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<String>, AppError> {
        let mut url = format!("{}/user/orgs", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawLogin>, _) = self
                .get_page(&url, user_token, query.as_deref(), "user_orgs")
                .await?;
            out.extend(page.into_iter().map(|o| o.login));
            match next {
                Some(next_url) => {
                    url = next_url;
                    query = None;
                }
                None => return Ok(out),
            }
        }
    }

    /// `POST /user/repos` (personal) or `POST /orgs/{org}/repos` (organization)
    /// with the USER token — the repo is created AS the signed-in user (issue
    /// #503). GitHub gates repo creation for App user-to-server tokens behind
    /// the App's `administration` repository permission; a 403 maps to a
    /// message naming that requirement so the operator knows what to grant.
    pub(crate) async fn create_repo(
        &self,
        user_token: &SecretString,
        org: Option<&str>,
        name: &str,
        private: bool,
        description: Option<&str>,
    ) -> Result<UserRepo, AppError> {
        let url = match org {
            Some(org) => format!("{}/orgs/{org}/repos", self.api_base),
            None => format!("{}/user/repos", self.api_base),
        };
        let mut body = serde_json::json!({ "name": name, "private": private });
        if let Some(desc) = description {
            body["description"] = serde_json::Value::String(desc.to_string());
        }
        let response = self
            .client
            .post(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .json(&body)
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github create-repo transport error");
                AppError::Unavailable("github create-repo request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            let raw: RawUserRepo = response.json().await.map_err(|e| {
                tracing::warn!(error = %e, "github create-repo response did not parse");
                AppError::Unavailable("github create-repo response was malformed".to_string())
            })?;
            return Ok(UserRepo {
                id: raw.id,
                owner: raw.owner.login,
                name: raw.name,
                private: raw.private,
                org: raw.owner.kind == "Organization",
                // The creator administers a fresh repo; default open if GitHub
                // omits the permissions block on the create response.
                admin: raw.permissions.map(|p| p.admin).unwrap_or(true),
            });
        }
        // Carry GitHub's own `message` (it names the real cause: permission
        // missing, name taken, org policy) without leaking anything else.
        let message = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("github returned status {status}"));
        Err(match status.as_u16() {
            401 => AppError::Unauthorized(format!("github rejected the token: {message}")),
            403 | 404 => AppError::Forbidden(format!(
                "GitHub refused repo creation: {message} — if this persists, the \
                 GitHub App likely lacks the 'Administration' repository \
                 permission required for user-token repo creation"
            )),
            422 => AppError::Validation(format!("GitHub rejected the repository: {message}")),
            _ => AppError::Unavailable(format!("github create-repo returned status {status}")),
        })
    }

    /// `DELETE /app/installations/{id}` (APP JWT — the app uninstalls itself
    /// from the account; issue #509). The only non-user-token call here.
    pub(crate) async fn delete_installation(
        &self,
        app_jwt: &SecretString,
        installation_id: i64,
    ) -> Result<(), AppError> {
        let url = format!("{}/app/installations/{installation_id}", self.api_base);
        self.delete_no_content(&url, app_jwt, "delete_installation")
            .await
    }

    /// Shared DELETE-expecting-204 helper (Bearer `auth`, GitHub error `message`
    /// carried into the error).
    async fn delete_no_content(
        &self,
        url: &str,
        auth: &SecretString,
        op: &str,
    ) -> Result<(), AppError> {
        let response = self
            .client
            .delete(url)
            .bearer_auth(auth.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(op, error = %e, "github delete transport error");
                AppError::Unavailable(format!("github {op} request failed"))
            })?;
        let status = response.status();
        if status.is_success() {
            return Ok(());
        }
        let message = response
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| {
                v.get("message")
                    .and_then(|m| m.as_str())
                    .map(str::to_string)
            })
            .unwrap_or_else(|| format!("github returned status {status}"));
        Err(match status.as_u16() {
            401 => AppError::Unauthorized(format!("github rejected the token: {message}")),
            403 => AppError::Forbidden(format!("GitHub refused {op}: {message}")),
            404 => AppError::NotFound(format!("github {op}: {message}")),
            _ => AppError::Unavailable(format!("github {op} returned status {status}")),
        })
    }

    /// `GET /user/installations` (user token) — the app installations this user can access.
    pub(crate) async fn user_installations(
        &self,
        user_token: &SecretString,
    ) -> Result<Vec<InstallationRef>, AppError> {
        let mut url = format!("{}/user/installations", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (InstallationsPage, _) = self
                .get_page(&url, user_token, query.as_deref(), "user_installations")
                .await?;
            out.extend(page.installations.into_iter().map(|raw| {
                let account_kind = github_account_kind(&raw.account.kind).to_string();
                InstallationRef {
                    id: raw.id,
                    account: raw.account.login,
                    account_kind,
                    repository_selection: raw.repository_selection,
                }
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

    /// `GET /app/installations` (App JWT) — every installation belonging to
    /// this deployment's GitHub App. This is intentionally used only after the
    /// caller has matched `FKST_GLOBAL_ADMINS`; ordinary dashboard callers stay
    /// on [`Self::user_installations`].
    pub(crate) async fn app_installations(
        &self,
        app_jwt: &SecretString,
    ) -> Result<Vec<InstallationRef>, AppError> {
        let mut url = format!("{}/app/installations", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawInstallation>, _) = self
                .get_page(&url, app_jwt, query.as_deref(), "app_installations")
                .await?;
            out.extend(page.into_iter().map(|raw| {
                let account_kind = github_account_kind(&raw.account.kind).to_string();
                InstallationRef {
                    id: raw.id,
                    account: raw.account.login,
                    account_kind,
                    repository_selection: raw.repository_selection,
                }
            }));
            match next {
                Some(n) => {
                    url = n;
                    query = None;
                }
                None => return Ok(out),
            }
        }
    }

    /// `GET /user/installations/{id}/repositories` (user token) — repos in the
    /// installation the user can access.
    pub(crate) async fn user_installation_repos(
        &self,
        user_token: &SecretString,
        installation_id: i64,
    ) -> Result<Vec<RepoRef>, AppError> {
        let mut url = format!(
            "{}/user/installations/{installation_id}/repositories",
            self.api_base
        );
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (ReposPage, _) = self
                .get_page(
                    &url,
                    user_token,
                    query.as_deref(),
                    "user_installation_repos",
                )
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

    /// `GET /installation/repositories` (installation token) — every repository
    /// covered by one App installation, including repositories outside the
    /// global admin's own user-token visibility. Returned repos are read-only in
    /// the admin projection (`admin=false`); GitHub still enforces any mutation
    /// attempted with the caller's user token on the separate write endpoints.
    pub(crate) async fn installation_repos(
        &self,
        installation_token: &SecretString,
    ) -> Result<Vec<UserRepo>, AppError> {
        let mut url = format!("{}/installation/repositories", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![("per_page", "100")]);
        let mut out = Vec::new();
        loop {
            let (page, next): (ReposPage, _) = self
                .get_page(
                    &url,
                    installation_token,
                    query.as_deref(),
                    "installation_repositories",
                )
                .await?;
            out.extend(page.repositories.into_iter().map(|raw| {
                let org = github_account_kind(&raw.owner.kind) == "org";
                UserRepo {
                    id: raw.id,
                    owner: raw.owner.login,
                    name: raw.name,
                    private: raw.private,
                    org,
                    admin: false,
                }
            }));
            match next {
                Some(n) => {
                    url = n;
                    query = None;
                }
                None => return Ok(out),
            }
        }
    }

    /// `GET /repos/{owner}/{repo}/issues?labels=<label>&state=all` (installation
    /// token), following pagination; PRs are excluded.
    pub(crate) async fn issues_by_label_all(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<IssueWithMeta>, AppError> {
        self.issues_by_label(token, owner, repo, label, "all").await
    }

    /// `GET /repos/{owner}/{repo}/issues?labels=<label>&state=<state>` (any
    /// bearer token), following pagination; PRs are excluded. `state` is one of
    /// GitHub's `open`/`closed`/`all` — the canvas overview reads `open` only so
    /// counting active sessions never pages through a repo's closed history.
    pub(crate) async fn issues_by_label(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
        state: &str,
    ) -> Result<Vec<IssueWithMeta>, AppError> {
        let mut url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![
            ("labels", label),
            ("state", state),
            ("per_page", "100"),
        ]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawIssue>, _) = self
                .get_page(&url, token, query.as_deref(), "issues_by_label")
                .await?;
            out.extend(
                page.into_iter()
                    .filter(|r| r.pull_request.is_none())
                    .map(|r| IssueWithMeta {
                        summary: IssueSummary {
                            number: r.number,
                            title: r.title,
                            body: r.body,
                            labels: r.labels.into_iter().map(|l| l.name).collect(),
                            state: r.state,
                            assignees: Vec::new(),
                            user_login: r.user.login,
                            user_id: r.user.id,
                        },
                        html_url: r.html_url,
                        created_at: r.created_at,
                        updated_at: r.updated_at,
                        closed_at: r.closed_at,
                    }),
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
}

/// Extract the `rel="next"` URL from a GitHub `Link` header, if present.
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

/// Pull the non-empty bearer token out of the `Authorization` header, or 401.
pub(crate) fn bearer_token(headers: &HeaderMap) -> Result<SecretString, AppError> {
    let value = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| AppError::Unauthorized("missing Authorization header".to_string()))?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::Unauthorized("Authorization must be a Bearer token".to_string())
        })?;
    Ok(SecretString::from(token.to_string()))
}

#[cfg(test)]
#[path = "dashboard_tests.rs"]
mod tests;
