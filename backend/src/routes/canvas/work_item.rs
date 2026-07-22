//! Queue work for an existing session from the canvas:
//! `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items`.
//!
//! Opens a plain GitHub issue in the repo pre-stamped with one label selected
//! from the SESSION's full applicable set: its explicit `### Work Label` plus
//! labels discovered from the manifest-expanded package graph. The handler
//! re-resolves that set with the reconciler's own resolvers before writing, so
//! a client cannot stamp an unrelated label. Any applicable label wakes the
//! session on the next sweep.
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
use crate::goals::trigger_parse::{parse_trigger_issue_body, TriggerSpec};
use crate::models::{GithubActor, RepoRef};
use crate::reconcile::desired::{config_hash, SessionDef, SessionRegistration};
use crate::reconcile::effective_packages::resolve_effective_packages;
use crate::reconcile::work_authz::is_work_author_allowed;
use crate::reconcile::work_labels::resolve_work_label_sets;
use crate::routes::canvas::sessions::validate_repo_segment;
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// Request body for queuing a work item on a session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkItemRequest {
    /// The work-issue title (also the GitHub issue title); required, non-blank.
    pub title: String,
    /// One of the session's resolved applicable work labels. The server
    /// re-resolves the session and rejects labels outside that set.
    pub work_label: String,
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
    state: String,
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
                #[serde(default)]
                state: String,
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
                state: raw.state,
                is_pull_request: raw.pull_request.is_some(),
                author_id: raw.user.map(|user| user.id).unwrap_or_default(),
            });
        }
        Err(trigger_read_error(status, github_message(response).await))
    }
}

/// `POST /api/v1/repos/{owner}/{name}/sessions/{issue_number}/work-items` —
/// open a work issue AS the signed-in user, stamped with the selected applicable
/// session label so the reconciler claims it for that session.
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
        (status = 409, description = "The session trigger is closed", body = ErrorEnvelope),
        (status = 422, description = "The trigger issue or its package sources are malformed, the session has no applicable work labels, or the selected label is not applicable", body = ErrorEnvelope),
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
    let requested_work_label = req.work_label.trim();
    if requested_work_label.is_empty() {
        return Err(AppError::Validation(
            "work_label must not be blank".to_string(),
        ));
    }
    // An omitted or blank body opens a body-less issue. Preserve a populated
    // body's original whitespace because indentation is meaningful Markdown.
    let body = req
        .body
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();

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
    if trigger.state != "open" {
        return Err(AppError::Conflict(format!(
            "the session at #{issue_number} is not running because its trigger issue is closed"
        )));
    }

    // Parse with the reconciler's own grammar: a malformed trigger surfaces the
    // parser's section-naming 422 rather than silently mislabeling the work item.
    let spec = parse_trigger_issue_body(&trigger.body)?;
    let mut reg = work_registration(&owner, &name, issue_number, trigger.author_id, spec);

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
    if !is_work_author_allowed(&reg, &admins, user.id, &user.login) {
        return Err(AppError::Forbidden(format!(
            "your GitHub account lacks work-item authority on the session at \
             #{issue_number}: only its trigger author, a listed Session Collaborator, \
             or a repo admin / org owner may queue work items"
        )));
    }

    // Resolve the same effective package + label graph as the reconcile
    // wake-gate. The signed-in user's token can read the public package sources
    // and keeps this mutation independent of any client-supplied label list.
    let effective =
        resolve_effective_packages(&gh.client, &gh.api_base, &token, std::slice::from_ref(&reg))
            .await;
    let Some(packages) = effective.by_session.get(&reg.session_id) else {
        let reason = effective
            .demotions
            .iter()
            .find(|(issue, _)| *issue == reg.trigger_issue)
            .map(|(_, reason)| reason.as_str())
            .unwrap_or("the effective package set could not be resolved");
        return Err(AppError::Unprocessable(format!(
            "this session cannot accept work because {reason}"
        )));
    };
    reg.effective_packages = packages.clone();
    let mut label_sets =
        resolve_work_label_sets(&gh.client, &gh.api_base, &token, std::slice::from_ref(&reg)).await;
    let applicable = label_sets.remove(&reg.session_id).unwrap_or_default();
    if applicable.is_empty() {
        return Err(AppError::Unprocessable(
            "this session has no applicable work labels".to_string(),
        ));
    }
    let work_label = applicable
        .iter()
        .find(|label| label.eq_ignore_ascii_case(requested_work_label))
        .cloned()
        .ok_or_else(|| {
            AppError::Unprocessable(format!(
                "work label `{requested_work_label}` is not applicable to this session; \
                 refresh the dashboard and choose one of: {}",
                applicable.join(", ")
            ))
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

/// Reconstruct the trigger registration needed by both the request-time R3
/// authority gate and the reconciler's effective-package/work-label resolvers.
/// Identity fields that do not affect either operation remain inert, while the
/// complete parsed session definition is preserved for exact label discovery.
fn work_registration(
    owner: &str,
    name: &str,
    trigger_issue: u64,
    trigger_author_id: i64,
    spec: TriggerSpec,
) -> SessionRegistration {
    let hash = config_hash(
        &spec.packages,
        spec.work_label.as_deref(),
        spec.environment.as_deref(),
        spec.output_lang.as_deref(),
        &spec.engine_config,
        &spec.manifest_refs,
    );
    let effective_packages = spec.packages.clone();
    SessionRegistration {
        installation_id: 0,
        repo: RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        trigger_issue: trigger_issue as i64,
        trigger_author_id,
        trigger_author_login: String::new(),
        creator_login: String::new(),
        creator_id: Some(trigger_author_id),
        def: SessionDef {
            name: spec.name,
            packages: spec.packages,
            manifest_refs: spec.manifest_refs,
            work_label: spec.work_label,
            environment: spec.environment,
            output_lang: spec.output_lang,
            engine_config: spec.engine_config,
        },
        effective_packages,
        session_id: format!("canvas-work-item-{trigger_issue}"),
        config_hash: hash,
        auto_merge: spec.auto_merge,
        log_access: spec.log_access,
        collaborators: spec.collaborators,
    }
}

#[cfg(test)]
#[path = "work_item_tests.rs"]
mod tests;
