//! The canvas session mutations, both acting WITH THE USER TOKEN so the
//! signed-in human — not the App bot — is the actor GitHub records and the
//! identity the control plane trusts:
//!
//! - `POST /api/v1/repos/{owner}/{name}/sessions` opens the trigger issue
//!   (rendered + round-trip validated through the reconciler's parser first;
//!   the issue author becomes the session's authz owner).
//! - `DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}` closes the
//!   trigger issue — closing IS the stop/retire contract: the reconciler kills
//!   the pod and retire-notifies on its next sweep/webhook nudge.
//!
//! GitHub natively enforces the caller's permission on both writes; its
//! 403/404 map straight onto the error envelope.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::routes::canvas::sessions::validate_repo_segment;
use crate::routes::canvas::trigger_body::{validated_trigger_body, CreateSessionRequest};
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// The created trigger issue.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateSessionResponse {
    /// The trigger issue's number.
    pub issue_number: i64,
    /// The trigger issue's github.com URL.
    pub html_url: String,
}

/// `POST /api/v1/repos/{owner}/{name}/sessions` — open a trigger issue AS the
/// signed-in user.
#[utoipa::path(
    post,
    path = "/repos/{owner}/{name}/sessions",
    tag = "canvas",
    operation_id = "canvas_create_session",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
    ),
    request_body = CreateSessionRequest,
    responses(
        (status = 201, description = "The trigger issue was created", body = CreateSessionResponse),
        (status = 400, description = "Invalid session fields (the message names the offending trigger section), or GitHub rejected the issue", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted, or GitHub refused the write for this caller", body = ErrorEnvelope),
        (status = 404, description = "Repo not found (or the caller cannot see it)", body = ErrorEnvelope),
        (status = 422, description = "Issues are disabled on the repo", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn create_session(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    _user: GithubUser,
    headers: HeaderMap,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    // Validate BEFORE any GitHub write: the rendered body must round-trip
    // through the reconciler's own parser, or the 400 carries its message.
    let body = validated_trigger_body(&req)?;

    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let labels = vec![state.config.reconcile.substrate_trigger_label.clone()];
    let created = gh
        .create_issue(&token, &owner, &name, req.name.trim(), &body, &labels)
        .await?;
    tracing::info!(
        owner = %owner,
        name = %name,
        issue = created.number,
        "canvas: trigger issue created as the signed-in user"
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateSessionResponse {
            issue_number: created.number,
            html_url: created.html_url,
        }),
    ))
}

/// `DELETE /api/v1/repos/{owner}/{name}/sessions/{issue_number}` — close the
/// trigger issue AS the signed-in user (the stop/retire contract).
#[utoipa::path(
    delete,
    path = "/repos/{owner}/{name}/sessions/{issue_number}",
    tag = "canvas",
    operation_id = "canvas_stop_session",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("issue_number" = u64, Path, description = "The session's trigger-issue number"),
    ),
    responses(
        (status = 204, description = "The trigger issue was closed (the reconciler retires the session on its next pass)"),
        (status = 400, description = "Malformed owner/name/issue number", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted, or GitHub refused the close for this caller", body = ErrorEnvelope),
        (status = 404, description = "No such issue (or the caller cannot see the repo)", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn stop_session(
    State(state): State<AppState>,
    Path((owner, name, issue_number)): Path<(String, String, u64)>,
    _user: GithubUser,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    if issue_number == 0 {
        return Err(AppError::Validation(
            "issue_number must be a positive issue number".to_string(),
        ));
    }
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // Pre-flight: only ever close an actual trigger issue. GitHub's issues
    // PATCH also closes pull requests and any unrelated issue the caller can
    // write, so a stale/wrong number from the UI must not silently retire the
    // wrong thing — refuse anything that is a PR or lacks the trigger label.
    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let issue = gh.get_issue(&token, &owner, &name, issue_number).await?;
    if issue.is_pull_request {
        return Err(AppError::Validation(format!(
            "#{issue_number} is a pull request, not a session trigger issue"
        )));
    }
    if !issue.labels.iter().any(|label| label == trigger_label) {
        return Err(AppError::NotFound(format!(
            "#{issue_number} is not a session trigger issue (missing the {trigger_label} label)"
        )));
    }

    gh.close_issue(&token, &owner, &name, issue_number).await?;
    tracing::info!(
        owner = %owner,
        name = %name,
        issue = issue_number,
        "canvas: trigger issue closed as the signed-in user (session stop)"
    );
    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
#[path = "mutate_tests.rs"]
mod tests;
