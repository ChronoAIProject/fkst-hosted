//! The signed-in user's full repo listing with App-installation status (issue
//! #499): every repo the USER token can access — private, public, and
//! organization repos — each flagged with whether THIS GitHub App is installed
//! on it, so the dashboard can render per-repo status and the guided install
//! entry point (`https://github.com/apps/<slug>/installations/new` — GitHub
//! offers no API to create an installation; consent happens on github.com).
//!
//! - `GET /api/v1/repos` — computed live from GitHub on every call (no cache:
//!   the listing is one paginated `/user/repos` walk plus the installation
//!   enumeration the dashboard pull already does; freshness matters right
//!   after the user returns from installing).
//!
//! Auth mirrors the dashboard: the [`GithubUser`] extractor verifies the
//! caller's GitHub token (and enforces the deployment allowlist); the SAME
//! bearer token then drives the user-scoped GitHub reads.

use std::collections::HashSet;

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// One repo the signed-in user can access, with its App-installation status.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RepoStatus {
    /// The repo's immutable GitHub id (removal keys off it).
    pub id: i64,
    /// Repo owner login (user or organization).
    pub owner: String,
    /// Repo name.
    pub name: String,
    /// Private repo?
    pub private: bool,
    /// Owned by an organization (vs a user account)?
    pub org: bool,
    /// The caller has admin permission on the repo (an install attempt
    /// completes directly; without it GitHub raises an approval request to
    /// the owner instead).
    pub admin: bool,
    /// This GitHub App is installed on the repo.
    pub installed: bool,
}

/// The signed-in viewer, for the frontend's grouping + create-repo owner picker.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Viewer {
    /// The signed-in user's GitHub login.
    pub login: String,
}

/// The repo listing plus what the frontend needs to build the install link,
/// group by account, and offer create-repo targets.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReposResponse {
    /// The App's slug (install page: `https://github.com/apps/{app_slug}/installations/new`);
    /// null when the deployment has no App slug configured.
    pub app_slug: Option<String>,
    /// The signed-in user (the "personal" grouping/creation target).
    pub viewer: Viewer,
    /// Organizations the user belongs to (creation targets + empty groups),
    /// sorted.
    pub orgs: Vec<String>,
    /// This App's installations the user can see, one per connected account.
    pub installations: Vec<InstallationInfo>,
    /// Every repo the user can access, sorted by `owner/name`.
    pub repos: Vec<RepoStatus>,
}

/// One App installation the signed-in user can see (account-level connection).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct InstallationInfo {
    /// The connected account (user or organization login).
    pub account: String,
    /// The installation id (drives the Manage deep-link + uninstall).
    pub installation_id: i64,
    /// `"all"` or `"selected"` — whether the installation covers every repo.
    pub repository_selection: String,
}

/// Request body for creating a repository as the signed-in user.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateRepoRequest {
    /// Organization to create under; null (or the viewer's own login) creates
    /// under the personal account.
    pub owner: Option<String>,
    /// Repository name (`[A-Za-z0-9._-]+`, at most 100 chars).
    pub name: String,
    /// Create as a private repository.
    pub private: bool,
    /// Optional repository description.
    pub description: Option<String>,
}

/// `GET /api/v1/repos` — every repo the signed-in user can access, each flagged
/// with whether this App is installed on it. Computed live from GitHub.
#[utoipa::path(
    get,
    path = "/repos",
    tag = "repos",
    operation_id = "list_repos",
    responses(
        (status = 200, description = "All user-accessible repos with installation status", body = ReposResponse),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
    )
)]
async fn list_repos(
    State(state): State<AppState>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<ReposResponse>, AppError> {
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    let accessible = gh.user_all_repos(&token).await?;
    let insts = gh.user_installations(&token).await?;
    let mut installed: HashSet<String> = HashSet::new();
    for inst in &insts {
        for repo in gh.user_installation_repos(&token, inst.id).await? {
            installed.insert(format!("{}/{}", repo.owner, repo.name));
        }
    }
    let mut installations: Vec<InstallationInfo> = insts
        .into_iter()
        .map(|i| InstallationInfo {
            account: i.account,
            installation_id: i.id,
            repository_selection: i.repository_selection,
        })
        .collect();
    installations.sort_by(|a, b| a.account.cmp(&b.account));

    let mut repos: Vec<RepoStatus> = accessible
        .into_iter()
        .map(|r| RepoStatus {
            installed: installed.contains(&format!("{}/{}", r.owner, r.name)),
            id: r.id,
            owner: r.owner,
            name: r.name,
            private: r.private,
            org: r.org,
            admin: r.admin,
        })
        .collect();
    repos.sort_by(|a, b| (&a.owner, &a.name).cmp(&(&b.owner, &b.name)));

    let mut orgs = gh.user_orgs(&token).await?;
    orgs.sort();

    Ok(Json(ReposResponse {
        app_slug: state
            .github_app
            .as_ref()
            .and_then(|g| g.app_slug().map(str::to_string)),
        viewer: Viewer { login: user.login },
        orgs,
        installations,
        repos,
    }))
}

/// `POST /api/v1/repos` — create a repository AS the signed-in user (their
/// token; personal account or an organization they belong to). The created
/// repo starts without the App installed — the guided install flow follows.
#[utoipa::path(
    post,
    path = "/repos",
    tag = "repos",
    operation_id = "create_repo",
    request_body = CreateRepoRequest,
    responses(
        (status = 201, description = "The created repository", body = RepoStatus),
        (status = 400, description = "Invalid name / GitHub rejected the repository", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted, or GitHub refused creation (App may lack the Administration permission)", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
async fn create_repo(
    State(state): State<AppState>,
    user: GithubUser,
    headers: HeaderMap,
    Json(req): Json<CreateRepoRequest>,
) -> Result<(axum::http::StatusCode, Json<RepoStatus>), AppError> {
    let name = req.name.trim();
    let valid = !name.is_empty()
        && name.len() <= 100
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if !valid {
        return Err(AppError::Validation(
            "repository name must be 1-100 chars of [A-Za-z0-9._-]".to_string(),
        ));
    }
    // The viewer's own login as owner means the personal account.
    let org = req
        .owner
        .as_deref()
        .map(str::trim)
        .filter(|o| !o.is_empty() && !o.eq_ignore_ascii_case(&user.login));

    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let created = gh
        .create_repo(
            &token,
            org,
            name,
            req.private,
            req.description
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty()),
        )
        .await?;

    Ok((
        axum::http::StatusCode::CREATED,
        Json(RepoStatus {
            id: created.id,
            owner: created.owner,
            name: created.name,
            private: created.private,
            org: created.org,
            admin: created.admin,
            installed: false,
        }),
    ))
}

/// Resolve the caller-visible installation on `owner`, or 404.
async fn find_installation(
    gh: &DashboardGithub,
    token: &secrecy::SecretString,
    owner: &str,
) -> Result<crate::routes::dashboard::InstallationRef, AppError> {
    gh.user_installations(token)
        .await?
        .into_iter()
        .find(|i| i.account.eq_ignore_ascii_case(owner))
        .ok_or_else(|| AppError::NotFound(format!("no installation on {owner}")))
}

/// `DELETE /api/v1/installations/{owner}` — uninstall the App from an account
/// the caller can see (the App deletes its own installation via its JWT).
#[utoipa::path(
    delete,
    path = "/installations/{owner}",
    tag = "repos",
    operation_id = "uninstall_account",
    params(("owner" = String, Path, description = "Account (user or org) login")),
    responses(
        (status = 204, description = "Installation removed"),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted", body = ErrorEnvelope),
        (status = 404, description = "No installation on that account", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
async fn uninstall_account(
    State(state): State<AppState>,
    _user: GithubUser,
    headers: HeaderMap,
    Path(owner): Path<String>,
) -> Result<axum::http::StatusCode, AppError> {
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let inst = find_installation(&gh, &token, &owner).await?;
    let app = state
        .github_app
        .as_ref()
        .ok_or_else(|| AppError::Unavailable("github app is not configured".to_string()))?;
    let jwt = app
        .app_jwt()
        .map_err(|e| AppError::Internal(anyhow::anyhow!("app jwt mint: {e}")))?;
    gh.delete_installation(&jwt, inst.id).await?;
    Ok(axum::http::StatusCode::NO_CONTENT)
}

/// The repos router (nested under `/api/v1`). GitHub-token authenticated via
/// the `GithubUser` extractor (like the dashboard), so no documented security
/// scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_repos, create_repo))
        .routes(routes!(uninstall_account))
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
