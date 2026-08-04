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

use axum::extract::State;
use axum::http::{Extensions, HeaderMap, StatusCode};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

use crate::audit::arguments::canvas_write::CreateWorkItemInput;
use crate::audit::arguments::{record, refine, AuditedJson, AuditedPath};
use crate::error::{AppError, ErrorEnvelope};
use crate::github_app::listing::IssueMetadata;
use crate::github_identity::GithubUser;
use crate::goals::trigger_parse::{parse_trigger_issue_body, TriggerSpec};
use crate::models::RepoRef;
use crate::reconcile::desired::{config_hash, SessionDef, SessionRegistration};
use crate::reconcile::effective_packages::resolve_effective_packages;
use crate::reconcile::work_authz::is_work_author_allowed;
use crate::reconcile::work_labels::{apply_work_label_namespace, resolve_work_label_sets};
use crate::reconcile::{effective_creator, CreatorResolution, SessionCreator};
use crate::routes::canvas::sessions::validate_repo_segment;
use crate::routes::dashboard::{bearer_token, DashboardGithub};
use crate::state::AppState;

/// Request body for queuing a work item on a session.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct CreateWorkItemRequest {
    /// The work-issue title (also the GitHub issue title); required, non-blank.
    pub title: String,
    /// An optional label selected from the session's resolved applicable set.
    /// When omitted, the trigger's explicit `### Work Label` is used; sessions
    /// whose labels are discovered from packages or manifests must name one.
    #[serde(default, alias = "work_label")]
    pub label: Option<String>,
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
        (status = 400, description = "Malformed owner/name/issue number, a blank title/selected label, or GitHub rejected the issue", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub token", body = ErrorEnvelope),
        (status = 403, description = "Not allowlisted, the caller lacks work-item authority on this session (not the creator, a listed Session Collaborator, nor a deployment global administrator), or GitHub refused the write for this caller", body = ErrorEnvelope),
        (status = 404, description = "No such trigger issue (or the caller cannot see the repo)", body = ErrorEnvelope),
        (status = 409, description = "The session trigger is closed", body = ErrorEnvelope),
        (status = 422, description = "The trigger issue or its package sources are malformed, the session has no applicable work labels, or the selected label is not applicable", body = ErrorEnvelope),
        (status = 503, description = "GitHub API unreachable", body = ErrorEnvelope),
    )
)]
pub(super) async fn create_work_item(
    State(state): State<AppState>,
    extensions: Extensions,
    AuditedPath((owner, name, issue_number)): AuditedPath<(String, String, u64)>,
    user: GithubUser,
    headers: HeaderMap,
    AuditedJson(req): AuditedJson<CreateWorkItemRequest>,
) -> Result<(StatusCode, Json<CreateWorkItemResponse>), AppError> {
    let trigger_issue = i64::try_from(issue_number).unwrap_or(i64::MAX);
    // Correlation is published up front so an authorization refusal still points
    // at the session it was refused for.
    super::record_repo_correlation(&extensions, &owner, &name);
    super::record_trigger_correlation(&extensions, trigger_issue);
    let title = req.title.trim();
    // An omitted or blank body opens a body-less issue. Preserve a populated
    // body's original whitespace because indentation is meaningful Markdown.
    let body = req
        .body
        .as_deref()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_default();
    // Recorded BEFORE the first thing that can refuse the request, so a 400, a
    // 403, a 404, and a 409 all describe the same request shape a 201 does.
    // `selected_label` is deliberately absent here: the effective label is a
    // property of the SESSION, and resolving it takes the very GitHub reads that
    // can refuse the call. The caller's requested string is never a substitute —
    // it may name nothing at all — so the label is filled in by the refinement
    // below, immediately before the write.
    record(
        &extensions,
        &CreateWorkItemInput {
            owner: &owner,
            repo: &name,
            trigger_issue,
            selected_label: "",
            title,
            body,
        },
    );

    validate_repo_segment(&owner, "owner")?;
    validate_repo_segment(&name, "name")?;
    if issue_number == 0 {
        return Err(AppError::Validation(
            "issue_number must be a positive issue number".to_string(),
        ));
    }
    if title.is_empty() {
        return Err(AppError::Validation("title must not be blank".to_string()));
    }
    let requested_label = req.label.as_deref().map(str::trim).map(str::to_string);
    if requested_label.as_deref().is_some_and(str::is_empty) {
        return Err(AppError::Validation("label must not be blank".to_string()));
    }

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

    // Resolve creator attribution before parsing, exactly as reconciliation does.
    // This is what makes #1902's composer path route both human-authored and
    // App-authored seeded sessions to their actual owner.
    let creator = match effective_creator(
        &IssueMetadata {
            number: issue_number as i64,
            labels: trigger.labels.clone(),
            state: trigger.state.clone(),
            assignees: trigger.assignees.clone(),
            user_login: trigger.author_login.clone(),
            user_id: trigger.author_id,
        },
        state.config.reconcile.github_bot_login.as_deref(),
    ) {
        CreatorResolution::Resolved(creator) => creator,
        CreatorResolution::Unattributable { assignee_count, .. } => {
            return Err(AppError::Unprocessable(format!(
                "this session cannot accept work because its App-authored trigger must have exactly one assignee (found {assignee_count})"
            )))
        }
    };

    // Parse with the reconciler's own grammar: a malformed trigger surfaces the
    // parser's section-naming 422 rather than silently mislabeling the work item.
    let spec = parse_trigger_issue_body(&trigger.body)?;
    let mut reg = work_registration(
        &owner,
        &name,
        issue_number,
        trigger.author_id,
        trigger.author_login,
        creator,
        spec,
    );

    // Request-time authorization uses the same creator/global-admin/collaborator
    // predicate as reconciliation. It runs before package/label discovery so a
    // rejected caller learns nothing about the session's effective config.
    if !is_work_author_allowed(&reg, &state.config.access, user.id, &user.login) {
        return Err(AppError::Forbidden(format!(
            "your GitHub account lacks work-item authority on the session at \
             #{issue_number}: only its creator, a listed Session Collaborator, or a \
             deployment fkst administrator may queue work items"
        )));
    }

    // Resolve the same effective package + label graph as the reconcile
    // wake-gate. The signed-in user's token can read the public package sources
    // and keeps this mutation independent of any client-supplied label list.
    let effective = resolve_effective_packages(
        &gh.client,
        &gh.api_base,
        &token,
        std::slice::from_ref(&reg),
        &state.config.reconcile.mandatory_packages,
    )
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
    let logical = label_sets.remove(&reg.session_id).unwrap_or_default();
    let applicable = apply_work_label_namespace(
        &logical,
        state.config.reconcile.work_label_namespace.as_deref(),
    )
    .map_err(|error| {
        AppError::Unprocessable(format!(
            "this session cannot accept work because its effective work labels are invalid: {error}"
        ))
    })?;
    if applicable.effective.is_empty() {
        return Err(AppError::Unprocessable(
            "this session has no applicable work labels".to_string(),
        ));
    }
    let requested_work_label = requested_label
        .as_deref()
        .or(reg.def.work_label.as_deref())
        .ok_or_else(|| {
            AppError::Unprocessable(format!(
                "this session has no explicit work label; choose one of: {}",
                applicable.effective.join(", ")
            ))
        })?;
    let work_label = applicable
        .logical
        .iter()
        .zip(applicable.effective.iter())
        .find(|(logical, effective)| {
            logical.eq_ignore_ascii_case(requested_work_label)
                || effective.eq_ignore_ascii_case(requested_work_label)
        })
        .map(|(_, effective)| effective.clone())
        .ok_or_else(|| {
            AppError::Unprocessable(format!(
                "work label `{requested_work_label}` is not applicable to this session; \
                 refresh the dashboard and choose one of: {}",
                applicable.effective.join(", ")
            ))
        })?;

    // The same argument set, refined immediately before the GitHub write — the
    // operation's only side effect — with the RESOLVED label. Every other
    // property is unchanged, which is the only shape a refinement may take.
    // The title and body are free-form issue text and contribute only their byte
    // sizes.
    refine(
        &extensions,
        &CreateWorkItemInput {
            owner: &owner,
            repo: &name,
            trigger_issue,
            selected_label: &work_label,
            title,
            body,
        },
    );

    let labels = vec![work_label.clone()];
    let assignees = vec![reg.creator_login.clone()];
    let created = gh
        .create_issue(&token, &owner, &name, title, body, &labels, &assignees)
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
    trigger_author_login: String,
    creator: SessionCreator,
    spec: TriggerSpec,
) -> SessionRegistration {
    let hash = config_hash(
        &spec.packages,
        spec.work_label.as_deref(),
        spec.environment.as_deref(),
        spec.output_lang.as_deref(),
        &spec.engine_config,
        &spec.manifest_refs,
        spec.source_branch.as_deref(),
        spec.target_branch.as_deref(),
        &spec.package_env,
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
        trigger_author_login,
        creator_login: creator.login,
        creator_id: creator.id,
        def: SessionDef {
            name: spec.name,
            packages: spec.packages,
            manifest_refs: spec.manifest_refs,
            work_label: spec.work_label,
            environment: spec.environment,
            output_lang: spec.output_lang,
            engine_config: spec.engine_config,
            source_branch: spec.source_branch,
            target_branch: spec.target_branch,
            package_env: spec.package_env,
        },
        effective_packages,
        session_id: format!("canvas-work-item-{trigger_issue}"),
        config_hash: hash,
        auto_merge: spec.auto_merge,
        log_access: spec.log_access,
        collaborators: spec.collaborators,
        effective_package_env: crate::goals::package_env::PackageEnv::new(),
    }
}

/// The trigger-issue read this endpoint uniquely needs (the body-bearing one).
#[path = "work_item_trigger.rs"]
mod trigger;

#[cfg(test)]
#[path = "work_item_tests.rs"]
mod tests;
