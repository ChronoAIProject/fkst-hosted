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

use axum::extract::State;
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

/// The repo listing plus what the frontend needs to build the install link.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReposResponse {
    /// The App's slug (install page: `https://github.com/apps/{app_slug}/installations/new`);
    /// null when the deployment has no App slug configured.
    pub app_slug: Option<String>,
    /// Every repo the user can access, sorted by `owner/name`.
    pub repos: Vec<RepoStatus>,
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
    _user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<ReposResponse>, AppError> {
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    let accessible = gh.user_all_repos(&token).await?;
    let mut installed: HashSet<String> = HashSet::new();
    for inst in gh.user_installations(&token).await? {
        for repo in gh.user_installation_repos(&token, inst.id).await? {
            installed.insert(format!("{}/{}", repo.owner, repo.name));
        }
    }

    let mut repos: Vec<RepoStatus> = accessible
        .into_iter()
        .map(|r| RepoStatus {
            installed: installed.contains(&format!("{}/{}", r.owner, r.name)),
            owner: r.owner,
            name: r.name,
            private: r.private,
            org: r.org,
            admin: r.admin,
        })
        .collect();
    repos.sort_by(|a, b| (&a.owner, &a.name).cmp(&(&b.owner, &b.name)));

    Ok(Json(ReposResponse {
        app_slug: state
            .github_app
            .as_ref()
            .and_then(|g| g.app_slug().map(str::to_string)),
        repos,
    }))
}

/// The repos router (nested under `/api/v1`). GitHub-token authenticated via
/// the `GithubUser` extractor (like the dashboard), so no documented security
/// scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(list_repos))
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
