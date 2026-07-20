//! Queue work for an existing session from the canvas:
//! `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items`.
//!
//! Opens a plain GitHub issue in the repo pre-stamped with the SESSION's work
//! label — looked up by parsing the trigger issue `{issue_number}`'s body with
//! the reconciler's own trigger parser ([`parse_trigger_issue_body`]). The
//! reconciler then claims the new issue on its next sweep (any open issue
//! carrying a session's work label wakes that session), so a user never has to
//! leave the dashboard for GitHub just to add a task.
//!
//! Acts WITH THE USER TOKEN (like create/stop session): the signed-in human is
//! the issue author, and GitHub natively enforces whether they may write here.
//! The same anti-mistake pre-flight as stop-session guards the trigger number —
//! a PR or a non-trigger issue is refused before anything is created.

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::goals::trigger_parse::parse_trigger_issue_body;
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{SessionDef, SessionRegistration};
use crate::reconcile::work_authz::is_work_author_allowed;
use crate::routes::canvas::sessions::validate_repo_segment;
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// Request body for queuing a work item on a session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkItemRequest {
    /// The work-issue title (also the GitHub issue title); required, non-blank.
    pub title: String,
    /// The optional work-issue body (Markdown); an omitted or blank value opens
    /// a body-less issue.
    #[serde(default)]
    pub body: Option<String>,
}

/// The created work issue.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkItemResponse {
    /// The work issue's number.
    pub issue_number: i64,
    /// The work issue's github.com URL.
    pub html_url: String,
}

/// The trigger issue as this endpoint reads it: the body (to parse the session's
/// work label + collaborators out of), the label names (to prove it really is a
/// trigger), whether the "issue" is actually a pull request (GitHub's issues API
/// serves PRs too), and the trigger AUTHOR's immutable numeric id (the
/// request-time work-item gate authorizes the author by id).
struct FetchedTrigger {
    body: String,
    labels: Vec<String>,
    is_pull_request: bool,
    /// The trigger author's immutable numeric GitHub id (`0` if GitHub omits the
    /// `user` block — a fail-safe: id 0 matches no real caller).
    author_id: i64,
}

/// Pull GitHub's own `message` out of an error body without leaking anything
/// else; falls back to the bare status.
async fn github_message(response: reqwest::Response) -> String {
    let status = response.status();
    response
        .json::<serde_json::Value>()
        .await
        .ok()
        .and_then(|value| {
            value
                .get("message")
                .and_then(|message| message.as_str())
                .map(str::to_string)
        })
        .unwrap_or_else(|| format!("github returned status {status}"))
}

/// Map a failed trigger read onto the API error surface. GitHub answers 404 for
/// both "no such issue" and "no access" (anti-enumeration), so both surface as
/// not-found here — the same contract stop-session's pre-flight uses.
fn trigger_read_error(status: reqwest::StatusCode, message: String) -> AppError {
    match status.as_u16() {
        401 => AppError::Unauthorized(format!("github rejected the token: {message}")),
        403 => AppError::Forbidden(format!("GitHub refused the read: {message}")),
        404 => AppError::NotFound(format!("github get_issue: {message}")),
        _ => AppError::Unavailable(format!("github get_issue returned status {status}")),
    }
}

impl DashboardGithub {
    /// `GET /repos/{owner}/{repo}/issues/{number}` (user token) returning the
    /// full BODY — the work-item endpoint parses the session's work label out of
    /// it. The sibling [`DashboardGithub::get_issue`] deliberately drops the
    /// body (stop-session's pre-flight needs only labels), so this endpoint owns
    /// the body-bearing read it uniquely requires.
    async fn fetch_trigger(
        &self,
        user_token: &SecretString,
        owner: &str,
        repo: &str,
        number: u64,
    ) -> Result<FetchedTrigger, AppError> {
        let url = format!("{}/repos/{owner}/{repo}/issues/{number}", self.api_base);
        let response = self
            .client
            .get(&url)
            .bearer_auth(user_token.expose_secret())
            .header(reqwest::header::ACCEPT, "application/vnd.github+json")
            .send()
            .await
            .map_err(|e| {
                tracing::warn!(error = %e, "github get-trigger transport error");
                AppError::Unavailable("github get-issue request failed".to_string())
            })?;
        let status = response.status();
        if status.is_success() {
            #[derive(Deserialize)]
            struct RawLabel {
                name: String,
            }
            #[derive(Deserialize)]
            struct RawUser {
                #[serde(default)]
                id: i64,
            }
            #[derive(Deserialize)]
            struct RawIssue {
                /// GitHub sends `"body": null` for a body-less issue.
                #[serde(default)]
                body: Option<String>,
                #[serde(default)]
                labels: Vec<RawLabel>,
                /// Present only when this "issue" is actually a PR.
                pull_request: Option<serde_json::Value>,
                /// The trigger author; `None` only on a malformed response.
                user: Option<RawUser>,
            }
            let raw: RawIssue = response.json().await.map_err(|e| {
                tracing::warn!(error = %e, "github get-trigger response did not parse");
                AppError::Upstream("github get-issue response was malformed".to_string())
            })?;
            return Ok(FetchedTrigger {
                body: raw.body.unwrap_or_default(),
                labels: raw.labels.into_iter().map(|label| label.name).collect(),
                is_pull_request: raw.pull_request.is_some(),
                author_id: raw.user.map(|user| user.id).unwrap_or_default(),
            });
        }
        Err(trigger_read_error(status, github_message(response).await))
    }
}

/// `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items` —
/// open a work issue AS the signed-in user, stamped with the session's work
/// label so the reconciler claims it for that session.
#[utoipa::path(
    post,
    path = "/repos/{owner}/{name}/sessions/{issue_number}/work-items",
    tag = "canvas",
    operation_id = "canvas_create_work_item",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("issue_number" = u64, Path, description = "The session's trigger-issue number"),
    ),
    request_body = CreateWorkItemRequest,
    responses(
        (status = 201, description = "The work issue was created", body = CreateWorkItemResponse),
        (status = 400, description = "Malformed owner/name/issue number, a blank title, or GitHub rejected the issue", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted, the caller lacks work-item authority on this session (not the trigger author, a listed Session Collaborator, nor a repo admin / org owner), or GitHub refused the write for this caller", body = ErrorEnvelope),
        (status = 404, description = "No such trigger issue (or the caller cannot see the repo)", body = ErrorEnvelope),
        (status = 422, description = "The trigger issue is malformed, or the session has no explicit work label to queue against", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn create_work_item(
    State(state): State<AppState>,
    Path((owner, name, issue_number)): Path<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
    Json(req): Json<CreateWorkItemRequest>,
) -> Result<(StatusCode, Json<CreateWorkItemResponse>), AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    if issue_number == 0 {
        return Err(AppError::Validation(
            "issue_number must be a positive issue number".to_string(),
        ));
    }
    let title = req.title.trim();
    if title.is_empty() {
        return Err(AppError::Validation("title must not be blank".to_string()));
    }
    // An omitted or blank body opens a body-less issue.
    let body = req.body.as_deref().map(str::trim).unwrap_or_default();

    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // Resolve the SESSION's work label from its trigger issue, guarding the
    // number exactly like stop-session: refuse a PR or an issue lacking the
    // trigger label, so a stale/wrong number from the UI can never stamp a work
    // item against something that is not a session.
    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let trigger = gh
        .fetch_trigger(&token, &owner, &name, issue_number)
        .await?;
    if trigger.is_pull_request {
        return Err(AppError::Validation(format!(
            "#{issue_number} is a pull request, not a session trigger issue"
        )));
    }
    if !trigger.labels.iter().any(|label| label == trigger_label) {
        return Err(AppError::NotFound(format!(
            "#{issue_number} is not a session trigger issue (missing the {trigger_label} label)"
        )));
    }

    // Parse with the reconciler's own grammar: a malformed trigger surfaces the
    // parser's section-naming 422 rather than silently mislabeling the work item.
    let spec = parse_trigger_issue_body(&trigger.body)?;

    // Request-time WORK-ITEM authorization (R5, epic #572): only the session's
    // trigger AUTHOR, a listed Session Collaborator, or a repo admin / org owner
    // may queue work — the exact author ∪ collaborators ∪ admins predicate the
    // reconciler enforces (R3, [`is_work_author_allowed`]), reused verbatim so the
    // request-time and reconciler-side gates can never diverge. Enforced ALWAYS
    // (not the reconciler's opt-in flag) so the canvas can never bypass R3. Runs
    // BEFORE the work-label resolution so an unauthorized caller learns nothing
    // about the session's config. The admin argument reflects THIS caller: one
    // user-token repo read decides their repo-admin / org-owner tier.
    let caller_is_admin = gh.caller_is_repo_admin(&token, &owner, &name).await?;
    let admins = if caller_is_admin {
        vec![GithubActor {
            id: user.id,
            login: user.login.clone(),
        }]
    } else {
        Vec::new()
    };
    let reg = authz_registration(
        &owner,
        &name,
        issue_number,
        trigger.author_id,
        &spec.collaborators,
    );
    if !is_work_author_allowed(&reg, &admins, user.id, &user.login) {
        return Err(AppError::Forbidden(format!(
            "your GitHub account lacks work-item authority on the session at \
             #{issue_number}: only its trigger author, a listed Session Collaborator, \
             or a repo admin / org owner may queue work items"
        )));
    }

    let work_label = spec.work_label.ok_or_else(|| {
        // Auto-discovered sessions resolve their wake labels from package
        // manifests, which this endpoint deliberately does not fetch; without an
        // explicit label there is no single one to stamp, so ask for one.
        AppError::Unprocessable(
            "this session has no explicit `### Work Label`; add one to its trigger issue \
             to queue work items from the dashboard"
                .to_string(),
        )
    })?;

    let labels = vec![work_label.clone()];
    let created = gh
        .create_issue(&token, &owner, &name, title, body, &labels)
        .await?;
    tracing::info!(
        owner = %owner,
        name = %name,
        trigger = issue_number,
        work_issue = created.number,
        work_label = %work_label,
        "canvas: work item queued as the signed-in user"
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateWorkItemResponse {
            issue_number: created.number,
            html_url: created.html_url,
        }),
    ))
}

/// Build the minimal [`SessionRegistration`] the R3 work-authority predicate
/// reads. [`is_work_author_allowed`] consults ONLY `trigger_author_id` and
/// `collaborators` (plus the caller-side `admins` argument), so every other field
/// is an inert placeholder here — this endpoint reconstructs just enough of a
/// registration to reuse the reconciler's exact predicate rather than
/// re-implementing the tiering, keeping the request-time and reconciler-side gates
/// byte-identical.
fn authz_registration(
    owner: &str,
    name: &str,
    trigger_issue: u64,
    trigger_author_id: i64,
    collaborators: &[String],
) -> SessionRegistration {
    SessionRegistration {
        installation_id: 0,
        repo: RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        trigger_issue: trigger_issue as i64,
        trigger_author_id,
        trigger_author_login: String::new(),
        def: SessionDef {
            name: String::new(),
            packages: Vec::new(),
            manifest_refs: Vec::new(),
            work_label: None,
            environment: None,
            output_lang: None,
            engine_config: std::collections::BTreeMap::new(),
        },
        effective_packages: Vec::new(),
        session_id: String::new(),
        config_hash: String::new(),
        auto_merge: false,
        log_access: Vec::new(),
        collaborators: collaborators.to_vec(),
    }
}

#[cfg(test)]
#[path = "work_item_tests.rs"]
mod tests;
