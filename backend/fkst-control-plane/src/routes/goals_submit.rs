//! The unified `POST /api/v1/goals/submit` endpoint handler (#178).
//!
//! One user-facing call that starts a session from EITHER an existing GitHub
//! issue reference OR all arguments inline. It ORCHESTRATES the existing
//! create-goal + trigger machinery — it does NOT reimplement the runner: it
//! wraps [`crate::goals::GoalIssueStore`],
//! [`crate::vault::VaultService::set_inline`], [`crate::ornn::validate_pins`],
//! and stops at [`crate::sessions::SessionService::create_for_goal`]. The two
//! legacy endpoints (`POST /goals`, `POST /goals/:id/trigger`) are untouched.
//!
//! This module is a sibling of [`super::goals`] purely for file-size hygiene
//! (the `goals` route file is already large); the route is still mounted by
//! `super::goals::router()`. The wire DTOs + reference parsers live in
//! [`super::goals_submit_dto`]; the issue-BODY template parser lives in
//! [`crate::goals::issue_parse`]. It reuses `goals`' `InlineSecretInput` and
//! `goal_ownership` rather than introducing parallel types — a non-secret env
//! var is just an `InlineSecretInput { kind: variable }`.
//!
//! ## Secret/prompt invariant (and its single documented divergence)
//!
//! For a SERVER-created issue (the inline source) the body is only
//! `non_sensitive_summary + marker` — never the goal prompt and never any
//! secret/env value (those flow exclusively through the in-memory vault).
//!
//! The issue source diverges ON PURPOSE: when adopting a USER-AUTHORED issue,
//! that issue's `### Goal` section becomes `GoalDoc.description` (the engine
//! prompt), so the prompt lives in the **user's own** issue by design. This is
//! the ONLY exception to "the prompt is never in GitHub," and it applies solely
//! to issues the user authored — never to server-created issues. The server's
//! own `patch_issue` on adoption still writes only the non-sensitive
//! summary+marker; it never echoes the prompt back.

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use secrecy::SecretString;

use crate::auth::AuthContext;
use crate::authz::permissions::{self, require_permission};
use crate::error::AppError;
use crate::github_hub::service::{self as github_hub, RepoRef as HubRepoRef};
use crate::github_hub::NyxIdGithubProxy;
use crate::goals::{
    parse_goal_issue_body, validate_goal_fields, GoalDoc, GoalStatus, ParsedGoal, RepoRef,
};
use crate::ornn::OrnnSkillPin;
use crate::routes::extract::AppJson;
use crate::sessions::GoalTriggerInfo;
use crate::state::AppState;
use crate::vault::{EnvKind, EnvScopeRef};

use super::goals::{goal_ownership, InlineSecretInput};
use super::goals_submit_dto::{
    IssueRef, RepoSpecBody, SubmitSessionRequest, SubmitSessionResponse,
};

/// The title prefix the `fkst-goal` Issue Form (#177) stamps. Stripped from an
/// adopted issue's title to derive `GoalDoc.title`.
const GOAL_TITLE_PREFIX: &str = "[fkst-goal]: ";

/// `POST /api/v1/goals/submit`: start a session from a GitHub issue or inline.
///
/// Authorization mirrors `create` + `trigger`: action-layer `GOAL_CREATE` then
/// `GOAL_TRIGGER` (admin bypasses both), then the same object-layer check
/// `trigger` runs (owner short-circuit / admin / else org-writer).
///
/// Secret/prompt invariant: see the module doc-comment. For the INLINE source
/// the filed issue carries only `non_sensitive_summary + marker` (never the
/// prompt, never a secret). The ISSUE source intentionally diverges — the
/// user-authored issue's own `### Goal` body becomes the prompt and lives in
/// the user's own issue by design; the server's adoption patch still writes
/// only the non-sensitive summary + marker.
pub async fn submit(
    State(state): State<AppState>,
    ctx: AuthContext,
    AppJson(request): AppJson<SubmitSessionRequest>,
) -> Result<(StatusCode, Json<SubmitSessionResponse>), AppError> {
    // 1. Action layer: BOTH create and trigger are required (this one call does
    //    both). Admin bypasses each.
    require_permission(&ctx, permissions::GOAL_CREATE)?;
    require_permission(&ctx, permissions::GOAL_TRIGGER)?;

    // 2. Resolve the source into a goal + its bound issue (creating or adopting).
    let Resolved {
        goal,
        repo,
        issue_number,
        ornn_skills,
        secrets,
    } = match request {
        SubmitSessionRequest::Issue { issue, secrets } => {
            resolve_issue_source(&state, &ctx, issue, secrets).await?
        }
        SubmitSessionRequest::Inline {
            goal,
            repo,
            package_names,
            ornn_skills,
            secrets,
        } => {
            resolve_inline_source(
                &state,
                &ctx,
                goal,
                repo,
                package_names,
                ornn_skills,
                secrets,
            )
            .await?
        }
    };

    // 2b. Object layer: the SAME primitives `trigger` uses — owner short-circuit
    //     / admin / else org-writer against the goal's org_id.
    authorize_object(&state, &ctx, &goal).await?;

    // 5. Boundary-validate Ornn pins on BOTH paths (issue pins came from the
    //    parser, inline pins from the body) before any session is placed.
    crate::ornn::validate_pins(&ornn_skills)?;

    // 6. Pre-flight validation seam (#179): package-correctness + Ornn
    //    availability must run HERE before `create_for_goal`. #179 is not yet
    //    merged — leave an observable seam rather than silently proceeding.
    // TODO(#179 pre-flight): call the package-correctness + Ornn-availability
    // validation here before create_for_goal; do NOT skip.
    tracing::warn!(
        goal_id = %goal.id,
        "pre-flight validation seam: #179 not yet wired"
    );

    // 7. Inline secrets/variables: held in the controller's in-memory vault for
    //    this session's repo scope (exactly as `trigger` does). Never logged.
    if let Some(inputs) = secrets {
        if !inputs.is_empty() {
            let scope = EnvScopeRef::repo(&repo.owner, &repo.name);
            let entries: Vec<(String, EnvKind, SecretString)> = inputs
                .into_iter()
                .map(|input| (input.key, input.kind, SecretString::from(input.value)))
                .collect();
            state
                .vault
                .set_inline(&goal.owner_user_id, &scope, entries)?;
        }
    }

    // 8. Delegate to the orchestration boundary (#178 stops here — no engine
    //    path). Forward the caller's token for per-session NyxID provisioning.
    let trigger_info = GoalTriggerInfo {
        goal_id: goal.id,
        repo: repo.clone(),
        package_names: goal.package_names.clone(),
        owner_user_id: goal.owner_user_id.clone(),
        org_id: goal.org_id.clone(),
        prior_status: goal.status,
        ornn_skills: (!ornn_skills.is_empty()).then_some(ornn_skills),
    };
    let result = state
        .sessions
        .create_for_goal(&state.goals, trigger_info, ctx.user_access_token.clone())
        .await?;

    // 9. Compose the issue locator + 202.
    let issue_url = format!(
        "https://github.com/{}/{}/issues/{}",
        repo.owner, repo.name, issue_number
    );
    tracing::info!(
        goal_id = %goal.id,
        session_id = %result.session_id,
        issue = issue_number,
        "session submitted"
    );
    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitSessionResponse {
            goal_id: goal.id.to_string(),
            session_id: result.session_id.to_string(),
            issue_number,
            issue_url,
            goal_status: result.goal_status,
            session_status: "pending",
        }),
    ))
}

/// The resolved-source outcome shared by both arms before orchestration.
struct Resolved {
    goal: GoalDoc,
    repo: RepoRef,
    issue_number: u64,
    ornn_skills: Vec<OrnnSkillPin>,
    secrets: Option<Vec<InlineSecretInput>>,
}

/// Issue source: fetch + parse the adopted issue, build the goal, adopt it.
async fn resolve_issue_source(
    state: &AppState,
    ctx: &AuthContext,
    issue: IssueRef,
    secrets: Option<Vec<InlineSecretInput>>,
) -> Result<Resolved, AppError> {
    let (repo_body, number) = issue.resolve()?;
    let repo = RepoRef {
        owner: repo_body.owner.clone(),
        name: repo_body.name.clone(),
    };

    // Fetch the user-authored issue body via the NyxID GitHub proxy (the same
    // seam the hub uses). The caller's own token authorizes the read.
    let proxy = NyxIdGithubProxy::from_context(&state.authz, ctx).await?;
    let hub_repo = HubRepoRef::new(&repo_body.owner, &repo_body.name)?;
    let view = github_hub::get_issue(&proxy, &hub_repo, number, None).await?;
    let body = view.body.ok_or_else(|| {
        AppError::Unprocessable("the referenced issue has no body to parse".to_string())
    })?;

    // Parse the body into structured goal fields (422 naming the section on a
    // template-format failure). Then derive the title from the issue title.
    let ParsedGoal {
        description,
        package_names,
        ornn_skills,
    } = parse_goal_issue_body(&body)?;
    let title = derive_title(&view.title);

    // The parser deliberately defers package-name grammar; validate it now via
    // the shared field validator (same as the inline source).
    validate_goal_fields(&title, &description, &package_names, Some(&repo))
        .map_err(AppError::Unprocessable)?;

    let goal = build_goal(ctx, title, description, package_names, &repo);
    state.goals.adopt_issue(&goal, &repo, number).await?;

    Ok(Resolved {
        goal,
        repo,
        issue_number: number,
        ornn_skills,
        secrets,
    })
}

/// Inline source: validate the inline args, file a fresh issue, read its number.
#[allow(clippy::too_many_arguments)]
async fn resolve_inline_source(
    state: &AppState,
    ctx: &AuthContext,
    goal_prompt: String,
    repo_spec: RepoSpecBody,
    package_names: Vec<String>,
    ornn_skills: Option<Vec<OrnnSkillPin>>,
    secrets: Option<Vec<InlineSecretInput>>,
) -> Result<Resolved, AppError> {
    let repo = repo_spec.resolve()?;
    // The inline source has no separate title; derive a non-empty one from the
    // repo (the prompt is secret and must never seed a server-visible title).
    let title = format!("{}/{}", repo.owner, repo.name);

    // Inline field validation stays on the EXISTING 400 path (mirrors `create`),
    // distinct from the new parsers' 422 contract.
    validate_goal_fields(&title, &goal_prompt, &package_names, Some(&repo))
        .map_err(AppError::Validation)?;

    let goal = build_goal(ctx, title, goal_prompt, package_names, &repo);
    state.goals.insert(&goal).await?;
    let issue_number = state.goals.issue_number(goal.id).await.ok_or_else(|| {
        // The store files the issue synchronously on `insert` when `repo` is set;
        // a missing number means the GitHub mirror write failed.
        AppError::Unavailable("the goal issue could not be filed on the target repo".to_string())
    })?;

    Ok(Resolved {
        goal,
        repo,
        issue_number,
        ornn_skills: ornn_skills.unwrap_or_default(),
        secrets,
    })
}

/// Build a fresh `NotStarted` goal owned by the caller.
fn build_goal(
    ctx: &AuthContext,
    title: String,
    description: String,
    package_names: Vec<String>,
    repo: &RepoRef,
) -> GoalDoc {
    let now = bson::DateTime::now();
    GoalDoc {
        id: bson::Uuid::new(),
        title: title.trim().to_string(),
        description,
        package_names,
        repo: Some(repo.clone()),
        status: GoalStatus::NotStarted,
        owner_user_id: ctx.user_id.clone(),
        // A submit-created goal is always owned by the caller (no org_id field
        // exists on the submit body, mirroring how `create` only sets an org_id
        // from an explicit request field). It is therefore owner-scoped, and the
        // object-layer check below always short-circuits on owner.
        org_id: None,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    }
}

/// Strip the `fkst-goal` form's title prefix; fall back to the slug if the
/// remainder is empty (the field validator rejects an empty title anyway).
fn derive_title(issue_title: &str) -> String {
    issue_title
        .strip_prefix(GOAL_TITLE_PREFIX)
        .unwrap_or(issue_title)
        .trim()
        .to_string()
}

/// The object-layer authz `trigger` runs: owner short-circuit, admin bypass,
/// else org-writer against the goal's org. Reuses `goal_ownership` (no new
/// helper) per the issue's instruction.
async fn authorize_object(
    state: &AppState,
    ctx: &AuthContext,
    goal: &GoalDoc,
) -> Result<(), AppError> {
    let ownership = goal_ownership(goal);
    if ownership.owner_user_id == Some(ctx.user_id.as_str())
        || ctx.has_permission(permissions::ADMIN)
    {
        return Ok(());
    }
    match &goal.org_id {
        Some(org_id) => state.authz.require_org_writer(ctx, org_id).await,
        None => Err(AppError::Forbidden(
            "insufficient permissions: only the owner can submit this goal".to_string(),
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx_with(perms: &[&str]) -> AuthContext {
        AuthContext {
            user_id: "u".to_string(),
            email: String::new(),
            display_name: "u".to_string(),
            roles: vec![],
            permissions: perms.iter().map(|p| p.to_string()).collect(),
            groups: vec![],
            user_access_token: None,
        }
    }

    /// The submit handler's action gate runs `GOAL_CREATE` then `GOAL_TRIGGER`.
    /// A caller with only `GOAL_CREATE` (missing `fkst:goal:trigger`) is denied
    /// at the trigger gate with a 403 — the same primitive the handler calls.
    #[test]
    fn missing_goal_trigger_permission_is_forbidden() {
        let ctx = ctx_with(&[permissions::GOAL_CREATE]);
        // create passes...
        require_permission(&ctx, permissions::GOAL_CREATE).expect("create allowed");
        // ...trigger does not.
        let err = require_permission(&ctx, permissions::GOAL_TRIGGER)
            .expect_err("trigger must be denied");
        assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
    }

    #[test]
    fn admin_bypasses_both_action_gates() {
        let ctx = ctx_with(&[permissions::ADMIN]);
        require_permission(&ctx, permissions::GOAL_CREATE).expect("admin -> create");
        require_permission(&ctx, permissions::GOAL_TRIGGER).expect("admin -> trigger");
    }

    /// `derive_title` strips the `fkst-goal` form prefix and trims.
    #[test]
    fn derive_title_strips_form_prefix() {
        assert_eq!(
            derive_title("[fkst-goal]: Build the thing"),
            "Build the thing"
        );
        assert_eq!(derive_title("No prefix here"), "No prefix here");
    }
}
