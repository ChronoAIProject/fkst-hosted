//! Repo fkst-context setup API: `POST /api/v1/repos/:owner/:name/fkst-setup`.
//!
//! Initializes an EXISTING GitHub repo for fkst by committing a `.fkst/`
//! directory (an example package + the per-repo `AGENTS.md`) onto the repo's
//! DEFAULT branch, using the GitHub App installation token (the only credential
//! that holds `contents:write` — #110). Idempotent and non-destructive by
//! default: an already-initialized repo returns `200` with `already_initialized:
//! true` and no write; `?force=true` re-commits the three scaffold paths.
//!
//! Authorization follows the create-handler shape (like `goals::create`), NOT the
//! object layer: there is no pre-existing fkst resource for an arbitrary GitHub
//! repo, so the action-layer `fkst:repo:setup` permission plus the App's
//! installation probe (GitHub itself enforces repo ownership) is the gate. An
//! optional `org_id` adds an org-writer check.
//!
//! Security: the App token and the scaffold file CONTENTS are NEVER logged — only
//! the repo, the path count, and byte sizes (in the writer).

use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::AuthContext;
use crate::authz::permissions::{self, require_permission};
use crate::error::AppError;
use crate::github_app::{GithubAppError, GithubAppTokens, InstallationProbe};
use crate::github_hub::{commit_files, ContentsWriteError};
use crate::routes::extract::AppJson;
use crate::routes::repos_scaffold;
use crate::state::AppState;

/// The commit message for the scaffold (a single atomic commit).
const SCAFFOLD_COMMIT_MESSAGE: &str = "fkst: initialize fkst context";

/// The `.fkst` directory whose presence marks a repo as already initialized.
const FKST_DIR: &str = ".fkst";

/// Query parameters for the setup endpoint.
#[derive(Debug, Deserialize, Default)]
pub struct SetupQuery {
    /// Re-commit the scaffold over an existing `.fkst` (overwriting only the
    /// three scaffold paths, never deleting other `.fkst` content). Default
    /// (absent/false) is the safe, no-overwrite path.
    #[serde(default)]
    pub force: bool,
}

/// Optional request body. Mirrors `goals::create`'s `org_id`: when present, the
/// caller must be a writer of that org.
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SetupRequest {
    #[serde(default)]
    pub org_id: Option<String>,
}

/// `{ owner, name }` echoed back in the response.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct RepoRef {
    pub owner: String,
    pub name: String,
}

/// Response body for the setup endpoint.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct SetupResponse {
    pub repo: RepoRef,
    pub default_branch: Option<String>,
    pub commit_sha: Option<String>,
    pub created_paths: Vec<String>,
    pub already_initialized: bool,
}

/// `POST /repos/:owner/:name/fkst-setup`: scaffold `.fkst/` onto the repo's
/// default branch. See the module docs for the authorization model.
async fn fkst_setup(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    Query(query): Query<SetupQuery>,
    ctx: AuthContext,
    body: Option<AppJson<SetupRequest>>,
) -> Result<Response, AppError> {
    // Action layer: may the caller scaffold repos at all?
    require_permission(&ctx, permissions::REPO_SETUP)?;

    // Optional org-writer check (mirrors goals::create).
    let request = body.map(|AppJson(b)| b).unwrap_or_default();
    if let Some(ref org_id) = request.org_id {
        state.authz.require_org_writer(&ctx, org_id).await?;
    }

    // Validate owner/name against the same anchored GitHub regexes the goal path
    // uses, before any upstream call. Malformed input is a 400.
    validate_repo_ref(&owner, &name)?;

    // The App is the ONLY credential with contents:write. Disabled => 422 (we
    // must really write files; never silently succeed).
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unprocessable(
            "github app is not configured; cannot scaffold .fkst (set the GitHub App)".to_string(),
        )
    })?;

    let (status, response) = run_fkst_setup(app, &owner, &name, query.force).await?;
    Ok((status, Json(response)).into_response())
}

/// Core orchestration, decoupled from `AppState` so it is unit-testable against a
/// wiremock-backed [`GithubAppTokens`]: installation probe → idempotency probe →
/// (maybe) commit → response shaping. Returns the HTTP status and body.
async fn run_fkst_setup(
    app: &GithubAppTokens,
    owner: &str,
    name: &str,
    force: bool,
) -> Result<(StatusCode, SetupResponse), AppError> {
    let owner_repo = format!("{owner}/{name}");

    // Installation probe: the App must already be installed on the repo (GitHub
    // enforces repo ownership at install time). Not-installed => 422 + hint.
    match app.probe_installation(&owner_repo).await {
        Ok(InstallationProbe::Installed) => {}
        Ok(InstallationProbe::NotInstalled { install_url }) => {
            return Err(not_installed_error(&owner_repo, install_url));
        }
        Ok(InstallationProbe::AwaitingApproval) => {
            return Err(not_installed_error(&owner_repo, app.install_url()));
        }
        Err(error) => return Err(map_probe_error(&owner_repo, error)),
    }

    // Idempotency probe: is `.fkst` already present? A 404 (NotFound) means the
    // repo is fresh; an Ok means it already exists.
    let already = match app.get_contents(&owner_repo, FKST_DIR).await {
        Ok(_) => true,
        Err(GithubAppError::NotFound { .. }) => false,
        Err(GithubAppError::NotInstalled { install_url, .. }) => {
            return Err(not_installed_error(&owner_repo, install_url));
        }
        Err(error) => return Err(map_probe_error(&owner_repo, error)),
    };

    // Already initialized and not forcing: return 200 without writing anything.
    if already && !force {
        tracing::info!(
            owner_repo = %owner_repo,
            "fkst-setup: .fkst already present; no-op (use ?force=true to re-commit)"
        );
        return Ok((
            StatusCode::OK,
            SetupResponse {
                repo: RepoRef {
                    owner: owner.to_string(),
                    name: name.to_string(),
                },
                default_branch: None,
                commit_sha: None,
                created_paths: vec![],
                already_initialized: true,
            },
        ));
    }

    // Fresh repo (or forced re-commit): write the scaffold atomically.
    let files = repos_scaffold::scaffold_files();
    let result = commit_files(app, owner, name, SCAFFOLD_COMMIT_MESSAGE, &files)
        .await
        .map_err(|e| map_write_error(&owner_repo, e))?;

    // A fresh init is 201; a forced re-commit over existing `.fkst` is 200.
    let status = if already {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        SetupResponse {
            repo: RepoRef {
                owner: owner.to_string(),
                name: name.to_string(),
            },
            default_branch: Some(result.default_branch),
            commit_sha: Some(result.commit_sha),
            created_paths: repos_scaffold::scaffold_paths(),
            already_initialized: false,
        },
    ))
}

/// Validate `owner` / `name` against the same anchored GitHub regexes the goal
/// repo reference uses. A mismatch is a 400.
fn validate_repo_ref(owner: &str, name: &str) -> Result<(), AppError> {
    let goal_repo = crate::goals::RepoRef {
        owner: owner.to_string(),
        name: name.to_string(),
    };
    // Reuse the goal field validator (it validates the repo owner/name shape);
    // the title/description/package placeholders are valid and ignored here.
    crate::goals::validate_goal_fields("x", "x", &["x".to_string()], Some(&goal_repo))
        .map_err(AppError::Validation)
}

/// A 422 "App not installed" error carrying the actionable install hint.
fn not_installed_error(owner_repo: &str, install_url: Option<String>) -> AppError {
    let hint = install_url
        .map(|url| format!(" ({url})"))
        .unwrap_or_else(|| " (ask an admin to install the fkst-hosted GitHub App)".to_string());
    AppError::Unprocessable(format!(
        "github app is not installed on {owner_repo}; install it, then retry{hint}"
    ))
}

/// Map a probe/read `GithubAppError` onto the right HTTP status. The standard
/// `From` mapping already handles auth/rate-limit/etc.; a repo-not-found on the
/// read path surfaces as 422 (it cannot be reached without an installation).
fn map_probe_error(owner_repo: &str, error: GithubAppError) -> AppError {
    match error {
        GithubAppError::NotFound { .. } => AppError::Unprocessable(format!(
            "repository {owner_repo} not found or not visible to the fkst-hosted GitHub App"
        )),
        other => AppError::from(other),
    }
}

/// Map a [`ContentsWriteError`] onto the right HTTP status:
/// - `NotInstalled` → 422 with the install hint,
/// - `RepoNotFound` → 404,
/// - `Conflict`     → 409,
/// - `Upstream`     → 502 (bad gateway).
fn map_write_error(owner_repo: &str, error: ContentsWriteError) -> AppError {
    match error {
        ContentsWriteError::NotInstalled { install_url, .. } => {
            not_installed_error(owner_repo, install_url)
        }
        ContentsWriteError::RepoNotFound(repo) => {
            AppError::NotFound(format!("repository not found: {repo}"))
        }
        ContentsWriteError::Conflict(detail) => AppError::Conflict(format!(
            "the default branch moved during scaffolding ({detail}); retry"
        )),
        ContentsWriteError::Upstream(detail) => {
            tracing::error!(owner_repo = %owner_repo, detail = %detail, "fkst-setup upstream write error");
            AppError::Upstream("github rejected the scaffold commit".to_string())
        }
    }
}

/// Repo routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new().route("/repos/:owner/:name/fkst-setup", post(fkst_setup))
}

#[cfg(test)]
#[path = "repos_tests.rs"]
mod tests;
