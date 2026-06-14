//! Goals HTTP API: CRUD for `/api/v1/goals`.
//!
//! Endpoints:
//! - `POST   /api/v1/goals`             — create a goal (201)
//! - `GET    /api/v1/goals`             — list goals (200, paginated)
//! - `GET    /api/v1/goals/{id}`        — fetch one goal (200)
//! - `PATCH  /api/v1/goals/{id}`        — partial update (200)
//! - `DELETE /api/v1/goals/{id}`        — delete (204)
//! - `POST   /api/v1/goals/{id}/trigger` — trigger a goal (202)
//!
//! This is purely the web edge: wire DTOs, UUID parsing, authz checks, and
//! status mapping. Validation logic lives in the goals domain module.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use bson::doc;
use serde::{Deserialize, Serialize};

use crate::auth::AuthContext;
use crate::authz::{Action, Ownership};
use crate::error::AppError;
use crate::goals::{
    validate_goal_fields, CreateRepoSpec, GoalDoc, GoalStatus, RepoRef, MAX_GOAL_DESCRIPTION_BYTES,
    MAX_GOAL_TITLE_CHARS,
};
use crate::routes::extract::AppJson;
use crate::routes::rfc3339;
use crate::sessions::GoalTriggerInfo;
use crate::state::AppState;

/// Statuses that allow mutation of package_names, repo, and deletion.
const MUTABLE_STATUSES: [GoalStatus; 3] = [
    GoalStatus::NotStarted,
    GoalStatus::Stopped,
    GoalStatus::Failed,
];

// ---- DTOs ---------------------------------------------------------------

/// Request body for `POST /api/v1/goals`. Unknown fields are denied.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateGoalRequest {
    pub title: String,
    /// The engine-facing goal prompt; content NEVER logged.
    pub description: String,
    pub package_names: Vec<String>,
    /// Optional GitHub repo reference.
    #[serde(default)]
    pub repo: Option<RepoRefBody>,
    /// Attach the goal to an org the caller belongs to (member+).
    #[serde(default)]
    pub org_id: Option<String>,
}

/// Request body for `PATCH /api/v1/goals/{id}`. Unknown fields are denied.
/// Absent fields are unchanged.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PatchGoalRequest {
    pub title: Option<String>,
    /// The engine-facing goal prompt; content NEVER logged.
    pub description: Option<String>,
    pub package_names: Option<Vec<String>>,
    /// Set the repo (mutually exclusive with `clear_repo`).
    pub repo: Option<RepoRefBody>,
    /// `true` clears the repo; mutually exclusive with `repo`.
    #[serde(default)]
    pub clear_repo: Option<bool>,
}

/// GitHub repo reference in request bodies.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RepoRefBody {
    pub owner: String,
    pub name: String,
}

/// Response body for goal endpoints (mirrors `GoalDoc` with string UUID and
/// RFC3339 timestamps, explicit nulls, snake_case status).
#[derive(Debug, Serialize)]
pub struct GoalView {
    pub id: String,
    pub title: String,
    pub description: String,
    pub package_names: Vec<String>,
    pub repo: Option<RepoRefView>,
    pub status: GoalStatus,
    pub owner_user_id: String,
    pub org_id: Option<String>,
    pub active_session_id: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// Repo reference in responses.
#[derive(Debug, Serialize)]
pub struct RepoRefView {
    pub owner: String,
    pub name: String,
}

/// Query parameters for `GET /api/v1/goals`.
#[derive(Debug, Deserialize, Default)]
pub struct ListGoalsQuery {
    pub status: Option<String>,
    pub limit: Option<u64>,
    pub offset: Option<u64>,
}

/// Default value for `repo_mode` field: `RepoMode::Existing`.
fn default_repo_mode() -> RepoMode {
    RepoMode::Existing
}

/// Default value for boolean fields that default to `true`.
fn default_true() -> bool {
    true
}

/// How the trigger handler should resolve the target repository.
#[derive(Debug, Clone, Copy, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RepoMode {
    /// Use an existing repo (the stored goal repo or `repo` override).
    #[default]
    Existing,
    /// Create a new GitHub repo via the NyxID proxy before triggering.
    CreateNew,
}

/// Request body for `POST /api/v1/goals/{id}/trigger`. Unknown fields denied.
/// The `repo` field is optional: when absent, the goal's stored repo is used.
/// When `repo_mode` is `create_new`, the `create` field is required and
/// specifies the new repository to create.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TriggerRequest {
    /// Override the goal's stored repo for this trigger (only for `existing` mode).
    #[serde(default)]
    pub repo: Option<RepoRefBody>,
    /// Whether to use an existing repo or create a new one.
    #[serde(default = "default_repo_mode")]
    pub repo_mode: RepoMode,
    /// Specification for the new repo to create. Required when `repo_mode` is
    /// `create_new`; forbidden when `repo_mode` is `existing`.
    #[serde(default)]
    pub create: Option<CreateRepoSpecBody>,
}

/// Request-body specification for creating a new GitHub repo during trigger.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRepoSpecBody {
    /// Repository name (required).
    pub name: String,
    /// Whether the repo should be private (defaults to `true`).
    #[serde(default = "default_true")]
    pub private: bool,
    /// Optional description for the repository.
    #[serde(default)]
    pub description: Option<String>,
    /// If set, create under this org; otherwise under the authenticated user.
    #[serde(default)]
    pub org_login: Option<String>,
}

/// Response body for `POST /api/v1/goals/{id}/trigger` (202).
#[derive(Debug, Serialize)]
pub struct TriggerResponse {
    pub goal_id: String,
    pub session_id: String,
    pub goal_status: GoalStatus,
    pub session_status: &'static str,
}

impl TryFrom<GoalDoc> for GoalView {
    type Error = AppError;

    fn try_from(doc: GoalDoc) -> Result<Self, Self::Error> {
        Ok(GoalView {
            id: doc.id.to_string(),
            title: doc.title,
            description: doc.description,
            package_names: doc.package_names,
            repo: doc.repo.map(|r| RepoRefView {
                owner: r.owner,
                name: r.name,
            }),
            status: doc.status,
            owner_user_id: doc.owner_user_id,
            org_id: doc.org_id,
            active_session_id: doc.active_session_id.map(|id| id.to_string()),
            created_at: rfc3339(doc.created_at)?,
            updated_at: rfc3339(doc.updated_at)?,
        })
    }
}

// ---- Helpers ---------------------------------------------------------------

/// Parse a UUID path parameter. Returns `AppError::Validation` for malformed
/// UUIDs (400, not 404).
fn parse_goal_uuid(id: &str) -> Result<bson::Uuid, AppError> {
    bson::Uuid::parse_str(id)
        .map_err(|_| AppError::Validation("invalid goal id: must be a UUID".to_string()))
}

/// Build the ownership struct from a goal doc for authz checks.
fn goal_ownership(doc: &GoalDoc) -> Ownership<'_> {
    Ownership {
        owner_user_id: Some(&doc.owner_user_id),
        org_id: doc.org_id.as_deref(),
    }
}

/// Statuses where package_names, repo, and deletion are allowed.
fn mutable_statuses() -> Vec<GoalStatus> {
    MUTABLE_STATUSES.to_vec()
}

// ---- Handlers ---------------------------------------------------------------

/// `POST /api/v1/goals`: validate and create a goal. Returns 201 with
/// Location header.
async fn create(
    State(state): State<AppState>,
    ctx: AuthContext,
    AppJson(request): AppJson<CreateGoalRequest>,
) -> Result<(StatusCode, [(HeaderName, HeaderValue); 1], Json<GoalView>), AppError> {
    // Org membership check (if org_id provided).
    if let Some(ref org_id) = request.org_id {
        state.authz.require_org_writer(&ctx, org_id).await?;
    }

    // Pure field validation.
    let repo_ref = request.repo.as_ref().map(|r| RepoRef {
        owner: r.owner.clone(),
        name: r.name.clone(),
    });
    validate_goal_fields(
        &request.title,
        &request.description,
        &request.package_names,
        repo_ref.as_ref(),
    )
    .map_err(AppError::Validation)?;

    // Validate each package exists and caller can use it.
    for name in &request.package_names {
        let pkg = state.packages.get(name).await?;
        match pkg {
            Some(p) => {
                let can_use = state
                    .authz
                    .can_use_package(&ctx, name, p.owner_user_id.as_deref(), p.org_id.as_deref())
                    .await;
                if !can_use {
                    return Err(AppError::Forbidden(format!("package not usable: {name}")));
                }
            }
            None => {
                return Err(AppError::Validation(format!("package not found: {name}")));
            }
        }
    }

    let now = bson::DateTime::now();
    let id = bson::Uuid::new();
    let goal = GoalDoc {
        id,
        title: request.title.trim().to_string(),
        description: request.description,
        package_names: request.package_names,
        repo: repo_ref,
        status: GoalStatus::NotStarted,
        owner_user_id: ctx.user_id.clone(),
        org_id: request.org_id,
        active_session_id: None,
        created_at: now,
        updated_at: now,
    };

    state.goals.insert(&goal).await?;

    let location = HeaderValue::try_from(format!("/api/v1/goals/{}", goal.id))
        .expect("UUID is ASCII and header-safe");

    // Log lengths only, never description content.
    tracing::info!(
        goal_id = %goal.id,
        title = %goal.title,
        description_bytes = goal.description.len(),
        packages = goal.package_names.len(),
        "goal created"
    );

    Ok((
        StatusCode::CREATED,
        [(header::LOCATION, location)],
        Json(GoalView::try_from(goal)?),
    ))
}

/// `GET /api/v1/goals`: list goals visible to the caller (owned + org).
/// Supports `?status=`, `?limit=` (default 50, max 200), `?offset=`.
async fn list(
    State(state): State<AppState>,
    ctx: AuthContext,
    Query(query): Query<ListGoalsQuery>,
) -> Result<Json<Vec<GoalView>>, AppError> {
    let org_ids = state.authz.visible_org_ids(&ctx).await?;

    let status: Option<GoalStatus> = match query.status.as_deref() {
        Some(s) => Some(
            serde_json::from_value(serde_json::Value::String(s.to_string()))
                .map_err(|_| AppError::Validation(format!("invalid status filter: {s}")))?,
        ),
        None => None,
    };

    let limit = query.limit.unwrap_or(50).min(200);
    let offset = query.offset.unwrap_or(0);

    let goals = state
        .goals
        .list(&ctx.user_id, &org_ids, status, limit, offset)
        .await?;

    let views: Vec<GoalView> = goals
        .into_iter()
        .map(GoalView::try_from)
        .collect::<Result<Vec<_>, _>>()?;

    tracing::debug!(count = views.len(), "goals listed");
    Ok(Json(views))
}

/// `GET /api/v1/goals/{id}`: fetch one goal. Performs read-repair for
/// dangling triggered/running goals with no active session.
async fn get_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<GoalView>, AppError> {
    let uuid = parse_goal_uuid(&id)?;
    let mut goal = state
        .goals
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("goal not found: {id}")))?;

    let ownership = goal_ownership(&goal);
    state
        .authz
        .authorize(&ctx, ownership, Action::Read, "goal", &id)
        .await?;

    // Read-repair: if status is triggered/running but there is no active
    // session, or the session is terminal, repair to stopped/failed.
    if matches!(goal.status, GoalStatus::Triggered | GoalStatus::Running) {
        let needs_repair = if let Some(session_id) = goal.active_session_id {
            // Check if the session is terminal.
            match state.sessions.get(session_id).await {
                Ok(Some(session)) => {
                    matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped
                            | crate::models::SessionStatus::Failed
                    )
                }
                Ok(None) => true, // Session gone
                Err(_) => false,  // Don't repair on DB error
            }
        } else {
            // No active_session_id: only repair if older than 5 minutes
            // (give the trigger flow time to complete).
            if goal.status == GoalStatus::Triggered {
                let age =
                    bson::DateTime::now().timestamp_millis() - goal.updated_at.timestamp_millis();
                age > 300_000 // 5 minutes
            } else {
                true
            }
        };

        if needs_repair {
            let repair_status = match goal.status {
                GoalStatus::Triggered => GoalStatus::Stopped,
                GoalStatus::Running => GoalStatus::Failed,
                _ => goal.status,
            };
            tracing::info!(
                goal_id = %id,
                from = ?goal.status,
                to = ?repair_status,
                "read-repair: goal has dangling active session"
            );
            if let Some(repaired) = state
                .goals
                .transition_status(
                    uuid,
                    &[goal.status],
                    doc! {
                        "status": bson::to_bson(&repair_status).expect("GoalStatus serializes"),
                        "active_session_id": bson::Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    },
                )
                .await?
            {
                goal = repaired;
            }
        }
    }

    tracing::debug!(goal_id = %id, "goal fetched");
    Ok(Json(GoalView::try_from(goal)?))
}

/// `PATCH /api/v1/goals/{id}`: partial update. Title/description editable in
/// any status; package_names/repo only in {not_started, stopped, failed}.
async fn update(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
    AppJson(body): AppJson<PatchGoalRequest>,
) -> Result<Json<GoalView>, AppError> {
    let uuid = parse_goal_uuid(&id)?;

    // Fetch existing for authz check.
    let existing = state
        .goals
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("goal not found: {id}")))?;

    let ownership = goal_ownership(&existing);
    state
        .authz
        .authorize(&ctx, ownership, Action::Write, "goal", &id)
        .await?;

    // Mutually exclusive: repo + clear_repo.
    if body.repo.is_some() && body.clear_repo == Some(true) {
        return Err(AppError::Validation(
            "repo and clear_repo are mutually exclusive".to_string(),
        ));
    }

    // Determine whether the update touches mutable-only fields.
    let touches_mutable =
        body.package_names.is_some() || body.repo.is_some() || body.clear_repo.is_some();

    // Build the `$set` document.
    let mut set = bson::Document::new();
    let mut needs_mutable_status = false;

    if let Some(ref title) = body.title {
        let trimmed = title.trim();
        if trimmed.is_empty() {
            return Err(AppError::Validation("empty title".to_string()));
        }
        if trimmed.len() > MAX_GOAL_TITLE_CHARS {
            return Err(AppError::Validation(format!(
                "title too long: {} chars exceeds {MAX_GOAL_TITLE_CHARS}",
                trimmed.len()
            )));
        }
        set.insert("title", trimmed.to_string());
    }

    if let Some(ref description) = body.description {
        if description.is_empty() {
            return Err(AppError::Validation("empty description".to_string()));
        }
        if description.len() > MAX_GOAL_DESCRIPTION_BYTES {
            return Err(AppError::Validation(format!(
                "description too large: {} bytes exceeds {MAX_GOAL_DESCRIPTION_BYTES}",
                description.len()
            )));
        }
        // Log length only, never content.
        tracing::debug!(
            description_bytes = description.len(),
            "goal description updated"
        );
        set.insert(
            "description",
            bson::to_bson(description).expect("description serializes"),
        );
    }

    if let Some(ref package_names) = body.package_names {
        // Validate count.
        if package_names.is_empty() {
            return Err(AppError::Validation(
                "at least one package is required".to_string(),
            ));
        }
        // Validate each package: format, existence, can_use.
        let mut seen = std::collections::HashSet::new();
        for name in package_names {
            if name.len() > crate::goals::MAX_PACKAGE_NAME_BYTES {
                return Err(AppError::Validation(format!(
                    "package name too long: {:?} exceeds {} bytes",
                    name,
                    crate::goals::MAX_PACKAGE_NAME_BYTES
                )));
            }
            if !crate::packages::is_valid_name(name) {
                return Err(AppError::Validation(format!(
                    "invalid package name: {:?}",
                    name
                )));
            }
            if !seen.insert(name.to_lowercase()) {
                return Err(AppError::Validation(format!(
                    "duplicate package name: {:?}",
                    name
                )));
            }
            let pkg = state.packages.get(name).await?;
            match pkg {
                Some(p) => {
                    let can_use = state
                        .authz
                        .can_use_package(
                            &ctx,
                            name,
                            p.owner_user_id.as_deref(),
                            p.org_id.as_deref(),
                        )
                        .await;
                    if !can_use {
                        return Err(AppError::Forbidden(format!("package not usable: {name}")));
                    }
                }
                None => {
                    return Err(AppError::Validation(format!("package not found: {name}")));
                }
            }
        }
        set.insert(
            "package_names",
            bson::to_bson(package_names).expect("package_names serializes"),
        );
        needs_mutable_status = true;
    }

    if let Some(ref repo) = body.repo {
        let repo_ref = RepoRef {
            owner: repo.owner.clone(),
            name: repo.name.clone(),
        };
        // Validate repo format.
        validate_goal_fields("dummy", "dummy", &["dummy".to_string()], Some(&repo_ref)).map_err(
            |e| {
                // Strip the dummy-related prefix and re-contextualize.
                AppError::Validation(e)
            },
        )?;
        set.insert(
            "repo",
            bson::to_bson(&Some(repo_ref)).expect("repo serializes"),
        );
        needs_mutable_status = true;
    }

    if body.clear_repo == Some(true) {
        set.insert("repo", bson::Bson::Null);
        needs_mutable_status = true;
    }

    set.insert("updated_at", bson::DateTime::now());

    // Determine the mutability CAS filter.
    let mutability_filter = if touches_mutable || needs_mutable_status {
        Some(mutable_statuses())
    } else {
        // Only title/description changes: no status restriction.
        None
    };

    let updated = state
        .goals
        .patch(uuid, mutability_filter, set)
        .await?
        .ok_or_else(|| {
            // Either not found, or status prevents the mutation.
            // Check if the goal exists to disambiguate.
            AppError::Conflict("goal cannot be modified in the current status".to_string())
        })?;

    tracing::info!(goal_id = %id, "goal updated");
    Ok(Json(GoalView::try_from(updated)?))
}

/// `DELETE /api/v1/goals/{id}`: delete a goal. Only allowed in {not_started,
/// stopped, failed}.
async fn delete_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<StatusCode, AppError> {
    let uuid = parse_goal_uuid(&id)?;

    // Fetch existing for authz check.
    let existing = state
        .goals
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("goal not found: {id}")))?;

    let ownership = goal_ownership(&existing);
    state
        .authz
        .authorize(&ctx, ownership, Action::Manage, "goal", &id)
        .await?;

    let deleted = state
        .goals
        .delete(uuid, mutable_statuses())
        .await?
        .ok_or_else(|| {
            // Goal exists but is in a status that doesn't allow deletion.
            AppError::Conflict("goal cannot be deleted in the current status".to_string())
        })?;

    tracing::info!(goal_id = %deleted.id, "goal deleted");
    Ok(StatusCode::NO_CONTENT)
}

/// `POST /api/v1/goals/{id}/trigger`: trigger a goal, creating a new session.
/// Returns 202 on success.
///
/// Supports two repo modes:
/// - `existing` (default): use the goal's stored repo or a `repo` override.
/// - `create_new`: create a new GitHub repo via NyxID proxy, then trigger.
///
/// Authorization: caller is the goal owner OR the goal has an org_id and the
/// caller's org role is admin or member (viewers excluded).
async fn trigger(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
    AppJson(body): AppJson<TriggerRequest>,
) -> Result<(StatusCode, Json<TriggerResponse>), AppError> {
    let uuid = parse_goal_uuid(&id)?;

    // Cross-field validation for repo_mode.
    match body.repo_mode {
        RepoMode::CreateNew => {
            if body.create.is_none() {
                return Err(AppError::Validation(
                    "create_new mode requires the 'create' field".to_string(),
                ));
            }
            if body.repo.is_some() {
                return Err(AppError::Validation(
                    "create_new mode forbids the 'repo' field".to_string(),
                ));
            }
        }
        RepoMode::Existing => {
            if body.create.is_some() {
                return Err(AppError::Validation(
                    "existing mode forbids the 'create' field".to_string(),
                ));
            }
        }
    }

    // Step 1: Load goal.
    let mut goal = state
        .goals
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("goal not found: {id}")))?;

    // Authorization check: owner can always trigger; org members (admin,
    // member) can trigger org goals -- NOT viewers.
    {
        let ownership = goal_ownership(&goal);
        // Owner or admin-scope: always allowed (checked by authorize with Write
        // action). For non-owners with an org, we need to check org membership
        // at member+ level.
        if ownership.owner_user_id != Some(ctx.user_id.as_str())
            && !ctx.has_scope(crate::authz::ADMIN_SCOPE)
        {
            if let Some(ref org_id) = goal.org_id {
                state.authz.require_org_writer(&ctx, org_id).await?;
            } else {
                // Not the owner, no org -- forbidden.
                return Err(AppError::Forbidden(
                    "insufficient permissions: only the owner can trigger this goal".to_string(),
                ));
            }
        }
    }

    // Read-repair: if status is triggered/running and active_session_id is
    // terminal or absent, CAS the goal to stopped/failed first.
    if matches!(goal.status, GoalStatus::Triggered | GoalStatus::Running) {
        let needs_repair = if let Some(session_id) = goal.active_session_id {
            match state.sessions.get(session_id).await {
                Ok(Some(session)) => {
                    matches!(
                        session.status,
                        crate::models::SessionStatus::Stopped
                            | crate::models::SessionStatus::Failed
                    )
                }
                Ok(None) => true,
                Err(_) => false,
            }
        } else if goal.status == GoalStatus::Triggered {
            let age = bson::DateTime::now().timestamp_millis() - goal.updated_at.timestamp_millis();
            age > 300_000 // 5 minutes
        } else {
            true
        };

        if needs_repair {
            let repair_status = match goal.status {
                GoalStatus::Triggered => GoalStatus::Stopped,
                GoalStatus::Running => GoalStatus::Failed,
                _ => goal.status,
            };
            tracing::info!(
                goal_id = %id,
                from = ?goal.status,
                to = ?repair_status,
                "read-repair: goal has dangling active session during trigger"
            );
            if let Some(repaired) = state
                .goals
                .transition_status(
                    uuid,
                    &[goal.status],
                    doc! {
                        "status": bson::to_bson(&repair_status).expect("GoalStatus serializes"),
                        "active_session_id": bson::Bson::Null,
                        "updated_at": bson::DateTime::now(),
                    },
                )
                .await?
            {
                goal = repaired;
            }
        } else {
            // Still actively triggered or running -- conflict.
            return Err(AppError::Conflict(
                "goal already triggered or running".to_string(),
            ));
        }
    }

    // Step 2: Resolve effective repo.
    let effective_repo = match body.repo_mode {
        RepoMode::Existing => match body.repo {
            Some(ref r) => RepoRef {
                owner: r.owner.clone(),
                name: r.name.clone(),
            },
            None => match goal.repo.clone() {
                Some(r) => r,
                None => {
                    return Err(AppError::Unprocessable(
                        "no repo specified and goal has no stored repo".to_string(),
                    ));
                }
            },
        },
        RepoMode::CreateNew => {
            // The `create` field is guaranteed present by cross-field validation.
            let spec_body = body.create.as_ref().expect("create field validated above");
            let spec = CreateRepoSpec {
                name: spec_body.name.clone(),
                private: spec_body.private,
                description: spec_body.description.clone(),
                org_login: spec_body.org_login.clone(),
            };

            // Validate the requested repo name format before calling GitHub.
            validate_goal_fields(
                "dummy",
                "dummy",
                &["dummy".to_string()],
                Some(&RepoRef {
                    // The owner will come from GitHub; validate the name at least.
                    owner: spec.org_login.clone().unwrap_or_else(|| "x".to_string()),
                    name: spec.name.clone(),
                }),
            )
            .map_err(AppError::Validation)?;

            // Idempotency: if the goal already has a repo matching the requested
            // name, skip creation.
            if let Some(ref existing_repo) = goal.repo {
                let matches = if let Some(ref org) = spec.org_login {
                    existing_repo.name == spec.name && existing_repo.owner == *org
                } else {
                    existing_repo.name == spec.name
                };
                if matches {
                    tracing::info!(
                        goal_id = %id,
                        "create_new idempotent: goal already has matching repo"
                    );
                    // Use the existing repo as the effective repo.
                    existing_repo.clone()
                } else {
                    // Goal has a different repo; proceed with creation.
                    create_new_repo(&state, &ctx, &goal, spec).await?
                }
            } else {
                create_new_repo(&state, &ctx, &goal, spec).await?
            }
        }
    };

    // Validate repo shape.
    validate_goal_fields(
        "dummy",
        "dummy",
        &["dummy".to_string()],
        Some(&effective_repo),
    )
    .map_err(AppError::Validation)?;

    // Re-validate every package_names entry exists + caller can_use_package.
    for name in &goal.package_names {
        let pkg = state.packages.get(name).await?;
        match pkg {
            Some(p) => {
                let can_use = state
                    .authz
                    .can_use_package(&ctx, name, p.owner_user_id.as_deref(), p.org_id.as_deref())
                    .await;
                if !can_use {
                    return Err(AppError::Forbidden(format!("package not usable: {name}")));
                }
            }
            None => {
                return Err(AppError::Unprocessable(format!(
                    "package not found: {name}"
                )));
            }
        }
    }

    // Step 3: Mint installation token.
    let github_app = state.github_app.as_ref().ok_or_else(|| {
        AppError::Unprocessable("github app not configured; cannot trigger goals".to_string())
    })?;
    let repo_ref_str = format!("{}/{}", effective_repo.owner, effective_repo.name);
    // The token is minted here but not yet stored (the GoalDrive will handle
    // token refresh in a later step). For now, just validate the app is
    // installed by minting a token (this serves as the installation check).
    github_app
        .token_for_repo(&repo_ref_str, None)
        .await
        .map_err(AppError::from)?;

    // Steps 4-8: Delegate to SessionService::create_for_goal.
    let trigger_info = GoalTriggerInfo {
        goal_id: goal.id,
        repo: RepoRef {
            owner: effective_repo.owner,
            name: effective_repo.name,
        },
        package_names: goal.package_names.clone(),
        owner_user_id: goal.owner_user_id.clone(),
        org_id: goal.org_id.clone(),
        prior_status: goal.status,
    };

    let result = state
        .sessions
        .create_for_goal(&state.goals, trigger_info)
        .await?;

    tracing::info!(
        goal_id = %goal.id,
        session_id = %result.session_id,
        "goal triggered"
    );

    Ok((
        StatusCode::ACCEPTED,
        Json(TriggerResponse {
            goal_id: goal.id.to_string(),
            session_id: result.session_id.to_string(),
            goal_status: result.goal_status,
            session_status: "pending",
        }),
    ))
}

/// Create a new GitHub repo via the NyxID proxy and persist it on the goal.
///
/// This function:
/// 1. Exchanges the user's token for a delegated token via NyxID.
/// 2. Proxies a "create repo" request through NyxID to GitHub.
/// 3. Persists the resulting [`RepoRef`] onto the goal document.
/// 4. Returns the [`RepoRef`] for use in the rest of the trigger flow.
async fn create_new_repo(
    state: &AppState,
    ctx: &AuthContext,
    goal: &GoalDoc,
    spec: CreateRepoSpec,
) -> Result<RepoRef, AppError> {
    // Obtain the NyxID client.
    let nyxid = state.authz.nyxid().ok_or_else(|| {
        AppError::Unavailable(
            "credential proxy not configured; cannot create repository".to_string(),
        )
    })?;

    // Exchange the user's inbound token for a delegated token.
    let delegated = nyxid.exchange_token(&ctx.raw_token).await.map_err(|e| {
        // Map NyxID errors to CreateRepoError, then AppError.
        crate::goals::CreateRepoError::from(e)
    })?;

    // Create the repository via the GitHub proxy.
    let created_repo = crate::goals::repo_create::create_repo(nyxid, &delegated, &spec).await?;

    // Persist the created repo onto the goal (CAS: only if status is triggered).
    // Note: at this point the goal is still in its pre-trigger status, so
    // set_repo may not match. That is acceptable: the repo is used as the
    // effective repo for the trigger regardless. The persist is best-effort
    // to support idempotent retries.
    let persisted = state.goals.set_repo(goal.id, &created_repo).await?;
    if persisted {
        tracing::info!(
            goal_id = %goal.id,
            repo_owner = %created_repo.owner,
            repo_name = %created_repo.name,
            "created repo persisted onto goal"
        );
    } else {
        tracing::debug!(
            goal_id = %goal.id,
            "set_repo did not match (goal may have progressed); using repo for trigger anyway"
        );
    }

    Ok(created_repo)
}

// ---- Router ---------------------------------------------------------------

/// Goal routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/goals", get(list).post(create))
        .route("/goals/:id", get(get_one).patch(update).delete(delete_one))
        .route("/goals/:id/trigger", post(trigger))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goal_view_emits_explicit_nulls() {
        let doc = GoalDoc {
            id: bson::Uuid::new(),
            title: "test".to_string(),
            description: "desc".to_string(),
            package_names: vec!["p".to_string()],
            repo: None,
            status: GoalStatus::NotStarted,
            owner_user_id: "u".to_string(),
            org_id: None,
            active_session_id: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        };
        let view = GoalView::try_from(doc).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        assert!(body["repo"].is_null(), "repo must be explicit null");
        assert!(body["org_id"].is_null(), "org_id must be explicit null");
        assert!(
            body["active_session_id"].is_null(),
            "active_session_id must be explicit null"
        );
    }

    #[test]
    fn goal_view_status_is_snake_case() {
        let doc = GoalDoc {
            id: bson::Uuid::new(),
            title: "test".to_string(),
            description: "desc".to_string(),
            package_names: vec!["p".to_string()],
            repo: None,
            status: GoalStatus::NotStarted,
            owner_user_id: "u".to_string(),
            org_id: None,
            active_session_id: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
        };
        let view = GoalView::try_from(doc).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        assert_eq!(body["status"], "not_started");
    }

    // ---- Trigger DTO tests ----

    #[test]
    fn trigger_request_accepts_empty_body() {
        let req: TriggerRequest = serde_json::from_str("{}").expect("empty body");
        assert!(req.repo.is_none());
        assert_eq!(req.repo_mode, RepoMode::Existing);
        assert!(req.create.is_none());
    }

    #[test]
    fn trigger_request_accepts_repo() {
        let req: TriggerRequest =
            serde_json::from_str(r#"{"repo":{"owner":"acme","name":"site"}}"#).expect("with repo");
        let repo = req.repo.expect("repo");
        assert_eq!(repo.owner, "acme");
        assert_eq!(repo.name, "site");
        assert_eq!(req.repo_mode, RepoMode::Existing);
    }

    #[test]
    fn trigger_request_accepts_create_new_mode() {
        let req: TriggerRequest =
            serde_json::from_str(r#"{"repo_mode":"create_new","create":{"name":"my-repo"}}"#)
                .expect("create_new");
        assert_eq!(req.repo_mode, RepoMode::CreateNew);
        assert!(req.repo.is_none());
        let create = req.create.expect("create present");
        assert_eq!(create.name, "my-repo");
        assert!(create.private); // defaults to true
        assert!(create.description.is_none());
        assert!(create.org_login.is_none());
    }

    #[test]
    fn trigger_request_create_new_with_all_fields() {
        let req: TriggerRequest = serde_json::from_str(
            r#"{"repo_mode":"create_new","create":{"name":"my-repo","private":false,"description":"A test repo","org_login":"acme"}}"#,
        )
        .expect("create_new full");
        let create = req.create.expect("create present");
        assert_eq!(create.name, "my-repo");
        assert!(!create.private);
        assert_eq!(create.description.as_deref(), Some("A test repo"));
        assert_eq!(create.org_login.as_deref(), Some("acme"));
    }

    #[test]
    fn trigger_request_explicit_existing_mode() {
        let req: TriggerRequest =
            serde_json::from_str(r#"{"repo_mode":"existing"}"#).expect("existing");
        assert_eq!(req.repo_mode, RepoMode::Existing);
    }

    #[test]
    fn trigger_request_rejects_unknown_fields() {
        let result = serde_json::from_str::<TriggerRequest>(r#"{"bogus":1}"#);
        assert!(result.is_err(), "unknown fields must be rejected");
    }

    #[test]
    fn trigger_request_rejects_unknown_fields_in_create() {
        let result = serde_json::from_str::<TriggerRequest>(
            r#"{"repo_mode":"create_new","create":{"name":"x","bogus":1}}"#,
        );
        assert!(result.is_err(), "unknown fields in create must be rejected");
    }

    #[test]
    fn trigger_response_serializes_to_documented_shape() {
        let goal_id = bson::Uuid::new();
        let session_id = bson::Uuid::new();
        let resp = TriggerResponse {
            goal_id: goal_id.to_string(),
            session_id: session_id.to_string(),
            goal_status: GoalStatus::Triggered,
            session_status: "pending",
        };
        let body = serde_json::to_value(&resp).unwrap();
        assert_eq!(body["goal_id"], goal_id.to_string());
        assert_eq!(body["session_id"], session_id.to_string());
        assert_eq!(body["goal_status"], "triggered");
        assert_eq!(body["session_status"], "pending");
    }

    // ---- RepoMode serde tests ----

    #[test]
    fn repo_mode_default_is_existing() {
        assert_eq!(default_repo_mode(), RepoMode::Existing);
    }

    #[test]
    fn repo_mode_deserializes_snake_case() {
        let mode: RepoMode =
            serde_json::from_value(serde_json::json!("create_new")).expect("deserialize");
        assert_eq!(mode, RepoMode::CreateNew);
        let mode: RepoMode =
            serde_json::from_value(serde_json::json!("existing")).expect("deserialize");
        assert_eq!(mode, RepoMode::Existing);
    }

    // ---- CreateRepoSpecBody tests ----

    #[test]
    fn create_repo_spec_body_minimal() {
        let body: CreateRepoSpecBody =
            serde_json::from_str(r#"{"name":"my-repo"}"#).expect("minimal");
        assert_eq!(body.name, "my-repo");
        assert!(body.private);
        assert!(body.description.is_none());
        assert!(body.org_login.is_none());
    }

    #[test]
    fn create_repo_spec_body_rejects_unknown_fields() {
        let result = serde_json::from_str::<CreateRepoSpecBody>(r#"{"name":"x","extra":true}"#);
        assert!(result.is_err());
    }
}
