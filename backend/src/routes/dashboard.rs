//! `GET /api/v1/dashboard` — the signed-in user's fkst sessions across the repos
//! where the fkst-hosted App is installed.
//!
//! Assembled fresh per request (Phase 2; caching + an async pull-job land later):
//! 1. The USER token (the `Authorization: Bearer` the [`GithubUser`] extractor
//!    verified) lists the App installations the user can access
//!    (`GET /user/installations`) and each installation's repos
//!    (`GET /user/installations/{id}/repositories`). A GitHub-App user token only
//!    ever sees THIS app's installations, so the result is already user-scoped.
//! 2. For each installed repo an APP installation token (minted via
//!    [`crate::github_app::GithubAppTokens`]) reads the repo's
//!    `fkst-substrate-trigger` issues — open AND closed — and, per trigger, its
//!    work-label issues. Each trigger issue is parsed with the reconciler's
//!    [`parse_registration`] so a session groups exactly as the control plane sees
//!    it (one trigger issue = one session + its work-label issues).
//!
//! Self-contained on purpose: it does NOT extend the reconciler's `GithubListing`
//! trait (whose `list_issues_by_label` is `state=open` and which has test doubles);
//! the user-token endpoints + `state=all` reads live here.

use axum::extract::State;
use axum::http::{header, HeaderMap};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::listing::IssueSummary;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::registry::parse_registration;
use crate::state::AppState;

const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

// ---- Response DTOs ----------------------------------------------------------

/// The whole dashboard: the repos where the App is installed for this user, each
/// with its fkst sessions.
#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardView {
    /// Whether the GitHub App is configured on this deployment. When false, repos
    /// are listed but sessions cannot be read (no App token to read issues with).
    pub app_configured: bool,
    /// Number of App installations scanned for this user.
    pub installations: usize,
    /// The installed repositories + their sessions.
    pub repos: Vec<RepoView>,
}

/// One repository where the App is installed, with its sessions.
#[derive(Debug, Serialize, ToSchema)]
pub struct RepoView {
    pub owner: String,
    pub name: String,
    pub installation_id: i64,
    pub sessions: Vec<SessionGroup>,
}

/// One fkst session = one trigger issue (+ its work-label issues). A trigger whose
/// body fails to parse carries `invalid_reason` and no work issues.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionGroup {
    /// The deterministic session id; absent when the trigger body is invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    /// The `### Session Name`; absent when invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// The `### Work Label` whose issues form this session's queue; absent when invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub work_label: Option<String>,
    /// The `### Auto-merge` opt-in; absent when invalid.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_merge: Option<bool>,
    /// The `### Environment` selection, if any.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<String>,
    /// The `### Packages`, rendered as `owner/repo@ref:path`.
    pub packages: Vec<String>,
    /// The parse error when the trigger body is malformed; else absent.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub invalid_reason: Option<String>,
    /// The `fkst-*` control-plane status labels on the trigger issue.
    pub status_labels: Vec<String>,
    /// The trigger issue itself.
    pub trigger: IssueView,
    /// The session's work-label issues (open AND closed).
    pub work_issues: Vec<IssueView>,
}

/// A trimmed GitHub issue for the dashboard.
#[derive(Debug, Serialize, ToSchema)]
pub struct IssueView {
    pub number: i64,
    pub title: String,
    /// `open` or `closed`.
    pub state: String,
    pub author: String,
    pub labels: Vec<String>,
}

impl IssueView {
    fn from_summary(issue: &IssueSummary) -> Self {
        IssueView {
            number: issue.number,
            title: issue.title.clone(),
            state: issue.state.clone(),
            author: issue.user_login.clone(),
            labels: issue.labels.clone(),
        }
    }
}

/// The `fkst-*` labels on a trigger issue (control-plane status markers).
fn status_labels(issue: &IssueSummary) -> Vec<String> {
    issue
        .labels
        .iter()
        .filter(|l| l.starts_with("fkst-"))
        .cloned()
        .collect()
}

/// Build a session view from a parsed registration + its work issues.
fn build_session(
    trigger: &IssueSummary,
    reg: &SessionRegistration,
    work_issues: Vec<IssueSummary>,
) -> SessionGroup {
    SessionGroup {
        session_id: Some(reg.session_id.clone()),
        name: Some(reg.def.name.clone()),
        work_label: Some(reg.def.work_label.clone()),
        auto_merge: Some(reg.auto_merge),
        environment: reg.def.environment.clone(),
        packages: reg
            .def
            .packages
            .iter()
            .map(|p| format!("{}/{}@{}:{}", p.owner, p.repo, p.git_ref, p.path))
            .collect(),
        invalid_reason: None,
        status_labels: status_labels(trigger),
        trigger: IssueView::from_summary(trigger),
        work_issues: work_issues.iter().map(IssueView::from_summary).collect(),
    }
}

/// Build a session view for a trigger issue whose body failed to parse.
fn build_invalid_session(trigger: &IssueSummary, reason: String) -> SessionGroup {
    SessionGroup {
        session_id: None,
        name: None,
        work_label: None,
        auto_merge: None,
        environment: None,
        packages: Vec::new(),
        invalid_reason: Some(reason),
        status_labels: status_labels(trigger),
        trigger: IssueView::from_summary(trigger),
        work_issues: Vec::new(),
    }
}

// ---- GitHub reads (user token + installation token) -------------------------

/// A minimal GitHub read client for the dashboard: the user-token installation
/// enumeration + `state=all` issue reads that the reconciler's `GithubListing`
/// does not expose.
pub(crate) struct DashboardGithub {
    api_base: String,
    client: reqwest::Client,
}

#[derive(Deserialize)]
struct RawLogin {
    login: String,
}
#[derive(Deserialize)]
struct RawInstallation {
    id: i64,
    account: RawLogin,
}
#[derive(Deserialize)]
struct InstallationsPage {
    #[serde(default)]
    installations: Vec<RawInstallation>,
}
#[derive(Deserialize)]
struct RawRepo {
    name: String,
    owner: RawLogin,
}
#[derive(Deserialize)]
struct ReposPage {
    #[serde(default)]
    repositories: Vec<RawRepo>,
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
#[derive(Debug)]
pub(crate) struct InstallationRef {
    pub id: i64,
    #[allow(dead_code)]
    pub account: String,
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
    async fn get_page<T: serde::de::DeserializeOwned>(
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
            out.extend(page.installations.into_iter().map(|raw| InstallationRef {
                id: raw.id,
                account: raw.account.login,
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

    /// `GET /repos/{owner}/{repo}/issues?labels=<label>&state=all` (installation
    /// token), following pagination; PRs are excluded.
    pub(crate) async fn issues_by_label_all(
        &self,
        token: &SecretString,
        owner: &str,
        repo: &str,
        label: &str,
    ) -> Result<Vec<IssueSummary>, AppError> {
        let mut url = format!("{}/repos/{owner}/{repo}/issues", self.api_base);
        let mut query: Option<Vec<(&str, &str)>> = Some(vec![
            ("labels", label),
            ("state", "all"),
            ("per_page", "100"),
        ]);
        let mut out = Vec::new();
        loop {
            let (page, next): (Vec<RawIssue>, _) = self
                .get_page(&url, token, query.as_deref(), "issues_by_label_all")
                .await?;
            out.extend(
                page.into_iter()
                    .filter(|r| r.pull_request.is_none())
                    .map(|r| IssueSummary {
                        number: r.number,
                        title: r.title,
                        body: r.body,
                        labels: r.labels.into_iter().map(|l| l.name).collect(),
                        state: r.state,
                        assignees: Vec::new(),
                        user_login: r.user.login,
                        user_id: r.user.id,
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

// ---- Assembly + handler -----------------------------------------------------

/// Assemble the dashboard for `user_token`.
async fn assemble(state: &AppState, user_token: &SecretString) -> Result<DashboardView, AppError> {
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let installations = gh.user_installations(user_token).await?;
    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let app = state.github_app.as_ref();

    let mut repos = Vec::new();
    for inst in &installations {
        for repo in gh.user_installation_repos(user_token, inst.id).await? {
            let mut sessions = Vec::new();
            // Sessions can only be read when the App is configured (its installation
            // token reads the issues); otherwise the repo lists with no sessions.
            if let Some(app) = app {
                let owner_repo = format!("{}/{}", repo.owner, repo.name);
                let inst_token = app.token_for_repo(&owner_repo, None).await?;
                let triggers = gh
                    .issues_by_label_all(&inst_token, &repo.owner, &repo.name, trigger_label)
                    .await?;
                for trigger in &triggers {
                    match parse_registration(inst.id, &repo, trigger) {
                        Ok(reg) => {
                            let work = gh
                                .issues_by_label_all(
                                    &inst_token,
                                    &repo.owner,
                                    &repo.name,
                                    &reg.def.work_label,
                                )
                                .await?;
                            sessions.push(build_session(trigger, &reg, work));
                        }
                        Err((_, reason)) => sessions.push(build_invalid_session(trigger, reason)),
                    }
                }
            }
            repos.push(RepoView {
                owner: repo.owner,
                name: repo.name,
                installation_id: inst.id,
                sessions,
            });
        }
    }

    Ok(DashboardView {
        app_configured: app.is_some(),
        installations: installations.len(),
        repos,
    })
}

/// Pull the non-empty bearer token out of the `Authorization` header, or 401.
fn bearer_token(headers: &HeaderMap) -> Result<SecretString, AppError> {
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

/// `GET /api/v1/dashboard` — the signed-in user's fkst sessions across installed repos.
#[utoipa::path(
    get,
    path = "/dashboard",
    tag = "dashboard",
    operation_id = "get_dashboard",
    responses(
        (status = 200, description = "The user's installed repos + fkst sessions", body = DashboardView),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 503, description = "GitHub unreachable / rate limited", body = ErrorEnvelope),
    )
)]
async fn get_dashboard(
    State(state): State<AppState>,
    // The extractor verifies the token → identity (401 on a bad token) before any work.
    _user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<DashboardView>, AppError> {
    let token = bearer_token(&headers)?;
    Ok(Json(assemble(&state, &token).await?))
}

/// The dashboard router (nested under `/api/v1`). GitHub-token authenticated via the
/// `GithubUser` extractor, so no documented security scheme (like the env routes).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_dashboard))
}

#[cfg(test)]
#[path = "dashboard_tests.rs"]
mod tests;
