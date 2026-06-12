//! Goals HTTP API: CRUD for `/api/v1/goals`.
//!
//! Endpoints:
//! - `POST   /api/v1/goals`         — create a goal (201)
//! - `GET    /api/v1/goals`         — list goals (200, paginated)
//! - `GET    /api/v1/goals/{id}`    — fetch one goal (200)
//! - `PATCH  /api/v1/goals/{id}`    — partial update (200)
//! - `DELETE /api/v1/goals/{id}`    — delete (204)
//!
//! This is purely the web edge: wire DTOs, UUID parsing, authz checks, and
//! status mapping. Validation logic lives in the goals domain module.

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderName, HeaderValue, StatusCode};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};

use crate::auth::AuthContext;
use crate::authz::{Action, Ownership};
use crate::error::AppError;
use crate::goals::{
    validate_goal_fields, GoalDoc, GoalStatus, RepoRef, MAX_GOAL_DESCRIPTION_BYTES,
    MAX_GOAL_TITLE_CHARS,
};
use crate::routes::extract::AppJson;
use crate::routes::rfc3339;
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

/// `GET /api/v1/goals/{id}`: fetch one goal.
async fn get_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<GoalView>, AppError> {
    let uuid = parse_goal_uuid(&id)?;
    let goal = state
        .goals
        .get(uuid)
        .await?
        .ok_or_else(|| AppError::NotFound(format!("goal not found: {id}")))?;

    let ownership = goal_ownership(&goal);
    state
        .authz
        .authorize(&ctx, ownership, Action::Read, "goal", &id)
        .await?;

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

// ---- Router ---------------------------------------------------------------

/// Goal routes, nested under `/api/v1`.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/goals", get(list).post(create))
        .route("/goals/:id", get(get_one).patch(update).delete(delete_one))
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
}
