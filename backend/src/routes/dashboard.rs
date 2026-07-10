//! The signed-in user's fkst sessions across the repos where the fkst-hosted App is
//! installed — a per-user CACHED dashboard, recomputed only on an explicit pull.
//!
//! - `GET /api/v1/dashboard` serves the cached result (`dashboards/<user_id>.json`),
//!   empty until the first pull.
//! - `POST /api/v1/dashboard/pull` starts a detached pull and returns a job id.
//! - `GET /api/v1/dashboard/pull/{job_id}` polls the job's progress/state.
//!
//! The pull:
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
//! 3. Progress is written to the job object (`dashboards/jobs/<job_id>.json`) after
//!    every repo — storage-backed, not in-memory, so the frontend's progress polling
//!    works across the deployment's replicas.
//!
//! Self-contained on purpose: it does NOT extend the reconciler's `GithubListing`
//! trait (whose `list_issues_by_label` is `state=open` and which has test doubles);
//! the user-token endpoints + `state=all` reads live here.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::listing::IssueSummary;
use crate::github_app::GithubAppTokens;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::reconcile::desired::SessionRegistration;
use crate::reconcile::registry::parse_registration;
use crate::state::AppState;
use crate::storage::{ChronoStorageClient, StorageError};

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

// ---- Cache + async pull job -------------------------------------------------
//
// The dashboard is served from a per-user cache (`dashboards/<user_id>.json` in
// object storage) and only recomputed when the user starts a pull. The pull runs
// as a detached task that writes its progress to a job object
// (`dashboards/jobs/<job_id>.json`) after each repo — storage-backed (not in-memory)
// so the frontend's progress polling works across the deployment's replicas.

/// The cached dashboard returned by `GET /dashboard`: the last-pull time (epoch ms,
/// UTC — the frontend renders it in SGT) + the dashboard; both null before the first pull.
#[derive(Debug, Serialize, ToSchema)]
pub struct DashboardResponse {
    /// Epoch milliseconds (UTC) of the last successful pull; null before the first.
    pub last_pulled_at_ms: Option<i64>,
    /// The cached dashboard; null before the first pull.
    pub dashboard: Option<DashboardView>,
}

/// The status of an async pull job — polled by the frontend to drive the progress bar.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PullJob {
    pub job_id: String,
    /// The owning user's GitHub id (authorizes status reads).
    pub user_id: i64,
    /// `running` | `done` | `error`.
    pub state: String,
    /// Human phase label for the progress bar.
    pub phase: String,
    /// Repos scanned so far.
    pub done: usize,
    /// Total repos to scan (0 until the repo list is known).
    pub total: usize,
    /// Failure detail when `state == "error"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn result_key(user_id: i64) -> String {
    format!("dashboards/{user_id}.json")
}
fn job_key(job_id: &str) -> String {
    format!("dashboards/jobs/{job_id}.json")
}

/// Read + JSON-decode a stored object; `Ok(None)` on a 404 (never written).
async fn read_json<T: serde::de::DeserializeOwned>(
    storage: &ChronoStorageClient,
    key: &str,
) -> Result<Option<T>, AppError> {
    match storage.download(key).await {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes).map_err(|e| {
            AppError::Internal(anyhow::anyhow!("dashboard cache decode: {e}"))
        })?)),
        Err(StorageError::Status { status: 404 }) => Ok(None),
        Err(e) => {
            tracing::warn!(key, error = %e, "dashboard storage read failed");
            Err(AppError::Unavailable(
                "dashboard storage unavailable".to_string(),
            ))
        }
    }
}

/// JSON-encode + write an object.
async fn write_json<T: Serialize>(
    storage: &ChronoStorageClient,
    key: &str,
    value: &T,
) -> Result<(), AppError> {
    let bytes = serde_json::to_vec(value)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("dashboard cache encode: {e}")))?;
    storage
        .upload(key, Bytes::from(bytes), "application/json")
        .await
        .map_err(|e| {
            tracing::warn!(key, error = %e, "dashboard storage write failed");
            AppError::Unavailable("dashboard storage unavailable".to_string())
        })?;
    Ok(())
}

/// Scan one installed repo for its fkst sessions (trigger issues grouped with their
/// work-label issues, open AND closed). Empty when the App is unconfigured.
async fn scan_repo_sessions(
    gh: &DashboardGithub,
    app: Option<&GithubAppTokens>,
    installation_id: i64,
    repo: &RepoRef,
    trigger_label: &str,
) -> Result<Vec<SessionGroup>, AppError> {
    let Some(app) = app else {
        return Ok(Vec::new());
    };
    let owner_repo = format!("{}/{}", repo.owner, repo.name);
    let inst_token = app.token_for_repo(&owner_repo, None).await?;
    let triggers = gh
        .issues_by_label_all(&inst_token, &repo.owner, &repo.name, trigger_label)
        .await?;
    let mut sessions = Vec::new();
    for trigger in &triggers {
        match parse_registration(installation_id, repo, trigger) {
            Ok(reg) => {
                let work = gh
                    .issues_by_label_all(&inst_token, &repo.owner, &repo.name, &reg.def.work_label)
                    .await?;
                sessions.push(build_session(trigger, &reg, work));
            }
            Err((_, reason)) => sessions.push(build_invalid_session(trigger, reason)),
        }
    }
    Ok(sessions)
}

/// The background pull: enumerate the user's installed repos, scan each (persisting
/// progress after every repo), then write the assembled dashboard to the user's cache.
/// Detached — failures are recorded on the job object, not propagated.
async fn run_pull(state: AppState, token: SecretString, user_id: i64, job_id: String) {
    let Some(storage) = state.storage.clone() else {
        return;
    };
    let mut job = PullJob {
        job_id: job_id.clone(),
        user_id,
        state: "running".to_string(),
        phase: "listing installations".to_string(),
        done: 0,
        total: 0,
        error: None,
    };

    let result: Result<DashboardView, AppError> = async {
        let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
        let installs = gh.user_installations(&token).await?;
        job.phase = "listing repositories".to_string();
        let _ = write_json(&storage, &job_key(&job_id), &job).await;

        let mut pairs: Vec<(i64, RepoRef)> = Vec::new();
        for inst in &installs {
            for repo in gh.user_installation_repos(&token, inst.id).await? {
                pairs.push((inst.id, repo));
            }
        }
        job.total = pairs.len();
        job.phase = "scanning sessions".to_string();
        let _ = write_json(&storage, &job_key(&job_id), &job).await;

        let trigger_label = &state.config.reconcile.substrate_trigger_label;
        let app = state.github_app.as_ref();
        let mut repos = Vec::with_capacity(pairs.len());
        for (installation_id, repo) in pairs {
            let sessions =
                scan_repo_sessions(&gh, app, installation_id, &repo, trigger_label).await?;
            repos.push(RepoView {
                owner: repo.owner,
                name: repo.name,
                installation_id,
                sessions,
            });
            job.done += 1;
            let _ = write_json(&storage, &job_key(&job_id), &job).await;
        }
        Ok(DashboardView {
            app_configured: state.github_app.is_some(),
            installations: installs.len(),
            repos,
        })
    }
    .await;

    match result {
        Ok(view) => {
            let cached = DashboardResponse {
                last_pulled_at_ms: Some(now_millis()),
                dashboard: Some(view),
            };
            if let Err(e) = write_json(&storage, &result_key(user_id), &cached).await {
                job.state = "error".to_string();
                job.error = Some(format!("{e}"));
            } else {
                job.state = "done".to_string();
                job.phase = "done".to_string();
            }
        }
        Err(e) => {
            job.state = "error".to_string();
            job.error = Some(format!("{e}"));
        }
    }
    let _ = write_json(&storage, &job_key(&job_id), &job).await;
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

/// `GET /api/v1/dashboard` — the CACHED dashboard for the signed-in user (empty until
/// the first pull). Never recomputes; a pull is started explicitly via `POST .../pull`.
#[utoipa::path(
    get,
    path = "/dashboard",
    tag = "dashboard",
    operation_id = "get_dashboard",
    responses(
        (status = 200, description = "The cached dashboard (nulls until the first pull)", body = DashboardResponse),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 503, description = "Dashboard storage is not configured / unavailable", body = ErrorEnvelope),
    )
)]
async fn get_dashboard(
    State(state): State<AppState>,
    user: GithubUser,
) -> Result<Response, AppError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("dashboard storage is not configured".to_string()))?;
    match storage.download(&result_key(user.id)).await {
        // Serve the stored bytes verbatim (already a DashboardResponse).
        Ok(bytes) => Ok(([(header::CONTENT_TYPE, "application/json")], bytes).into_response()),
        Err(StorageError::Status { status: 404 }) => Ok(Json(DashboardResponse {
            last_pulled_at_ms: None,
            dashboard: None,
        })
        .into_response()),
        Err(e) => {
            tracing::warn!(user_id = user.id, error = %e, "dashboard cache read failed");
            Err(AppError::Unavailable(
                "dashboard storage unavailable".to_string(),
            ))
        }
    }
}

/// `POST /api/v1/dashboard/pull` — start a background pull of the user's dashboard and
/// return the job to poll. The pull writes progress to the job object as it scans.
#[utoipa::path(
    post,
    path = "/dashboard/pull",
    tag = "dashboard",
    operation_id = "start_dashboard_pull",
    responses(
        (status = 202, description = "Pull started; poll the returned job id", body = PullJob),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 503, description = "Dashboard storage is not configured", body = ErrorEnvelope),
    )
)]
async fn start_pull(
    State(state): State<AppState>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<PullJob>), AppError> {
    let token = bearer_token(&headers)?;
    let storage = state
        .storage
        .clone()
        .ok_or_else(|| AppError::Unavailable("dashboard storage is not configured".to_string()))?;
    let job_id = format!("{}-{}", user.id, now_millis());
    let job = PullJob {
        job_id: job_id.clone(),
        user_id: user.id,
        state: "running".to_string(),
        phase: "starting".to_string(),
        done: 0,
        total: 0,
        error: None,
    };
    // Persist the job BEFORE spawning so an immediate status poll always finds it.
    write_json(&storage, &job_key(&job_id), &job).await?;
    let task_state = state.clone();
    let task_job_id = job_id.clone();
    let user_id = user.id;
    tokio::spawn(async move { run_pull(task_state, token, user_id, task_job_id).await });
    Ok((StatusCode::ACCEPTED, Json(job)))
}

/// `GET /api/v1/dashboard/pull/{job_id}` — poll a pull job's progress/state.
#[utoipa::path(
    get,
    path = "/dashboard/pull/{job_id}",
    tag = "dashboard",
    operation_id = "dashboard_pull_status",
    params(("job_id" = String, Path, description = "The pull job id from POST /dashboard/pull")),
    responses(
        (status = 200, description = "The pull job's current progress/state", body = PullJob),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "The job belongs to a different user, or the verified GitHub identity is not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 404, description = "Unknown job id", body = ErrorEnvelope),
    )
)]
async fn pull_status(
    State(state): State<AppState>,
    user: GithubUser,
    Path(job_id): Path<String>,
) -> Result<Json<PullJob>, AppError> {
    let storage = state
        .storage
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("dashboard storage is not configured".to_string()))?;
    let job: PullJob = read_json(storage, &job_key(&job_id))
        .await?
        .ok_or_else(|| AppError::NotFound("no such pull job".to_string()))?;
    if job.user_id != user.id {
        return Err(AppError::Forbidden("not your pull job".to_string()));
    }
    Ok(Json(job))
}

/// The dashboard router (nested under `/api/v1`). GitHub-token authenticated via the
/// `GithubUser` extractor, so no documented security scheme (like the env routes).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_dashboard))
        .routes(routes!(start_pull))
        .routes(routes!(pull_status))
}

#[cfg(test)]
#[path = "dashboard_tests.rs"]
mod tests;
