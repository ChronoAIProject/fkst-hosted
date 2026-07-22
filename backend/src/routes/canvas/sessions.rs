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
//! Access scoping: an ordinary CALLER's user token must see an App installation
//! covering the repo (`/user/installations` + its repo listing). A repo outside
//! the caller's installations renders `installed: false` with no sessions —
//! never another user's private session data. A verified `FKST_GLOBAL_ADMINS`
//! caller instead resolves through the App's complete installation inventory,
//! which is the explicit operator-only exception to that anti-enumeration rule.

use std::collections::{HashMap, HashSet};

use axum::extract::{Path, State};
use axum::http::HeaderMap;
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::listing::IssueSummary;
use crate::github_identity::GithubUser;
use crate::models::RepoRef;
use crate::reconcile::automerge::linked_issue_number;
use crate::reconcile::branches::DEFAULT_TARGET_BRANCH;
use crate::reconcile::desired::PodLiveness;
use crate::reconcile::{
    effective_creator, CreatorResolution, SUBSTRATE_CONFIG_REJECTED_LABEL,
    SUBSTRATE_DEGRADED_LABEL, SUBSTRATE_INVALID_LABEL, SUBSTRATE_RETIRED_LABEL,
    WORK_UNAUTHORIZED_LABEL,
};
use crate::routes::canvas::github::RepoPull;
use crate::routes::canvas::parse_trigger_registration;
use crate::routes::canvas::types::{render_package_ref, IssueDetail};
use crate::routes::canvas::work_projection::work_issues_by_session;
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
    /// The effective human creator. For App-authored seeded triggers this is
    /// the sole assignee; empty only when an invalid trigger is unattributable.
    pub creator: String,
    /// The `### Work Label`; null when invalid or auto-discovered.
    pub work_label: Option<String>,
    /// Every label that can wake this session: the explicit `### Work Label`
    /// plus labels discovered from the manifest-expanded package set. Sorted
    /// and deduplicated; empty when the trigger or package sources cannot be
    /// resolved.
    pub work_labels: Vec<String>,
    /// The `### Auto-merge` opt-in; null when invalid.
    pub auto_merge: Option<bool>,
    /// The `### Environment` selection, if any.
    pub environment: Option<String>,
    /// The authored source branch; null means the repository default branch.
    pub source_branch: Option<String>,
    /// The resolved target branch, including `fkst-hosted-default` when the
    /// trigger omits `### Target Branch`.
    pub target_branch: String,
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
    /// The live runtime phase (`starting`/`live`/`terminating`); null when no
    /// runtime is actively transitioning/running. [`recovery`](Self::recovery)
    /// distinguishes absent, terminal, and observation-unavailable states.
    pub liveness: Option<String>,
    /// Backend-derived recovery state. This is a bounded read model over the
    /// trigger/work ledger and runtime observation; it never contains raw
    /// backend/GitHub errors.
    pub recovery: SessionRecoveryProjection,
    /// The App bot's devloop PRs whose work issue belongs to this session.
    pub prs: Vec<PrDetail>,
}

/// Coarse session recovery condition for operator and user surfaces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionRecoveryState {
    Normal,
    Idle,
    Recovering,
    Degraded,
    Unknown,
    Retired,
    Invalid,
}

/// Stable explanation for a [`SessionRecoveryState`]. No transport or provider
/// error strings cross this boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionRecoveryReason {
    RuntimeLive,
    NoPendingWork,
    RuntimeStarting,
    RuntimeTerminating,
    RuntimeAbsent,
    RuntimeTerminal,
    RuntimeObservationUnavailable,
    RuntimeHealthDegraded,
    TriggerClosed,
    RegistrationInvalid,
    ConfigurationRejected,
}

/// Full runtime observation, including the states the legacy `liveness` field
/// deliberately collapses to null.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SessionRuntimeState {
    Absent,
    Starting,
    Live,
    Terminating,
    Terminal,
    Unknown,
}

/// Typed, bounded recovery diagnostics for one session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SessionRecoveryProjection {
    pub state: SessionRecoveryState,
    pub reason: SessionRecoveryReason,
    /// Open exact-work-label issues after durable unauthorized/retired latches
    /// are excluded.
    pub open_work_items: usize,
    pub runtime: SessionRuntimeState,
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

/// Resolve one requested repository through the appropriate visibility boundary.
/// Ordinary callers use only their user-token installation list; verified global
/// admins use the App-wide installation inventory. The returned repository always
/// carries GitHub's canonical owner/name casing.
pub(super) async fn resolve_visible_repo(
    state: &AppState,
    gh: &DashboardGithub,
    user: &GithubUser,
    user_token: &secrecy::SecretString,
    owner: &str,
    name: &str,
) -> Result<Option<(i64, RepoRef)>, AppError> {
    if state.config.access.is_global_admin(user.id, &user.login) {
        let app = state.github_app.as_ref().ok_or_else(|| {
            AppError::Unavailable("the github app is not configured on this deployment".to_string())
        })?;
        let app_jwt = app.app_jwt()?;
        let installations = gh.app_installations(&app_jwt).await?;
        let Some(installation) = installations
            .into_iter()
            .find(|candidate| candidate.account.eq_ignore_ascii_case(owner))
        else {
            return Ok(None);
        };
        let installation_token = app.installation_wide_token(installation.id).await?;
        let repo = gh
            .installation_repos(&installation_token)
            .await?
            .into_iter()
            .find(|repo| {
                repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
            })
            .map(|repo| RepoRef {
                owner: repo.owner,
                name: repo.name,
            });
        return Ok(repo.map(|repo| (installation.id, repo)));
    }

    let installations = gh.user_installations(user_token).await?;
    let Some(installation) = installations
        .iter()
        .find(|candidate| candidate.account.eq_ignore_ascii_case(owner))
    else {
        return Ok(None);
    };
    let repo = gh
        .user_installation_repos(user_token, installation.id)
        .await?
        .into_iter()
        .find(|repo| {
            repo.owner.eq_ignore_ascii_case(owner) && repo.name.eq_ignore_ascii_case(name)
        });
    Ok(repo.map(|repo| (installation.id, repo)))
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

fn runtime_state(liveness: Option<PodLiveness>) -> SessionRuntimeState {
    match liveness {
        Some(PodLiveness::Absent) => SessionRuntimeState::Absent,
        Some(PodLiveness::Starting) => SessionRuntimeState::Starting,
        Some(PodLiveness::Live) => SessionRuntimeState::Live,
        Some(PodLiveness::Terminating) => SessionRuntimeState::Terminating,
        Some(PodLiveness::Terminal) => SessionRuntimeState::Terminal,
        None => SessionRuntimeState::Unknown,
    }
}

/// Preserve the difference between an authoritative observation that did not
/// contain this session (`Absent`) and a repository observation that never
/// completed (`None` / `Unknown`).
fn observed_session_liveness(
    runtime_observed: bool,
    liveness_by_session: &HashMap<String, PodLiveness>,
    session_id: &str,
) -> Option<PodLiveness> {
    runtime_observed.then(|| {
        liveness_by_session
            .get(session_id)
            .copied()
            .unwrap_or(PodLiveness::Absent)
    })
}

fn open_actionable_work_items(work: &[IssueWithMeta]) -> usize {
    work.iter()
        .filter(|issue| issue.summary.state == "open")
        .filter(|issue| {
            !issue
                .summary
                .labels
                .iter()
                .any(|label| label == WORK_UNAUTHORIZED_LABEL || label == SUBSTRATE_RETIRED_LABEL)
        })
        .count()
}

fn project_session_recovery(
    trigger_state: &str,
    labels: &[String],
    open_work_items: usize,
    liveness: Option<PodLiveness>,
) -> SessionRecoveryProjection {
    let runtime = runtime_state(liveness);
    let has = |expected: &str| labels.iter().any(|label| label == expected);
    let (state, reason) = if has(SUBSTRATE_CONFIG_REJECTED_LABEL) {
        (
            SessionRecoveryState::Invalid,
            SessionRecoveryReason::ConfigurationRejected,
        )
    } else if has(SUBSTRATE_INVALID_LABEL) {
        (
            SessionRecoveryState::Invalid,
            SessionRecoveryReason::RegistrationInvalid,
        )
    } else if trigger_state == "closed" || has(SUBSTRATE_RETIRED_LABEL) {
        (
            SessionRecoveryState::Retired,
            SessionRecoveryReason::TriggerClosed,
        )
    } else if has(SUBSTRATE_DEGRADED_LABEL) {
        (
            SessionRecoveryState::Degraded,
            SessionRecoveryReason::RuntimeHealthDegraded,
        )
    } else if open_work_items == 0 {
        (
            SessionRecoveryState::Idle,
            SessionRecoveryReason::NoPendingWork,
        )
    } else {
        match runtime {
            SessionRuntimeState::Live => (
                SessionRecoveryState::Normal,
                SessionRecoveryReason::RuntimeLive,
            ),
            SessionRuntimeState::Starting => (
                SessionRecoveryState::Recovering,
                SessionRecoveryReason::RuntimeStarting,
            ),
            SessionRuntimeState::Terminating => (
                SessionRecoveryState::Recovering,
                SessionRecoveryReason::RuntimeTerminating,
            ),
            SessionRuntimeState::Absent => (
                SessionRecoveryState::Recovering,
                SessionRecoveryReason::RuntimeAbsent,
            ),
            SessionRuntimeState::Terminal => (
                SessionRecoveryState::Recovering,
                SessionRecoveryReason::RuntimeTerminal,
            ),
            SessionRuntimeState::Unknown => (
                SessionRecoveryState::Unknown,
                SessionRecoveryReason::RuntimeObservationUnavailable,
            ),
        }
    };

    SessionRecoveryProjection {
        state,
        reason,
        open_work_items,
        runtime,
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
    bot_login: Option<&str>,
) -> SessionDetail {
    let status_labels = canvas_status_labels(&trigger.summary, trigger_label);
    let recovery_reason = if status_labels
        .iter()
        .any(|label| label == SUBSTRATE_CONFIG_REJECTED_LABEL)
    {
        SessionRecoveryReason::ConfigurationRejected
    } else {
        SessionRecoveryReason::RegistrationInvalid
    };

    let creator = match effective_creator(&trigger.summary.metadata(), bot_login) {
        CreatorResolution::Resolved(creator) => creator.login,
        CreatorResolution::Unattributable { .. } => String::new(),
    };

    SessionDetail {
        session_id: None,
        name: None,
        creator,
        work_label: None,
        work_labels: Vec::new(),
        auto_merge: None,
        environment: None,
        source_branch: None,
        target_branch: DEFAULT_TARGET_BRANCH.to_string(),
        packages: Vec::new(),
        manifests: Vec::new(),
        log_access: Vec::new(),
        collaborators: Vec::new(),
        output_lang: None,
        invalid_reason: Some(reason),
        status_labels,
        trigger: IssueDetail::from(trigger),
        work_issues: Vec::new(),
        log_url: None,
        liveness: None,
        recovery: SessionRecoveryProjection {
            state: SessionRecoveryState::Invalid,
            reason: recovery_reason,
            open_work_items: 0,
            runtime: SessionRuntimeState::Unknown,
        },
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
    user: GithubUser,
    headers: HeaderMap,
) -> Result<Json<RepoSessionsResponse>, AppError> {
    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    let token = bearer_token(&headers)?;
    let gh = DashboardGithub::new(&state.config.github_api_base_url)?;

    // The matched repo carries GitHub's canonical casing — session ids, log URLs,
    // and runtime annotations are derived case-sensitively, so the request's case
    // variant must never leak into them.
    let Some((installation_id, repo_ref)) =
        resolve_visible_repo(&state, &gh, &user, &token, &owner, &name).await?
    else {
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

    // Parse first, then resolve all sessions' manifest-expanded label sets in one
    // batch. This mirrors the reconciler and bounds shared manifest/package reads
    // within one dashboard request.
    let mut registrations = Vec::new();
    let mut parse_errors = HashMap::new();
    for trigger in &triggers {
        match parse_trigger_registration(
            installation_id,
            &repo_ref,
            &trigger.summary,
            state.config.reconcile.github_bot_login.as_deref(),
        ) {
            Ok(reg) => registrations.push(reg),
            Err((issue, reason)) => {
                parse_errors.insert(issue, reason);
            }
        }
    }
    let mut work_projection =
        work_issues_by_session(&gh, &inst_token, &owner, &name, &mut registrations).await?;
    let mut registrations_by_issue: HashMap<_, _> = registrations
        .into_iter()
        .map(|reg| (reg.trigger_issue, reg))
        .collect();

    // A successful repository observation makes a missing session authoritative
    // `Absent`; an unavailable backend/failed observation remains `Unknown`.
    let (runtime_observed, liveness_by_session): (bool, HashMap<String, PodLiveness>) = match state
        .session_backend
        .as_ref()
    {
        Some(backend) => match backend.observe_repo(&repo_ref).await {
            Ok(pods) => (
                true,
                pods.into_iter()
                    .map(|pod| (pod.session_id, pod.liveness))
                    .collect(),
            ),
            Err(error) => {
                tracing::warn!(owner = %owner, name = %name, error = %error, "canvas sessions: observe_repo failed; rendering runtime observation unknown");
                (false, HashMap::new())
            }
        },
        None => (false, HashMap::new()),
    };

    let pulls = gh.list_pulls_all(&inst_token, &owner, &name).await?;
    let all_devloop_prs = devloop_prs(&pulls, state.config.reconcile.github_bot_login.as_deref());

    let mut sessions = Vec::with_capacity(triggers.len());
    for trigger in &triggers {
        match registrations_by_issue.remove(&trigger.summary.number) {
            Some(reg) => {
                let work = work_projection
                    .issues_by_session
                    .remove(&reg.session_id)
                    .unwrap_or_default();
                let work_labels = work_projection
                    .labels_by_session
                    .remove(&reg.session_id)
                    .unwrap_or_default();
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
                let observed_liveness = observed_session_liveness(
                    runtime_observed,
                    &liveness_by_session,
                    &reg.session_id,
                );
                let liveness = observed_liveness
                    .and_then(liveness_label)
                    .map(str::to_string);
                let status_labels = canvas_status_labels(&trigger.summary, trigger_label);
                let open_work_items = open_actionable_work_items(&work);
                let recovery = project_session_recovery(
                    &trigger.summary.state,
                    &status_labels,
                    open_work_items,
                    observed_liveness,
                );
                sessions.push(SessionDetail {
                    session_id: Some(reg.session_id.clone()),
                    name: Some(reg.def.name.clone()),
                    creator: reg.creator_login.clone(),
                    work_label: reg.def.work_label.clone(),
                    work_labels,
                    auto_merge: Some(reg.auto_merge),
                    environment: reg.def.environment.clone(),
                    source_branch: reg.def.source_branch.clone(),
                    target_branch: reg
                        .def
                        .target_branch
                        .clone()
                        .unwrap_or_else(|| DEFAULT_TARGET_BRANCH.to_string()),
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
                    status_labels,
                    trigger: IssueDetail::from(trigger),
                    work_issues: work.iter().map(IssueDetail::from).collect(),
                    log_url,
                    liveness,
                    recovery,
                    prs,
                });
            }
            None => {
                let reason = parse_errors
                    .remove(&trigger.summary.number)
                    .unwrap_or_else(|| "trigger registration could not be resolved".to_string());
                sessions.push(invalid_session_detail(
                    trigger,
                    reason,
                    trigger_label,
                    state.config.reconcile.github_bot_login.as_deref(),
                ))
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
