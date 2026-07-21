//! One repo's sessions, scanned live: `GET /api/v1/repos/{owner}/{name}/sessions`.
//!
//! The synchronous level-2 counterpart of the cached dashboard's
//! `scan_repo_sessions`: trigger issues state=all (closed triggers ARE the
//! session history), each parsed with the reconciler's `parse_registration` so
//! a session groups exactly as the control plane sees it, plus what the detail
//! panel needs on top — issue links/timestamps, the App bot's devloop PRs
//! linked back to their work issues (via the auto-merge module's shared
//! head_ref/title parse), the session's log-download URL, and the live runtime
//! liveness from the session backend.
//!
//! Access scoping: the CALLER's user token must see an App installation
//! covering the repo (`/user/installations` + its repo listing). A repo outside
//! the caller's installations renders `installed: false` with no sessions —
//! never another user's private session data.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::listing::IssueSummary;
use crate::github_identity::GithubUser;
use crate::reconcile::automerge::linked_issue_number;
use crate::reconcile::desired::PodLiveness;
use crate::reconcile::registry::parse_registration;
use crate::routes::canvas::github::RepoPull;
use crate::routes::canvas::types::{render_package_ref, IssueDetail};
use crate::routes::dashboard::{bearer_token, status_labels, DashboardGithub, IssueWithMeta};
use crate::state::AppState;

/// One repo's sessions, scanned live.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RepoSessionsResponse {
    /// GitHub's canonical owner casing when installed (a case-variant request
    /// path is canonicalized); the caller's raw segment when not installed.
    pub owner: String,
    /// GitHub's canonical repo-name casing when installed; the caller's raw
    /// segment when not installed.
    pub name: String,
    /// The App covers this repo for the CALLER (an installation the caller can
    /// see lists it). False renders an empty canvas card, never an error.
    pub installed: bool,
    /// One entry per trigger issue (open AND closed), newest-first as GitHub
    /// lists them.
    pub sessions: Vec<SessionDetail>,
}

/// One session = one trigger issue, with everything the level-2 panel renders.
/// A trigger whose body fails to parse carries `invalid_reason` and nulls.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SessionDetail {
    /// The deterministic session id; null when the trigger body is invalid.
    pub session_id: Option<String>,
    /// The `### Session Name`; null when invalid.
    pub name: Option<String>,
    /// The `### Work Label`; null when invalid or auto-discovered.
    pub work_label: Option<String>,
    /// The `### Auto-merge` opt-in; null when invalid.
    pub auto_merge: Option<bool>,
    /// The `### Environment` selection, if any.
    pub environment: Option<String>,
    /// The `### Packages`, rendered as `owner/repo@ref:path`.
    pub packages: Vec<String>,
    /// The `### Manifest` fkst-manifest references, rendered as
    /// `owner/repo@ref:path` (each names a JSON bundle the server expands into a
    /// package list); empty when invalid or none were specified.
    pub manifests: Vec<String>,
    /// The `### Log Access Allowlist` grantees (extra log-download logins/ids,
    /// beyond the trigger author + global admins); empty when invalid or none
    /// were specified. Frozen by config-immutability after registration.
    pub log_access: Vec<String>,
    /// The `### Session Collaborators` — GitHub logins granted WORK-ITEM
    /// AUTHORITY over the session (beyond the trigger author); empty when
    /// invalid or none were specified. A DISTINCT list from
    /// [`log_access`](Self::log_access) (log-download access). Frozen by
    /// config-immutability after registration.
    pub collaborators: Vec<String>,
    /// The `### Output Language` locale (rendered into the session as
    /// `FKST_OUTPUT_LANG`); null when invalid or unspecified.
    pub output_lang: Option<String>,
    /// The parse error when the trigger body is malformed; else null.
    pub invalid_reason: Option<String>,
    /// The `fkst-*` control-plane status labels on the trigger issue, minus
    /// the configured trigger label itself (present on every session by
    /// definition, so it is noise as a status marker).
    pub status_labels: Vec<String>,
    /// The trigger issue itself.
    pub trigger: IssueDetail,
    /// The session's work-label issues (open AND closed).
    pub work_issues: Vec<IssueDetail>,
    /// The identity-gated log download URL
    /// (`FKST_PUBLIC_BASE_URL/api/v1/logs/{session_id}`); null when the base
    /// URL is unconfigured or the session is invalid.
    pub log_url: Option<String>,
    /// The live runtime phase (`starting`/`live`/`terminating`); null when the
    /// session backend is unavailable, errored, or holds no pod (absent and
    /// finished-awaiting-GC runtimes both render null).
    pub liveness: Option<String>,
    /// The App bot's devloop PRs whose work issue belongs to this session.
    pub prs: Vec<PrDetail>,
}

/// One devloop pull request of the App bot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct PrDetail {
    pub number: i64,
    pub title: String,
    pub html_url: String,
    /// `open` or `closed`.
    pub state: String,
    pub merged: bool,
    /// The work-issue number parsed from the devloop head branch / PR title;
    /// null when the number does not fit the issue-number domain.
    pub work_issue: Option<i64>,
}

/// Validate one `owner`/`name` path segment against the same charset the App
/// token service accepts (`^[A-Za-z0-9_.-]+$`) — anything else can never be a
/// GitHub repo and would only end up interpolated into GitHub URLs.
pub(super) fn validate_repo_segment(value: &str, which: &str) -> Result<(), AppError> {
    let valid = !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if valid {
        Ok(())
    } else {
        Err(AppError::Validation(format!(
            "invalid {which}: must match ^[A-Za-z0-9_.-]+$"
        )))
    }
}

/// Project a runtime liveness into the canvas' three visible phases. `Absent`
/// and `Terminal` (finished, awaiting GC) both render null — the canvas only
/// blinks for something actually running or on its way in/out.
fn liveness_label(liveness: PodLiveness) -> Option<&'static str> {
    match liveness {
        PodLiveness::Starting => Some("starting"),
        PodLiveness::Live => Some("live"),
        PodLiveness::Terminating => Some("terminating"),
        PodLiveness::Absent | PodLiveness::Terminal => None,
    }
}

/// Keep only the App bot's devloop PRs: authored by `bot_login` AND carrying a
/// parseable work-issue number in the devloop head branch / title (the SAME
/// parse auto-merge uses). Template-reconcile and other non-devloop bot PRs
/// carry no such number and drop out.
///
/// `pub(super)` so the outcomes endpoint reuses the exact same devloop-PR
/// projection (one grouping rule for both surfaces).
pub(super) fn devloop_prs(pulls: &[RepoPull], bot_login: Option<&str>) -> Vec<PrDetail> {
    let Some(bot) = bot_login else {
        tracing::debug!("canvas sessions: FKST_GITHUB_BOT_LOGIN unset; listing no devloop PRs");
        return Vec::new();
    };
    pulls
        .iter()
        .filter(|pull| pull.author == bot)
        .filter_map(|pull| {
            let issue = linked_issue_number(&pull.head_ref, &pull.title)?;
            Some(PrDetail {
                number: pull.number,
                title: pull.title.clone(),
                html_url: pull.html_url.clone(),
                state: pull.state.clone(),
                merged: pull.merged,
                work_issue: i64::try_from(issue).ok(),
            })
        })
        .collect()
}

/// The canvas projection of a trigger's `fkst-*` labels: the dashboard's
/// [`status_labels`] MINUS the configured trigger label itself — that label is
/// what makes the issue a session at all, not a status marker worth a chip.
fn canvas_status_labels(issue: &IssueSummary, trigger_label: &str) -> Vec<String> {
    status_labels(issue)
        .into_iter()
        .filter(|label| label != trigger_label)
        .collect()
}

/// A session detail for a trigger issue whose body failed to parse.
fn invalid_session_detail(
    trigger: &IssueWithMeta,
    reason: String,
    trigger_label: &str,
) -> SessionDetail {
    SessionDetail {
        session_id: None,
        name: None,
        work_label: None,
        auto_merge: None,
        environment: None,
        packages: Vec::new(),
        manifests: Vec::new(),
        log_access: Vec::new(),
        collaborators: Vec::new(),
        output_lang: None,
        invalid_reason: Some(reason),
        status_labels: canvas_status_labels(&trigger.summary, trigger_label),
        trigger: IssueDetail::from(trigger),
        work_issues: Vec::new(),
        log_url: None,
        liveness: None,
        prs: Vec::new(),
    }
}

/// `GET /api/v1/repos/{owner}/{name}/sessions` — one repo's sessions, live.
#[utoipa::path(
    get,
    path = "/repos/{owner}/{name}/sessions",
    tag = "canvas",
    operation_id = "canvas_repo_sessions",
    params(
        ("owner" = String, Path, description = "Repo owner (user or org) login"),
        ("name" = String, Path, description = "Repo name"),
    ),
    responses(
        (status = 200, description = "The repo's sessions (empty with installed=false when the caller's installations do not cover it)", body = RepoSessionsResponse),
        (status = 400, description = "Malformed owner/name", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Verified GitHub identity not allowlisted (FKST_ACCESS_ALLOWED_USERS)", body = ErrorEnvelope),
        (status = 502, description = "GitHub API error", body = ErrorEnvelope),
        (status = 503, description = "The GitHub App is not configured, or GitHub is unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn repo_sessions(
    State(state): State<AppState>,
    Path((owner, name)): Path<(String, String)>,
    _user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<RepoSessionsResponse>, AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // Caller-scoped installation check: the user token only ever sees THIS
    // App's installations, so membership here is both the `installed` flag and
    // the access gate on another user's session data. The path segments match
    // case-insensitively (GitHub treats owner/name that way), but the MATCHED
    // installation repo carries GitHub's canonical casing — session ids, log
    // URLs, and pod annotations are all derived case-sensitively from that
    // casing, so the caller's variant must never leak into them.
    let installations = gh.user_installations(&token).await?;
    let installation = installations
        .iter()
        .find(|inst| inst.account.eq_ignore_ascii_case(&owner));
    let canonical = match installation {
        Some(inst) => gh
            .user_installation_repos(&token, inst.id)
            .await?
            .into_iter()
            .find(|repo| {
                repo.owner.eq_ignore_ascii_case(&owner) && repo.name.eq_ignore_ascii_case(&name)
            }),
        None => None,
    };
    let (Some(installation), Some(repo_ref)) = (installation, canonical) else {
        tracing::debug!(owner = %owner, name = %name, "canvas sessions: repo not in the caller's installations");
        return Ok(Json(RepoSessionsResponse {
            owner,
            name,
            installed: false,
            sessions: Vec::new(),
        }));
    };
    // From here on GitHub's canonical casing is authoritative.
    let owner = repo_ref.owner.clone();
    let name = repo_ref.name.clone();

    let app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unavailable("the github app is not configured on this deployment".to_string())
    })?;
    let owner_repo = format!("{owner}/{name}");
    let inst_token = app.token_for_repo(&owner_repo, None).await?;

    let trigger_label = &state.config.reconcile.substrate_trigger_label;
    let triggers = gh
        .issues_by_label_all(&inst_token, &owner, &name, trigger_label)
        .await?;

    // Live runtime phases, keyed by session id. Best-effort: a missing backend
    // or an observe failure renders liveness null, never fails the scan.
    let liveness_by_session: HashMap<String, PodLiveness> = match state.session_backend.as_ref() {
        Some(backend) => match backend.observe_repo(&repo_ref).await {
            Ok(pods) => pods
                .into_iter()
                .map(|pod| (pod.session_id, pod.liveness))
                .collect(),
            Err(error) => {
                tracing::warn!(owner = %owner, name = %name, error = %error, "canvas sessions: observe_repo failed; rendering liveness null");
                HashMap::new()
            }
        },
        None => HashMap::new(),
    };

    let pulls = gh.list_pulls_all(&inst_token, &owner, &name).await?;
    let all_devloop_prs = devloop_prs(&pulls, state.config.reconcile.github_bot_login.as_deref());

    let mut sessions = Vec::with_capacity(triggers.len());
    for trigger in &triggers {
        match parse_registration(installation.id, &repo_ref, &trigger.summary) {
            Ok(reg) => {
                let work: Vec<IssueWithMeta> = match reg.def.work_label.as_deref() {
                    Some(work_label) => {
                        gh.issues_by_label_all(&inst_token, &owner, &name, work_label)
                            .await?
                    }
                    None => Vec::new(),
                };
                let work_numbers: HashSet<i64> =
                    work.iter().map(|issue| issue.summary.number).collect();
                let prs: Vec<PrDetail> = all_devloop_prs
                    .iter()
                    .filter(|pr| pr.work_issue.is_some_and(|n| work_numbers.contains(&n)))
                    .cloned()
                    .collect();
                let log_url = state.config.log.public_base_url.as_deref().map(|base| {
                    format!(
                        "{}/api/v1/logs/{}",
                        base.trim_end_matches('/'),
                        reg.session_id
                    )
                });
                let liveness = liveness_by_session
                    .get(&reg.session_id)
                    .and_then(|phase| liveness_label(*phase))
                    .map(str::to_string);
                sessions.push(SessionDetail {
                    session_id: Some(reg.session_id.clone()),
                    name: Some(reg.def.name.clone()),
                    work_label: reg.def.work_label.clone(),
                    auto_merge: Some(reg.auto_merge),
                    environment: reg.def.environment.clone(),
                    packages: reg.def.packages.iter().map(render_package_ref).collect(),
                    manifests: reg
                        .def
                        .manifest_refs
                        .iter()
                        .map(render_package_ref)
                        .collect(),
                    log_access: reg.log_access.clone(),
                    collaborators: reg.collaborators.clone(),
                    output_lang: reg.def.output_lang.clone(),
                    invalid_reason: None,
                    status_labels: canvas_status_labels(&trigger.summary, trigger_label),
                    trigger: IssueDetail::from(trigger),
                    work_issues: work.iter().map(IssueDetail::from).collect(),
                    log_url,
                    liveness,
                    prs,
                });
            }
            Err((_, reason)) => {
                sessions.push(invalid_session_detail(trigger, reason, trigger_label))
            }
        }
    }

    tracing::debug!(owner = %owner, name = %name, sessions = sessions.len(), "canvas repo sessions assembled");
    Ok(Json(RepoSessionsResponse {
        owner,
        name,
        installed: true,
        sessions,
    }))
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
