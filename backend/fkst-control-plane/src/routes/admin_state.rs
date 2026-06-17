//! `GET /api/v1/admin/state`: the controller's LIVE, ephemeral in-memory state
//! for operators (issue #144).
//!
//! The controller is datastore-free (#143): there is no DB to inspect, so the
//! durable audit trail is the GitHub Issues a goal/session writes (the goal
//! issue + its labels + the committed journal file). THIS endpoint is the
//! complementary live view: a point-in-time snapshot of the three in-memory
//! authorities — the [`ClaimMap`](crate::controller::ClaimMap) (claims), the
//! [`WorkerRegistry`](crate::controller::WorkerRegistry) (workers), and the
//! in-memory [`SessionRepo`](crate::sessions::SessionRepo) (sessions) — that a
//! controller rebuilds from worker self-reports on restart and that no datastore
//! preserves across a controller loss.
//!
//! ## Redaction (mandatory, load-bearing)
//! This route NEVER serializes a secret value. The controller holds secret
//! material in memory (per-session user tokens, inline vault secrets, minted
//! GitHub-App tokens), and none of it is reachable through the snapshot types
//! used here: the [`ClaimEntry`](crate::controller::ClaimEntry), the
//! [`WorkerSnapshot`](crate::controller::WorkerSnapshot), and the projected
//! session view carry only ids, statuses, owner/worker identifiers, fences, and
//! liveness — plus PRESENCE booleans (e.g. `owner_present`) where a value would
//! otherwise hint at sensitive data. The view is built by deliberate field
//! selection, not by serializing a domain document, so a secret can never leak
//! by accident.
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

/// One claim row in the admin view: the lease/goal key, the bound session, the
/// owning worker, the controller-authoritative status, and the run fence. No
/// secret material — a [`ClaimEntry`](crate::controller::ClaimEntry) carries
/// none.
#[derive(Debug, Serialize, ToSchema)]
pub struct ClaimView {
    /// The lease key (`<package>` classic / `goal-<uuid>` goal).
    pub lease_key: String,
    pub session_id: String,
    pub owner_worker: String,
    /// Controller-authoritative lifecycle status (`pending`/`running`/...).
    pub status: crate::models::SessionStatus,
    /// Current run fence (journaling-idempotency id, never a credential).
    pub fencing_id: i64,
    /// The goal this claim drives, when it is a goal session.
    pub goal_id: Option<String>,
}

/// One worker row in the admin view: its id, heartbeat age, capacity, the
/// controller-authoritative current load (active claim count), and its
/// lifecycle + liveness. No secret material — a
/// [`WorkerSnapshot`](crate::controller::WorkerSnapshot) is pure liveness.
#[derive(Debug, Serialize, ToSchema)]
pub struct WorkerView {
    pub worker_id: String,
    /// Whole seconds since this worker's last heartbeat, at snapshot time.
    pub last_heartbeat_age_secs: u64,
    /// Max concurrent engine sessions the worker self-reported on registration.
    pub capacity: u32,
    /// Controller-authoritative active claim count owned by this worker (the
    /// immediate load placement uses — not the heartbeat-lagged self-report).
    pub current_load: u64,
    /// `active` / `draining` (worker-reported lifecycle).
    pub lifecycle_state: fkst_shared::protocol::LifecycleState,
    /// Whether the worker is still within the liveness TTL (not yet expired).
    pub alive: bool,
}

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

/// The full admin-state response: the three live in-memory authorities.
#[derive(Debug, Serialize, ToSchema)]
pub struct AdminStateView {
    pub claims: Vec<ClaimView>,
    pub workers: Vec<WorkerView>,
    /// Session id -> projected (redacted) view.
    pub sessions: BTreeMap<String, AdminSessionView>,
}

/// `GET /api/v1/admin/state`: the live controller state snapshot. Admin-gated.
#[utoipa::path(
    get,
    path = "/admin/state",
    tag = "admin",
    operation_id = "admin_state",
    security(("NyxIdIdentity" = [])),
    responses(
        (status = 200, description = "Live snapshot of claims, workers, and sessions", body = AdminStateView),
        (status = 401, description = "Missing proxy-injected identity", body = ErrorEnvelope),
        (status = 403, description = "Caller lacks the fkst:admin permission", body = ErrorEnvelope)
    )
)]
async fn get_state(
    State(state): State<AppState>,
    ctx: AuthContext,
) -> Result<Json<AdminStateView>, AppError> {
    // Action layer: only a platform admin (`fkst:admin`) may read the live
    // controller state. Admin bypasses every other gate, so this single check is
    // sufficient (no object layer — the state is controller-global, not a
    // per-user resource).
    require_permission(&ctx, permissions::ADMIN)?;

    // Claims: empty when no controller is wired (a minimal/test build).
    let claims = match &state.claims {
        Some(claims) => claims
            .snapshot()
            .into_iter()
            .map(|e| ClaimView {
                lease_key: e.lease_key,
                session_id: e.session_id.to_string(),
                owner_worker: e.owner_worker,
                status: e.status,
                fencing_id: e.fencing_id,
                goal_id: e.goal_id.map(|g| g.to_string()),
            })
            .collect(),
        None => Vec::new(),
    };

    // Workers: each worker's controller-authoritative load is the claim map's
    // active count for it (immediate), not the heartbeat-lagged self-report.
    let workers = match (&state.worker_registry, &state.claims) {
        (Some(registry), Some(claims)) => registry
            .snapshot()
            .await
            .into_iter()
            .map(|w| WorkerView {
                current_load: claims.active_load(&w.worker_id),
                worker_id: w.worker_id,
                last_heartbeat_age_secs: w.last_heartbeat_age_secs,
                capacity: w.capacity,
                lifecycle_state: w.lifecycle_state,
                alive: w.alive,
            })
            .collect(),
        // No claim map => load is unknown; report it as 0 (the worker is tracked
        // but the controller-authoritative load is unavailable in this build).
        (Some(registry), None) => registry
            .snapshot()
            .await
            .into_iter()
            .map(|w| WorkerView {
                current_load: 0,
                worker_id: w.worker_id,
                last_heartbeat_age_secs: w.last_heartbeat_age_secs,
                capacity: w.capacity,
                lifecycle_state: w.lifecycle_state,
                alive: w.alive,
            })
            .collect(),
        _ => Vec::new(),
    };

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
        .collect();

    tracing::debug!(
        claims = claims.len(),
        workers = workers.len(),
        "admin state snapshot served"
    );
    Ok(Json(AdminStateView {
        claims,
        workers,
        sessions,
    }))
}

/// Admin-state route, nested under `/api/v1`.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_state))
}
