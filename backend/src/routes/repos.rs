//! The signed-in user's repo mutations against GitHub (issue #503, #509): create
//! a repository as the user, and uninstall the App from an account. The read-side
//! repo listing is served by the canvas overview (`GET /api/v1/overview`), which
//! carries per-repo App-installation status — so there is no `GET /api/v1/repos`.
//! GitHub offers no API to CREATE an installation; consent happens on github.com
//! via the guided install link (`https://github.com/apps/<slug>/installations/new`,
//! built from `app_slug` in the overview payload).
//!
//! Auth: the [`GithubUser`] extractor verifies the caller's GitHub token (and
//! enforces the deployment allowlist); the SAME bearer token then drives the
//! user-scoped GitHub writes.

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
        .routes(routes!(create_repo))
        .routes(routes!(uninstall_account))
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
