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
use secrecy::SecretString;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::{GithubListing, HttpGithubListing};
use crate::github_identity::GithubUser;
use crate::goals::trigger_parse::parse_trigger_issue_body;
use crate::reconcile::{effective_creator, CreatorResolution};
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
        (status = 403, description = "Not allowlisted, the caller lacks admin/maintain permission on the repository, or GitHub refused the write for this caller", body = ErrorEnvelope),
        (status = 404, description = "Repo not found (or the caller cannot see it)", body = ErrorEnvelope),
        (status = 409, description = "The requested explicit work label collides with an existing open session on this repo (the message names the conflicting issue)", body = ErrorEnvelope),
        (status = 422, description = "A source/target branch is invalid, issues are disabled, or another semantic precondition failed", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable, or disposable-environment handoff unavailable on this control-plane process", body = ErrorEnvelope),
    )
)]
pub(super) async fn create_session(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    user: GithubUser,
    headers: HeaderMap,
    Json(req): Json<CreateSessionRequest>,
) -> Result<(StatusCode, Json<CreateSessionResponse>), AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    if let Some(disposable) = &req.disposable_environment {
        disposable.validate(&state.config)?;
    }
    // Validate BEFORE any GitHub write: the rendered body must round-trip
    // through the reconciler's own parser, or the 400 carries its message.
    let body = validated_trigger_body(&req)?;
    if req.disposable_environment.is_some() && state.reconciler.is_none() {
        return Err(AppError::Unavailable(
            "disposable environment handoff is unavailable because the session reconciler is not running"
                .to_string(),
        ));
    }

    ensure_session_creator_authorized(&state, &user, &owner, &name).await?;

    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let trigger_label = state.config.reconcile.substrate_trigger_label.clone();

    // Pre-flight (R4b): refuse a create that would collide with an existing OPEN
    // session's work label on this repo BEFORE opening the trigger issue —
    // immediate UX for the reconciler's authoritative R4a collision backstop.
    // Only runs when the request names an explicit work label (a request with no
    // explicit label has no known label pre-creation; see the helper's contract).
    if let Some(requested_label) = req
        .work_label
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        ensure_no_work_label_collision(
            &gh,
            &token,
            (&owner, &name),
            &trigger_label,
            requested_label,
            &user.login,
            state.config.reconcile.github_bot_login.as_deref(),
        )
        .await?;
    }

    let labels = vec![trigger_label];
    let created = gh
        .create_issue(&token, &owner, &name, req.name.trim(), &body, &labels, &[])
        .await?;
    if let Some(disposable) = &req.disposable_environment {
        state
            .disposable_environments
            .insert(&owner, &name, created.number, user.id, disposable);
    }
    tracing::info!(
        owner = %owner,
        name = %name,
        issue = created.number,
        disposable_environment = req.disposable_environment.is_some(),
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

/// Apply the reconciler's creator gate synchronously so a caller cannot create
/// a trigger that will be rejected one reconcile pass later. Global admins
/// short-circuit; everyone else must hold GitHub's `admin` or `maintain` role,
/// read with the App installation token (the same source of truth as reconcile).
async fn ensure_session_creator_authorized(
    state: &AppState,
    user: &GithubUser,
    owner: &str,
    name: &str,
) -> Result<(), AppError> {
    if state.config.access.is_global_admin(user.id, &user.login) {
        return Ok(());
    }

    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let installation_token = app.token_for_repo(&owner_repo, None).await?;
    let listing = HttpGithubListing::new(&state.config.github_api_base_url).map_err(|error| {
        tracing::warn!(error = %error, "canvas: creator-role client could not be initialized");
        AppError::Unavailable("github collaborator-role lookup is unavailable".to_string())
    })?;
    let role = listing
        .get_collaborator_role(&installation_token, owner, name, &user.login)
        .await
        .map_err(|error| {
            tracing::warn!(
                owner,
                name,
                creator = %user.login,
                error = %error,
                "canvas: creator-role lookup failed"
            );
            AppError::Unavailable("github collaborator-role lookup failed".to_string())
        })?;
    if matches!(role.as_deref(), Some("admin" | "maintain")) {
        return Ok(());
    }

    Err(AppError::Forbidden(format!(
        "@{} must have admin or maintain permission on {owner}/{name} to create a session",
        user.login
    )))
}

/// Reject a canvas create that would collide with an existing OPEN session's
/// work label for the same creator on the same repo — the immediate-UX pre-flight fronting the
/// reconciler's authoritative R4a collision backstop.
///
/// Contract: the caller invokes this ONLY when the new request names an explicit
/// `### Work Label`. A request with no explicit label has no comparable label
/// until it registers (its wake labels are auto-discovered from its packages
/// AFTER creation), so there is nothing to check here — the R4a backstop owns
/// that case.
///
/// It adds exactly ONE GitHub read: the repo's OPEN trigger issues, via the
/// caller's own user token (the same token that opens the issue). Each open
/// trigger is first attributed with the shared effective-creator rule, then
/// parsed and compared on its EXPLICIT work label; another creator's session,
/// an unattributable trigger, or a malformed body is skipped.
/// Existing sessions that rely solely on package-discovered labels are likewise
/// left to R4a — resolving those would fan out a manifest read per package,
/// which this single-read fast path deliberately avoids.
async fn ensure_no_work_label_collision(
    gh: &DashboardGithub,
    token: &SecretString,
    repo: (&str, &str),
    trigger_label: &str,
    requested_label: &str,
    creator_login: &str,
    bot_login: Option<&str>,
) -> Result<(), AppError> {
    let (owner, name) = repo;
    let open_triggers = gh
        .issues_by_label(token, owner, name, trigger_label, "open")
        .await?;
    for trigger in &open_triggers {
        let CreatorResolution::Resolved(existing_creator) =
            effective_creator(&trigger.summary.metadata(), bot_login)
        else {
            continue;
        };
        if !existing_creator.login.eq_ignore_ascii_case(creator_login) {
            continue;
        }
        // Tolerant parse: a trigger whose body no longer parses contributes no
        // comparable label (the reconciler flags it invalid on its own pass).
        let Ok(spec) = parse_trigger_issue_body(&trigger.summary.body) else {
            continue;
        };
        let existing = spec.work_label.as_deref().map(str::trim).unwrap_or("");
        if !existing.is_empty() && existing == requested_label {
            return Err(AppError::Conflict(format!(
                "work label \"{requested_label}\" is already in use by the open session \
                 #{} on {owner}/{name}; close that session or choose a different work label",
                trigger.summary.number
            )));
        }
    }
    Ok(())
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
        (status = 403, description = "Not allowlisted, the caller is not the session's trigger author nor a repo admin / org owner (session-management authority), or GitHub refused the close for this caller", body = ErrorEnvelope),
        (status = 404, description = "No such issue (or the caller cannot see the repo)", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn stop_session(
    State(state): State<AppState>,
    Path((owner, name, issue_number)): Path<(String, String, u64)>,
    user: GithubUser,
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

    // Request-time SESSION-MANAGEMENT authorization (R5, epic #572): stopping a
    // session is reserved to the caller who OWNS it — the trigger AUTHOR (matched
    // by immutable id, never the renamable login) — or a repo admin / org owner.
    // Session Collaborators hold WORK-ITEM authority only, never session
    // management, so they are deliberately NOT admitted here. Enforced ALWAYS (not
    // the reconciler's opt-in flag) so the canvas can never be a bypass around the
    // reconciler-side gate; the author tier short-circuits the admin lookup.
    let authorized =
        user.id == issue.author_id || gh.caller_is_repo_admin(&token, &owner, &name).await?;
    if !authorized {
        return Err(AppError::Forbidden(format!(
            "only the session's trigger author or a repo admin / org owner may stop \
             #{issue_number}"
        )));
    }

    gh.close_issue(&token, &owner, &name, issue_number).await?;
    if let Ok(issue_number) = i64::try_from(issue_number) {
        state
            .disposable_environments
            .remove_issue(&owner, &name, issue_number);
    }
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

#[cfg(test)]
#[path = "mutate_stop_tests.rs"]
mod stop_tests;
