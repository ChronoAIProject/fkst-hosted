//! Session HTTP API: `GET /api/v1/sessions/{id}` and
//! `POST /api/v1/sessions/{id}/stop`.
//!
//! Pure web edge: wire DTOs, UUID parsing, and status mapping. All
//! orchestration (driver task, engine lifecycle, CAS transitions) lives in
//! [`crate::sessions::SessionService`].
//!
//! Sessions are created exclusively via a goal trigger
//! (`POST /api/v1/goals/{id}/trigger`) since packages became repo-scoped
//! (#115): a session loads its packages from its goal repo's
//! `.fkst/packages/`, so there is no longer a repo-less "classic" session to
//! create directly. These read/stop endpoints serve goal sessions.

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthContext;
use crate::authz::permissions::{self, require_permission};
use crate::authz::{Action, Ownership};
use crate::error::{AppError, ErrorEnvelope};
use crate::models::{SessionDoc, SessionStatus, TerminalCause};
use crate::routes::{rfc3339, RepoRefView};
use crate::state::AppState;

/// Response body for `POST /api/v1/sessions/{id}/stop` (202). Always
/// `{"status":"stopping"}`: the 202 acknowledges the request; clients poll
/// `GET` for the true current state.
#[derive(Debug, Serialize, ToSchema)]
pub struct StopResponse {
    pub status: SessionStatus,
}

/// Response body for `GET /api/v1/sessions/{id}` (200): the full document
/// projection. Unset fields serialize as explicit `null`; timestamps are
/// RFC3339 UTC strings with a trailing `Z`.
#[derive(Debug, Serialize, ToSchema)]
pub struct SessionView {
    pub id: String,
    pub package_name: String,
    pub status: SessionStatus,
    pub pod_id: Option<String>,
    pub fencing_token: Option<i64>,
    pub pid: Option<i32>,
    pub runtime_dir: Option<String>,
    pub error: Option<String>,
    /// Why the session reached its terminal state (#180): `terminated`
    /// (user-stop), `completed` (graceful engine finish), or `failed` (error).
    /// Omitted while the session is still live.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminal_cause: Option<TerminalCause>,
    /// Owner user ID (explicit null for legacy sessions).
    pub owner_user_id: Option<String>,
    /// Organization ID (explicit null for personal sessions).
    pub org_id: Option<String>,
    /// Goal this session was spawned from (explicit null for classic sessions).
    pub goal_id: Option<String>,
    /// Target GitHub repo (explicit null for classic sessions).
    pub repo: Option<RepoRefView>,
    /// Event that triggered this session (explicit null for classic sessions).
    pub triggered_by: Option<String>,
    /// All package names for this session (always present, >=1 entry).
    pub package_names: Vec<String>,
    pub created_at: String,
    pub started_at: Option<String>,
    pub stopped_at: Option<String>,
}

impl TryFrom<&SessionDoc> for SessionView {
    type Error = AppError;

    fn try_from(doc: &SessionDoc) -> Result<Self, Self::Error> {
        Ok(SessionView {
            id: doc.id.to_string(),
            package_name: doc.package_name.clone(),
            status: doc.status,
            pod_id: doc.pod_id.clone(),
            fencing_token: doc.fencing_token,
            pid: doc.pid,
            runtime_dir: doc.runtime_dir.clone(),
            error: doc.error.clone(),
            terminal_cause: doc.terminal_cause,
            owner_user_id: doc.owner_user_id.clone(),
            org_id: doc.org_id.clone(),
            goal_id: doc.goal_id.map(|id| id.to_string()),
            repo: doc.repo.as_ref().map(|r| RepoRefView {
                owner: r.owner.clone(),
                name: r.name.clone(),
            }),
            triggered_by: doc.triggered_by.clone(),
            package_names: doc.effective_package_names(),
            created_at: rfc3339(doc.created_at)?,
            started_at: doc.started_at.map(rfc3339).transpose()?,
            stopped_at: doc.stopped_at.map(rfc3339).transpose()?,
        })
    }
}

/// Parse a path id into a `bson::Uuid` at the edge. A malformed id is a
/// `400`, never a `404`; a valid-but-uppercase UUID canonicalizes for free
/// (the stored `_id` is BSON Binary, compared by bytes, not by string case).
fn parse_session_id(raw: &str) -> Result<bson::Uuid, AppError> {
    bson::Uuid::parse_str(raw).map_err(|_| {
        tracing::warn!(id_bytes = raw.len(), "malformed session id rejected");
        AppError::Validation("invalid session id: must be a UUID".to_string())
    })
}

/// `GET /api/v1/sessions/{id}`: full status projection or `404`.
///
/// Authorization for goal sessions:
/// - Owner can read
/// - `triggered_by` user can read (resolved via goal ownership)
/// - Org members: any member reads
#[utoipa::path(
    get,
    path = "/sessions/{id}",
    tag = "sessions",
    operation_id = "get_session",
    security(("NyxIdIdentity" = [])),
    params(
        ("id" = String, Path, description = "Session UUID (case-insensitive)")
    ),
    responses(
        (status = 200, description = "Session status projection", body = SessionView),
        (status = 400, description = "Malformed session id", body = ErrorEnvelope),
        (status = 401, description = "Missing proxy-injected identity", body = ErrorEnvelope),
        (status = 403, description = "Caller may not read this session", body = ErrorEnvelope),
        (status = 404, description = "Session not found", body = ErrorEnvelope)
    )
)]
async fn get_one(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<Json<SessionView>, AppError> {
    // Action layer: may the caller read sessions at all? Object layer (owner /
    // org) is then enforced below per the specific session.
    require_permission(&ctx, permissions::SESSION_READ)?;
    let id = parse_session_id(&id)?;
    match state.sessions.get(id).await? {
        Some(session) => {
            authorize_session_read(&state, &ctx, &session, &id.to_string()).await?;
            tracing::debug!(session_id = %id, status = ?session.status, "session fetched");
            Ok(Json(SessionView::try_from(&session)?))
        }
        None => Err(AppError::NotFound(format!("session not found: {id}"))),
    }
}

/// `POST /api/v1/sessions/{id}/stop`: request a stop. `202` for both the
/// real transition and the idempotent no-op; `404` for an unknown id.
///
/// Authorization for goal sessions:
/// - Owner can stop
/// - Org members with member+ role can stop
#[utoipa::path(
    post,
    path = "/sessions/{id}/stop",
    tag = "sessions",
    operation_id = "stop_session",
    security(("NyxIdIdentity" = [])),
    params(
        ("id" = String, Path, description = "Session UUID (case-insensitive)")
    ),
    responses(
        (status = 202, description = "Stop requested (idempotent); poll GET for the true state", body = StopResponse),
        (status = 400, description = "Malformed session id", body = ErrorEnvelope),
        (status = 401, description = "Missing proxy-injected identity", body = ErrorEnvelope),
        (status = 403, description = "Caller may not stop this session", body = ErrorEnvelope),
        (status = 404, description = "Session not found", body = ErrorEnvelope)
    )
)]
async fn stop(
    State(state): State<AppState>,
    ctx: AuthContext,
    Path(id): Path<String>,
) -> Result<(StatusCode, Json<StopResponse>), AppError> {
    // Action layer: may the caller stop sessions at all? Object layer (owner /
    // org writer) is then enforced below per the specific session.
    require_permission(&ctx, permissions::SESSION_STOP)?;
    let id = parse_session_id(&id)?;
    // Fetch the session for authorization.
    let session = state.sessions.get(id).await?;
    match session {
        Some(session) => {
            authorize_session_write(&state, &ctx, &session, &id.to_string()).await?;
            state.sessions.request_stop(id).await?;
            Ok((
                StatusCode::ACCEPTED,
                Json(StopResponse {
                    status: SessionStatus::Stopping,
                }),
            ))
        }
        None => Err(AppError::NotFound(format!("session not found: {id}"))),
    }
}

/// Authorize a read on a session. For classic sessions this is the standard
/// ownership check. For goal sessions, the policy is:
/// - Owner can read
/// - Any org member can read
/// - The goal owner (triggered_by) can read
fn authorize_session_read<'a>(
    state: &'a AppState,
    ctx: &'a AuthContext,
    session: &'a SessionDoc,
    id_str: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + 'a>> {
    Box::pin(async move {
        let ownership = Ownership {
            owner_user_id: session.owner_user_id.as_deref(),
            org_id: session.org_id.as_deref(),
        };
        state
            .authz
            .authorize(ctx, ownership, Action::Read, "session", id_str)
            .await
    })
}

/// Authorize a write (stop) on a session. For classic sessions this is the
/// standard ownership check. For goal sessions, the policy is:
/// - Owner can stop
/// - Org members with member+ role can stop
fn authorize_session_write<'a>(
    state: &'a AppState,
    ctx: &'a AuthContext,
    session: &'a SessionDoc,
    id_str: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), AppError>> + Send + 'a>> {
    Box::pin(async move {
        let ownership = Ownership {
            owner_user_id: session.owner_user_id.as_deref(),
            org_id: session.org_id.as_deref(),
        };
        state
            .authz
            .authorize(ctx, ownership, Action::Write, "session", id_str)
            .await
    })
}

/// Session routes, to be nested under `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(get_one))
        .routes(routes!(stop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_response_serializes_to_the_documented_shape() {
        let body = serde_json::to_value(StopResponse {
            status: SessionStatus::Stopping,
        })
        .unwrap();
        assert_eq!(body, serde_json::json!({ "status": "stopping" }));
    }

    #[test]
    fn session_view_emits_explicit_nulls_and_z_suffixed_timestamps() {
        let doc = SessionDoc {
            id: bson::Uuid::new(),
            package_name: "demo".to_string(),
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            run_key: None,
            owner_user_id: None,
            org_id: None,
            package_names: vec![],
            goal_id: None,
            repo: None,
            env_scope: None,
            triggered_by: None,
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            ornn_skills: None,
            terminal_cause: None,
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            started_at: None,
            stopped_at: None,
        };
        let view = SessionView::try_from(&doc).expect("view");
        let body = serde_json::to_value(&view).unwrap();
        for field in [
            "pod_id",
            "fencing_token",
            "pid",
            "runtime_dir",
            "error",
            "owner_user_id",
            "org_id",
            "goal_id",
            "repo",
            "triggered_by",
            "started_at",
            "stopped_at",
        ] {
            assert!(body[field].is_null(), "{field} must be an explicit null");
        }
        let created_at = body["created_at"].as_str().unwrap();
        assert!(created_at.ends_with('Z'), "got {created_at}");
        assert_eq!(body["id"], doc.id.to_string());
        assert_eq!(body["status"], "pending");
        // package_names always present, at least one entry.
        let names = body["package_names"]
            .as_array()
            .expect("package_names array");
        assert_eq!(names.len(), 1, "falls back to [package_name]");
        assert_eq!(names[0], "demo");
    }

    #[test]
    fn parse_session_id_rejects_malformed_and_accepts_uppercase() {
        for bad in ["", "not-a-uuid", "f4e2c0a1-9b3d-4d2e-8c11"] {
            assert!(parse_session_id(bad).is_err(), "must reject {bad:?}");
        }
        let lower = "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12";
        let upper = lower.to_uppercase();
        let parsed_lower = parse_session_id(lower).expect("lowercase parses");
        let parsed_upper = parse_session_id(&upper).expect("uppercase parses");
        assert_eq!(
            parsed_lower, parsed_upper,
            "case must not change the identity"
        );
    }
}
