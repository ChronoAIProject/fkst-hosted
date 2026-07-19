//! A session's OUTCOME files, grouped by devloop PR, plus a raw blob-stream for
//! previewing/downloading one committed file (`/api/v1/repos/{owner}/{name}/…`):
//!
//! - `GET …/sessions/{issue_number}/outcomes` — the session's devloop PRs (the
//!   SAME grouping the sessions endpoint uses: bot-authored PRs whose linked
//!   work-issue belongs to the session's work label), each with its changed-file
//!   list. Best-effort per PR: a failed file fetch flags that one PR
//!   `files_error: true` rather than failing the whole response.
//! - `GET …/blob/{sha}?name=&download=` — one committed file's RAW bytes, for a
//!   media/text preview or a download. Content-Type is guessed from `name`'s
//!   extension; over [`MAX_BLOB_BYTES`] answers 413.
//!
//! Access scoping mirrors [`super::sessions`]: identity via the [`GithubUser`]
//! extractor, then the CALLER's own App installations must cover the repo (a repo
//! the caller cannot see renders 404, never another user's data). PR-file fetches
//! run under [`OUTCOME_FILE_CONCURRENCY`] bounded concurrency.

use std::collections::HashSet;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use futures::stream::{self, StreamExt};
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::GithubAppError;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::reconcile::registry::parse_registration;
use crate::routes::canvas::sessions::{devloop_prs, validate_repo_segment};
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// Max bytes the blob-stream endpoint will buffer + serve (25 MiB). A larger
/// blob answers 413 — the frontend then links out to GitHub.
#[cfg(not(test))]
const MAX_BLOB_BYTES: usize = 25 * 1024 * 1024;
/// Tests use a tiny cap so a small fixture can exercise the 413 path (the
/// mechanism under test is identical); a happy-path fixture stays well under it.
#[cfg(test)]
const MAX_BLOB_BYTES: usize = 64;

/// At most this many per-PR file fetches run concurrently within one outcomes call.
const OUTCOME_FILE_CONCURRENCY: usize = 6;

/// A session's outcome files, grouped by devloop PR.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionOutcomes {
    /// GitHub's canonical owner casing.
    pub owner: String,
    /// GitHub's canonical repo-name casing.
    pub name: String,
    /// The trigger issue number identifying the session in-repo.
    pub trigger_issue: i64,
    /// One entry per devloop PR belonging to this session.
    pub prs: Vec<PrOutcome>,
}

/// One devloop PR with its changed-file list.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrOutcome {
    pub number: i64,
    pub title: String,
    pub html_url: String,
    /// `open` or `closed`.
    pub state: String,
    pub merged: bool,
    /// The linked work-issue number; null when it does not fit the issue domain.
    pub work_issue: Option<i64>,
    /// The PR's changed files (empty when `files_error`).
    pub files: Vec<OutcomeFile>,
    /// True when fetching this PR's file list failed (the rest still render).
    pub files_error: bool,
}

/// One changed file of a devloop PR.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OutcomeFile {
    pub filename: String,
    /// `added`/`modified`/`removed`/`renamed`/`copied`/`changed`.
    pub status: String,
    pub additions: i64,
    pub deletions: i64,
    /// The file's blob sha at the PR head (the handle the blob endpoint reads).
    pub sha: String,
    /// The prior path, present only for a rename.
    pub previous_filename: Option<String>,
    /// Coarse media class guessed from the extension: `text`/`image`/`video`/`audio`/`binary`.
    pub kind: String,
    /// `additions + deletions` for a text file; null for binary/media.
    pub size_hint: Option<i64>,
}

/// Query for the blob-stream endpoint.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BlobQuery {
    /// The filename — drives the `Content-Type` guess and the download name.
    pub name: Option<String>,
    /// `1` forces a download (`attachment`); anything else previews inline.
    pub download: Option<u8>,
}

/// `GET /api/v1/repos/{owner}/{name}/sessions/{issue_number}/outcomes`.
#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/sessions/{issue_number}/outcomes",
    tag = "canvas",
    operation_id = "canvas_session_outcomes",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("issue_number" = i64, Path, description = "The session's trigger issue number"),
    ),
    responses(
        (status = 200, description = "The session's devloop PRs with their changed files", body = SessionOutcomes),
        (status = 400, description = "Malformed owner/name", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 404, description = "Repo not visible to the caller, or no such trigger issue", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn session_outcomes(
    State(state): State<AppState>,
    Path((owner, name, issue_number)): Path<(String, String, i64)>,
    _user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<SessionOutcomes>, AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    let (installation_id, repo_ref) = resolve_installed_repo(&gh, &token, &owner, &name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{owner}/{name}: not found")))?;
    let owner = repo_ref.owner.clone();
    let name = repo_ref.name.clone();

    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let inst_token = app.token_for_repo(&owner_repo, None).await?;

    // Locate the trigger issue among the repo's trigger-labelled issues, so the
    // session groups exactly as the sessions endpoint / reconciler sees it.
    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let triggers = gh
        .issues_by_label_all(&inst_token, &owner, &name, trigger_label)
        .await?;
    let trigger = triggers
        .iter()
        .find(|issue| issue.summary.number == issue_number)
        .ok_or_else(|| {
            AppError::NotFound(format!("no trigger issue #{issue_number} in {owner_repo}"))
        })?;

    // Resolve this session's devloop PRs the same way the sessions endpoint does:
    // work-label issues → work-issue numbers → bot devloop PRs linked to them. A
    // malformed trigger (no parseable work label) simply has no outcomes.
    let session_prs = match parse_registration(installation_id, &repo_ref, &trigger.summary) {
        Ok(reg) => {
            let work = match reg.def.work_label.as_deref() {
                Some(label) => {
                    gh.issues_by_label_all(&inst_token, &owner, &name, label)
                        .await?
                }
                None => Vec::new(),
            };
            let work_numbers: HashSet<i64> =
                work.iter().map(|issue| issue.summary.number).collect();
            let pulls = gh.list_pulls_all(&inst_token, &owner, &name).await?;
            devloop_prs(&pulls, state.config.reconcile.github_bot_login.as_deref())
                .into_iter()
                .filter(|pr| pr.work_issue.is_some_and(|n| work_numbers.contains(&n)))
                .collect::<Vec<_>>()
        }
        Err((_, reason)) => {
            tracing::debug!(owner = %owner, name = %name, issue = issue_number, reason = %reason, "outcomes: trigger body invalid; no PRs");
            Vec::new()
        }
    };

    // Fetch each PR's file list under bounded concurrency, best-effort per PR.
    // Each job OWNS its `PrDetail` (moved via `into_iter`) so the futures borrow
    // nothing from the loop, only the shared `&app` — a single concrete lifetime.
    let mut slots: Vec<Option<PrOutcome>> = (0..session_prs.len()).map(|_| None).collect();
    let jobs = session_prs.into_iter().enumerate().map(|(idx, pr)| {
        let owner_repo = owner_repo.clone();
        async move {
            let (files, files_error) = match app.list_pull_files(&owner_repo, pr.number).await {
                Ok(metas) => (metas.iter().map(outcome_file).collect(), false),
                Err(error) => {
                    tracing::warn!(pr = pr.number, error = %error, "outcomes: list_pull_files failed; flagging files_error");
                    (Vec::new(), true)
                }
            };
            (
                idx,
                PrOutcome {
                    number: pr.number,
                    title: pr.title,
                    html_url: pr.html_url,
                    state: pr.state,
                    merged: pr.merged,
                    work_issue: pr.work_issue,
                    files,
                    files_error,
                },
            )
        }
    });
    let results: Vec<(usize, PrOutcome)> = stream::iter(jobs)
        .buffer_unordered(OUTCOME_FILE_CONCURRENCY)
        .collect()
        .await;
    for (idx, outcome) in results {
        slots[idx] = Some(outcome);
    }
    let prs: Vec<PrOutcome> = slots.into_iter().flatten().collect();

    tracing::debug!(owner = %owner, name = %name, issue = issue_number, prs = prs.len(), "canvas session outcomes assembled");
    Ok(Json(SessionOutcomes {
        owner,
        name,
        trigger_issue: issue_number,
        prs,
    }))
}

/// `GET /api/v1/repos/{owner}/{name}/blob/{sha}?name=&download=` — stream one
/// committed file's raw bytes.
#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/blob/{sha}",
    tag = "canvas",
    operation_id = "canvas_outcome_blob",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("sha" = String, Path, description = "The file's git blob sha"),
        BlobQuery,
    ),
    responses(
        (status = 200, description = "The file's raw bytes (Content-Type guessed from ?name)", content_type = "application/octet-stream"),
        (status = 400, description = "Malformed owner/name/sha", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted", body = ErrorEnvelope),
        (status = 404, description = "Repo not visible to the caller, or no such blob", body = ErrorEnvelope),
        (status = 413, description = "The blob exceeds the previewable size cap"),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn outcome_blob(
    State(state): State<AppState>,
    Path((owner, name, sha)): Path<(String, String, String)>,
    Query(query): Query<BlobQuery>,
    _user: GithubUser,
    headers: HeaderMap,
) -> Response {
    match blob_bytes(&state, &owner, &name, &sha, &headers).await {
        Ok(bytes) => blob_response(bytes, query),
        Err(BlobError::TooLarge) => (
            StatusCode::PAYLOAD_TOO_LARGE,
            "file is too large to preview; open it on GitHub",
        )
            .into_response(),
        Err(BlobError::App(err)) => err.into_response(),
    }
}

/// Blob-fetch failure: a too-large blob (→ 413) distinct from the generic
/// [`AppError`] surface (the raw endpoint cannot render the JSON envelope for the
/// 413, so it is carried separately).
enum BlobError {
    TooLarge,
    App(AppError),
}

impl From<AppError> for BlobError {
    fn from(err: AppError) -> Self {
        BlobError::App(err)
    }
}

/// Validate + scope + fetch the blob bytes (shared by [`outcome_blob`]).
async fn blob_bytes(
    state: &AppState,
    owner: &str,
    name: &str,
    sha: &str,
    headers: &HeaderMap,
) -> Result<Vec<u8>, BlobError> {
    validate_repo_segment(owner, "owner").map_err(BlobError::App)?;
    validate_repo_segment(name, "name").map_err(BlobError::App)?;
    validate_blob_sha(sha).map_err(BlobError::App)?;
    let token = bearer_token(headers).map_err(BlobError::App)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url).map_err(BlobError::App)?;

    let (_installation_id, repo_ref) = resolve_installed_repo(&gh, &token, owner, name)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("{owner}/{name}: not found")))?;

    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{}/{}", repo_ref.owner, repo_ref.name);
    match app.get_blob_raw(&owner_repo, sha, MAX_BLOB_BYTES).await {
        Ok(bytes) => Ok(bytes),
        Err(GithubAppError::BlobTooLarge) => Err(BlobError::TooLarge),
        Err(err) => Err(BlobError::App(err.into())),
    }
}

/// Build the raw-bytes response with a guessed `Content-Type` and an inline /
/// attachment `Content-Disposition`.
fn blob_response(bytes: Vec<u8>, query: BlobQuery) -> Response {
    let name = query.name.as_deref().unwrap_or("");
    let content_type = content_type_for(name);
    let disposition = disposition_header(name, query.download == Some(1));
    let mut response = (StatusCode::OK, bytes).into_response();
    let headers = response.headers_mut();
    headers.insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    if let Ok(value) = HeaderValue::from_str(&disposition) {
        headers.insert(header::CONTENT_DISPOSITION, value);
    }
    response
}

/// Resolve the caller-visible installation + canonical repo casing. `Ok(None)`
/// when the caller's App installations do not cover `owner/name` (the access
/// gate on another user's data).
async fn resolve_installed_repo(
    gh: &DashboardGithub,
    token: &SecretString,
    owner: &str,
    name: &str,
) -> Result<Option<(i64, RepoRef)>, AppError> {
    let installations = gh.user_installations(token).await?;
    let Some(installation) = installations
        .iter()
        .find(|inst| inst.account.eq_ignore_ascii_case(owner))
    else {
        return Ok(None);
    };
    let canonical = gh
        .user_installation_repos(token, installation.id)
        .await?
        .into_iter()
        .find(|repo| {
            repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
        });
    Ok(canonical.map(|repo| (installation.id, repo)))
}

#[path = "outcomes_media.rs"]
mod media;
use media::{content_type_for, disposition_header, outcome_file, validate_blob_sha};

#[cfg(test)]
#[path = "outcomes_tests.rs"]
mod tests;
