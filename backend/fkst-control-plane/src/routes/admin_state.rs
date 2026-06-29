//! `GET /api/v1/admin/state`: the control plane's LIVE, ephemeral in-memory
//! state for operators (issue #144).
//!
//! The control plane is API-only and datastore-free: there is no DB to inspect,
//! no claim authority, and no worker registry, so the durable audit trail is the
//! GitHub Issues a goal/session writes (the goal issue + its labels). THIS
//! endpoint is the complementary live view: a point-in-time snapshot of the
//! in-memory [`SessionRepo`](crate::sessions::SessionRepo) (sessions) that no
//! datastore preserves across a restart.
//!
//! ## Redaction (mandatory, load-bearing)
//! This route NEVER serializes a secret value. The control plane holds secret
//! material in memory (per-session user tokens, inline vault secrets, minted
//! GitHub-App tokens), and none of it is reachable through the projected session
//! view used here: it carries only ids, statuses, owner/worker identifiers —
//! plus PRESENCE booleans (e.g. `owner_present`) where a value would otherwise
//! hint at sensitive data. The view is built by deliberate field selection, not
//! by serializing a domain document, so a secret can never leak by accident.
//!
//! ## Authorization
//! Gated by the platform-admin permission [`permissions::ADMIN`] (`fkst:admin`),
//! which already bypasses every other gate, so it is the single coarse
//! capability an operator needs. With `FKST_AUTH_ENABLED=false` the dev context
//! carries `fkst:admin`, so the route is open locally.

use std::collections::BTreeMap;

use axum::extract::State;
use axum::Json;
use serde::Serialize;
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::auth::AuthContext;
use crate::authz::permissions::{self, require_permission};
use crate::error::{AppError, ErrorEnvelope};
use crate::state::AppState;

/// One session row in the admin view, keyed by session id in the response map.
/// Deliberately reduced from the full `SessionDoc`: status, the goal id, an
/// owner-PRESENCE boolean (never the owner id value), and the owning worker /
/// pod. This is where redaction is most visible — the owner is reported as a
/// boolean, and NO token / secret / env value is present.
///
/// Named `AdminSessionView` (not `SessionView`) so it does not collide, in the
/// generated spec, with the public [`super::sessions::SessionView`] — a
/// different, fuller projection.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminSessionView {
    pub status: crate::models::SessionStatus,
    pub goal_id: Option<String>,
    /// Presence of an owner-user binding (the value is intentionally NOT
    /// serialized — redaction by deliberate field selection).
    pub owner_present: bool,
    /// The worker / pod that owns this session's run, when assigned.
    pub worker: Option<String>,
}

/// The full admin-state response: the live in-memory session store.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminStateView {
    /// Session id -> projected (redacted) view.
    pub sessions: BTreeMap<String, AdminSessionView>,
}

/// `GET /api/v1/admin/state`: the live session-store snapshot. Admin-gated.
#[utoipa::path(
    get,
    path = "/admin/state",
    tag = "admin",
    operation_id = "admin_state",
    security(("NyxIdIdentity" = [])),
    responses(
        (status = 200, description = "Live snapshot of the in-memory sessions", body = AdminStateView),
        (status = 401, description = "Missing proxy-injected identity", body = ErrorEnvelope),
        (status = 403, description = "Caller lacks the fkst:admin permission", body = ErrorEnvelope)
    )
)]
async fn get_state(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<AdminStateView>, AppError> {
    // Action layer: only a platform admin (`fkst:admin`) may read the live
    // state. Admin bypasses every other gate, so this single check is sufficient
    // (no object layer — the state is plane-global, not a per-user resource).
    require_permission(&ctx, permissions::ADMIN)?;

    // Sessions: the in-memory store, projected to the redacted per-session view.
    let sessions = state
        .sessions
        .repo()
        .snapshot()
        .await
        .into_iter()
        .map(|doc| {
            (
                doc.id.to_string(),
                AdminSessionView {
                    status: doc.status,
                    goal_id: doc.goal_id.map(|g| g.to_string()),
                    owner_present: doc.owner_user_id.is_some(),
                    worker: doc.pod_id,
                },
            )
        })
        .collect::<BTreeMap<_, _>>();

    tracing::debug!(sessions = sessions.len(), "admin state snapshot served");
    Ok(Json(AdminStateView { sessions }))
}

/// Admin-state route, nested under `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_state))
}
