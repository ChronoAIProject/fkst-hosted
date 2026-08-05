//! The schedules surface: six operations over a repository's scheduled workflows.
//!
//! A pure PROJECTION plus three durable state changes. Nothing here is stored:
//! the reads derive every field from the definition issue and its run records, and
//! the writes are GitHub label changes and one comment — the same durable state
//! the reconciler's clock reads on its next pass. That is what makes the dashboard
//! and the clock incapable of disagreeing, and what lets a run started from the UI
//! be indistinguishable downstream from one the clock started.
//!
//! Authorization reuses the two tiers the sibling canvas operations already
//! define, rather than inventing a third:
//!
//! - READS: repository visibility (`resolve_visible_repo`), exactly as
//!   `repo_sessions`. A caller sees schedules on repositories they can already see.
//! - WRITES: the definition issue's author OR a repository admin / org owner,
//!   exactly as `stop_session`. Pausing someone else's schedule is a management
//!   action, not a work-item one.

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use k8s_openapi::chrono::{DateTime, Utc};
use secrecy::SecretString;

use crate::audit::arguments::AuditedPath;
use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::comments::{HttpIssueCommentReader, IssueCommentReader};
use crate::github_identity::GithubUser;
use crate::goals::scheduled_workflow_parse::{parse_scheduled_workflow, ScheduledWorkflowSpec};
use crate::reconcile::reserved_labels::{
    CRON_PAUSED_LABEL, CRON_RUNNING_LABEL, SCHEDULED_WORKFLOW_LABEL,
};
use crate::reconcile::schedule_pass::comment_is_from_bot;
use crate::reconcile::schedule_run_issue::RunIssueRequest;
use crate::routes::canvas::schedule_projection::{
    detail, run_detail, summarize, RepoSchedulesResponse, ScheduleDetail, ScheduleFacts,
    ScheduleRunDetail, ScheduleSummary,
};
use crate::routes::canvas::sessions::{resolve_visible_repo, validate_repo_segment};
use crate::routes::dashboard::{bearer_token, DashboardGithub, IssueWithMeta};
use crate::schedule::{collect_records, RunRecord};
use crate::state::AppState;

/// One definition plus everything read for it, owned so the projection can borrow.
struct LoadedSchedule {
    issue: IssueWithMeta,
    spec: Result<ScheduledWorkflowSpec, String>,
    records: Vec<RunRecord>,
}

impl LoadedSchedule {
    fn facts(&self) -> ScheduleFacts<'_> {
        ScheduleFacts {
            schedule_issue: self.issue.summary.number,
            title: &self.issue.summary.title,
            html_url: &self.issue.html_url,
            labels: &self.issue.summary.labels,
            created_at: self
                .issue
                .created_at
                .parse()
                .unwrap_or(DateTime::UNIX_EPOCH),
            spec: self.spec.as_ref().map_err(String::clone),
            records: &self.records,
        }
    }
}

#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/schedules",
    tag = "canvas",
    operation_id = "canvas_repo_schedules",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
    ),
    responses(
        (status = 200, description = "The repo's scheduled workflows (empty with installed=false when the caller's installations do not cover it)", body = RepoSchedulesResponse),
        (status = 400, description = "Malformed owner/name", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn repo_schedules(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name)): AuditedPath<(String, String)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<RepoSchedulesResponse>, AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasRepoSchedules::new(&owner, &name),
    );
    super::record_repo_correlation(&extensions, &owner, &name);
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    let Some((_, repo_ref)) =
        resolve_visible_repo(&state, &gh, &user, &token, &owner, &name).await?
    else {
        return Ok(Json(RepoSchedulesResponse {
            owner,
            name,
            installed: false,
            schedules: Vec::new(),
        }));
    };
    let (owner, name) = (repo_ref.owner.clone(), repo_ref.name.clone());
    let loaded = load_schedules(&state, &gh, &owner, &name).await?;
    let now = Utc::now();
    let schedules: Vec<ScheduleSummary> = loaded
        .iter()
        .map(|schedule| summarize(&schedule.facts(), now))
        .collect();
    Ok(Json(RepoSchedulesResponse {
        owner,
        name,
        installed: true,
        schedules,
    }))
}

#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/schedules/{schedule_issue}",
    tag = "canvas",
    operation_id = "canvas_schedule_detail",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("schedule_issue" = u64, Path, description = "The definition issue's number"),
    ),
    responses(
        (status = 200, description = "The schedule, its next firings, and its run history", body = ScheduleDetail),
        (status = 400, description = "Malformed owner/name/issue number", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted", body = ErrorEnvelope),
        (status = 404, description = "No such scheduled workflow on this repository", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn schedule_detail(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name, schedule_issue)): AuditedPath<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<ScheduleDetail>, AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasScheduleDetail::new(
            &owner,
            &name,
            schedule_issue as i64,
        ),
    );
    let loaded = load_one(
        &state,
        &extensions,
        &owner,
        &name,
        schedule_issue,
        &user,
        &headers,
    )
    .await?;
    Ok(Json(detail(&loaded.facts(), Utc::now())))
}

#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/schedules/{schedule_issue}/runs/{slot}",
    tag = "canvas",
    operation_id = "canvas_schedule_run",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("schedule_issue" = u64, Path, description = "The definition issue's number"),
        ("slot" = String, Path, description = "The run's slot, RFC 3339 (e.g. 2026-08-05T01:00:00Z)"),
    ),
    responses(
        (status = 200, description = "The run and its per-step outcomes", body = ScheduleRunDetail),
        (status = 400, description = "Malformed owner/name/issue number, or an unparseable slot", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted", body = ErrorEnvelope),
        (status = 404, description = "No such scheduled workflow, or no run for that slot", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn schedule_run(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name, schedule_issue, slot)): AuditedPath<(String, String, u64, String)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<ScheduleRunDetail>, AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasScheduleRun::new(
            &owner,
            &name,
            schedule_issue as i64,
            &slot,
        ),
    );
    let parsed: DateTime<Utc> = DateTime::parse_from_rfc3339(&slot)
        .map_err(|_| AppError::Validation(format!("slot {slot:?} is not an RFC 3339 timestamp")))?
        .with_timezone(&Utc);
    let loaded = load_one(
        &state,
        &extensions,
        &owner,
        &name,
        schedule_issue,
        &user,
        &headers,
    )
    .await?;
    run_detail(&loaded.records, parsed)
        .map(Json)
        .ok_or_else(|| AppError::NotFound(format!("no run recorded for slot {slot}")))
}

#[utoipa::path(
    post,
    path = "/repos/{owner}/{name}/schedules/{schedule_issue}/pause",
    tag = "canvas",
    operation_id = "canvas_pause_schedule",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("schedule_issue" = u64, Path, description = "The definition issue's number"),
    ),
    responses(
        (status = 204, description = "The schedule is paused (idempotent)"),
        (status = 400, description = "Malformed owner/name/issue number", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not the definition's author nor a repo admin / org owner", body = ErrorEnvelope),
        (status = 404, description = "Not a scheduled-workflow issue", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn pause_schedule(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name, schedule_issue)): AuditedPath<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasPauseSchedule::new(
            &owner,
            &name,
            schedule_issue as i64,
        ),
    );
    let (gh, token, owner, name) = authorize_write(
        &state,
        &extensions,
        owner,
        name,
        schedule_issue,
        &user,
        &headers,
    )
    .await?;
    // Additive, so pausing an already-paused schedule is a no-op rather than an
    // error: the UI's toggle must be safe to press twice.
    gh.add_issue_label(&token, &owner, &name, schedule_issue, CRON_PAUSED_LABEL)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/repos/{owner}/{name}/schedules/{schedule_issue}/resume",
    tag = "canvas",
    operation_id = "canvas_resume_schedule",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("schedule_issue" = u64, Path, description = "The definition issue's number"),
    ),
    responses(
        (status = 204, description = "The schedule is running again (idempotent)"),
        (status = 400, description = "Malformed owner/name/issue number", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not the definition's author nor a repo admin / org owner", body = ErrorEnvelope),
        (status = 404, description = "Not a scheduled-workflow issue", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn resume_schedule(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name, schedule_issue)): AuditedPath<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<StatusCode, AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasResumeSchedule::new(
            &owner,
            &name,
            schedule_issue as i64,
        ),
    );
    let (gh, token, owner, name) = authorize_write(
        &state,
        &extensions,
        owner,
        name,
        schedule_issue,
        &user,
        &headers,
    )
    .await?;
    gh.remove_issue_label(&token, &owner, &name, schedule_issue, CRON_PAUSED_LABEL)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

#[utoipa::path(
    post,
    path = "/repos/{owner}/{name}/schedules/{schedule_issue}/run",
    tag = "canvas",
    operation_id = "canvas_run_schedule_now",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
        ("schedule_issue" = u64, Path, description = "The definition issue's number"),
    ),
    responses(
        (status = 202, description = "A manual run was dispatched; the response is the created run issue's number", body = u64),
        (status = 400, description = "Malformed owner/name/issue number", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not the definition's author nor a repo admin / org owner", body = ErrorEnvelope),
        (status = 404, description = "Not a scheduled-workflow issue", body = ErrorEnvelope),
        (status = 409, description = "A run is already in flight, or the definition is invalid", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn run_schedule_now(
    State(state): State<AppState>,
    extensions: axum::http::Extensions,
    AuditedPath((owner, name, schedule_issue)): AuditedPath<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
) -> Result<(StatusCode, Json<u64>), AppError> {
    crate::audit::arguments::record_safe(
        &extensions,
        &crate::audit::arguments::canvas::SafeCanvasRunScheduleNow::new(
            &owner,
            &name,
            schedule_issue as i64,
        ),
    );
    let (gh, _, owner, name) = authorize_write(
        &state,
        &extensions,
        owner.clone(),
        name.clone(),
        schedule_issue,
        &user,
        &headers,
    )
    .await?;

    let loaded = load_definition(&state, &gh, &owner, &name, schedule_issue).await?;
    let spec = loaded
        .spec
        .as_ref()
        .map_err(|detail| AppError::Conflict(format!("this schedule is not runnable: {detail}")))?;
    if loaded
        .issue
        .summary
        .labels
        .iter()
        .any(|label| label == CRON_RUNNING_LABEL)
    {
        return Err(AppError::Conflict(
            "a run is already in flight for this schedule".to_string(),
        ));
    }

    // The manual run goes through the SAME dispatch the clock uses, so it is
    // indistinguishable downstream — same run-issue shape, same record, same
    // completion detection. The slot is `now`, which cannot collide with a cron
    // slot to the second and is what marks the run manual in the history.
    let work_label = resolve_run_label(&state, &owner, &name, &loaded).await?;
    let request = RunIssueRequest {
        schedule_issue: schedule_issue as i64,
        workflow_id: spec.workflow_id.clone(),
        slot: Utc::now(),
        arguments: spec.arguments.clone(),
        work_label,
        creator_login: creator_login(&loaded),
        manual: true,
    };
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let run_issue = crate::reconcile::schedule_execute::dispatch_manual_run(
        &format!("{owner}/{name}"),
        request,
        app,
    )
    .await?;
    Ok((StatusCode::ACCEPTED, Json(run_issue)))
}

/// The definition's sole assignee: the session creator its run issues route to.
fn creator_login(loaded: &LoadedSchedule) -> String {
    loaded
        .issue
        .summary
        .assignees
        .first()
        .cloned()
        .unwrap_or_default()
}

/// The effective work label a manual run's issue must carry.
///
/// Resolved from the definition's assignee the same way the clock resolves it, so
/// a manual run cannot route somewhere a scheduled one would not.
async fn resolve_run_label(
    state: &AppState,
    owner: &str,
    name: &str,
    loaded: &LoadedSchedule,
) -> Result<String, AppError> {
    let creator = creator_login(loaded);
    if creator.is_empty() {
        return Err(AppError::Conflict(
            "this schedule has no assignee, so a run has no session to route to".to_string(),
        ));
    }
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let token = app.token_for_repo(&owner_repo, None).await?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    // Package/manifest reads are plain authenticated contents fetches; a
    // request-scoped client keeps this out of `AppState`, which nothing else on
    // the canvas surface needs.
    let http = reqwest::Client::builder()
        .user_agent("fkst-hosted-api")
        .build()
        .map_err(|error| AppError::Unavailable(format!("http client build failed: {error}")))?;
    let triggers: Vec<_> = gh
        .issues_by_label(
            &token,
            owner,
            name,
            &state.config.reconcile.substrate_trigger_label,
            "open",
        )
        .await?
        .into_iter()
        .map(|trigger| trigger.summary)
        .collect();
    crate::reconcile::schedule_pass::resolve_manual_run_label(
        &http,
        &state.config.github_api_base_url,
        &token,
        &crate::models::RepoRef {
            owner: owner.to_string(),
            name: name.to_string(),
        },
        &triggers,
        &creator,
        &state.config.reconcile,
    )
    .await
    .map_err(AppError::Conflict)
}

/// Load every definition on a repository, with its run history.
async fn load_schedules(
    state: &AppState,
    gh: &DashboardGithub,
    owner: &str,
    name: &str,
) -> Result<Vec<LoadedSchedule>, AppError> {
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let token = app.token_for_repo(&owner_repo, None).await?;
    let issues = gh
        .issues_by_label(&token, owner, name, SCHEDULED_WORKFLOW_LABEL, "open")
        .await?;

    let mut out = Vec::with_capacity(issues.len());
    for issue in issues {
        out.push(load_records(state, app, &token, owner, name, issue).await?);
    }
    Ok(out)
}

/// Load ONE definition by number, refusing anything that is not one.
async fn load_definition(
    state: &AppState,
    gh: &DashboardGithub,
    owner: &str,
    name: &str,
    schedule_issue: u64,
) -> Result<LoadedSchedule, AppError> {
    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let token = app.token_for_repo(&owner_repo, None).await?;
    // Listed rather than fetched by number so the label check and the projection
    // read the same shape, and so a number that is not a definition 404s here
    // instead of half-projecting.
    let issues = gh
        .issues_by_label(&token, owner, name, SCHEDULED_WORKFLOW_LABEL, "all")
        .await?;
    let issue = issues
        .into_iter()
        .find(|issue| issue.summary.number == schedule_issue as i64)
        .ok_or_else(|| {
            AppError::NotFound(format!(
                "#{schedule_issue} is not a scheduled workflow on {owner}/{name}"
            ))
        })?;
    load_records(state, app, &token, owner, name, issue).await
}

/// Attach a definition's parsed spec and its trusted run records.
async fn load_records(
    state: &AppState,
    app: &crate::github_app::GithubAppTokens,
    token: &SecretString,
    owner: &str,
    name: &str,
    issue: IssueWithMeta,
) -> Result<LoadedSchedule, AppError> {
    let _ = app;
    let spec = parse_scheduled_workflow(&issue.summary.body).map_err(|error| error.to_string());
    // Only App-authored comments are records — the SAME trust rule the clock
    // applies, so the dashboard can never show a forged run as real. With no App
    // identity configured the history reads empty rather than unverified.
    let records = match state.config.reconcile.github_bot_login.as_deref() {
        None => Vec::new(),
        Some(bot_login) => {
            let reader = HttpIssueCommentReader::new(&state.config.github_api_base_url)?;
            let comments = reader
                .list_recent_issue_comments(
                    token,
                    owner,
                    name,
                    issue.summary.number as u64,
                    state.config.reconcile.cron_history_pages,
                )
                .await?;
            let trusted: Vec<String> = comments
                .into_iter()
                .filter(|comment| comment_is_from_bot(&comment.user_login, bot_login))
                .map(|comment| comment.body)
                .collect();
            collect_records(&trusted)
        }
    };
    Ok(LoadedSchedule {
        issue,
        spec,
        records,
    })
}

/// Read-tier load of one definition: repository visibility, then the definition.
async fn load_one(
    state: &AppState,
    extensions: &axum::http::Extensions,
    owner: &str,
    name: &str,
    schedule_issue: u64,
    user: &GithubUser,
    headers: &HeaderMap,
) -> Result<LoadedSchedule, AppError> {
    super::record_repo_correlation(extensions, owner, name);
    validate_repo_segment(owner, "owner")?;
    validate_repo_segment(name, "name")?;
    let token = bearer_token(headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;
    let Some((_, repo_ref)) = resolve_visible_repo(state, &gh, user, &token, owner, name).await?
    else {
        return Err(AppError::NotFound(format!(
            "{owner}/{name} is not visible to this caller"
        )));
    };
    load_definition(state, &gh, &repo_ref.owner, &repo_ref.name, schedule_issue).await
}

/// Write-tier gate: the definition's author OR a repository admin / org owner.
///
/// Mirrors `stop_session` exactly. Session Collaborators are deliberately NOT
/// admitted: they hold work-item authority, and pausing or firing someone else's
/// schedule is a management action.
#[allow(clippy::type_complexity)]
async fn authorize_write(
    state: &AppState,
    extensions: &axum::http::Extensions,
    owner: String,
    name: String,
    schedule_issue: u64,
    user: &GithubUser,
    headers: &HeaderMap,
) -> Result<(DashboardGithub, SecretString, String, String), AppError> {
    super::record_repo_correlation(extensions, &owner, &name);
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    if schedule_issue == 0 {
        return Err(AppError::Validation(
            "schedule_issue must be a positive issue number".to_string(),
        ));
    }
    let token = bearer_token(headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // Refuse anything that is not a definition BEFORE writing: GitHub's label
    // endpoints would happily label an unrelated issue the caller can write.
    let issue = gh.get_issue(&token, &owner, &name, schedule_issue).await?;
    if issue.is_pull_request || !issue.labels.iter().any(|l| l == SCHEDULED_WORKFLOW_LABEL) {
        return Err(AppError::NotFound(format!(
            "#{schedule_issue} is not a scheduled workflow (missing the \
             {SCHEDULED_WORKFLOW_LABEL} label)"
        )));
    }
    let authorized =
        user.id == issue.author_id || gh.caller_is_repo_admin(&token, &owner, &name).await?;
    if !authorized {
        return Err(AppError::Forbidden(format!(
            "only the schedule's author or a repo admin / org owner may operate #{schedule_issue}"
        )));
    }
    Ok((gh, token, owner, name))
}

#[cfg(test)]
#[path = "schedules_tests.rs"]
mod tests;
