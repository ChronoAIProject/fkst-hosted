//! `GET /api/v1/operations/sandboxes` — the row-authorized live inventory.
//!
//! The handler is a sequence of gates, in this exact NORMATIVE order:
//!
//! ```text
//! 1. AuthenticatedViewer         401 missing/invalid identity, 403 not admitted
//! 2. normalize the filters       400  (pure; no registry, no backend)
//! 3. resolve the scope           403 operations_scope_forbidden
//! 4. record the safe arguments   (once, on BOTH the allowed and refused paths)
//! 5. preauthorize an exact id    404 sandbox_not_found  (accessible only)
//! 6. require registry readiness  503 session_visibility_unavailable (accessible)
//! 7. require a runtime backend   503 sandbox_inventory_disabled
//! 8. ONE inventory read          200 / 503
//! ```
//!
//! Steps 2–7 all complete before a single backend call, so a refused request
//! costs the deployment nothing and a caller learns nothing from timing.
//!
//! ## Why step 6 is not "return an empty list"
//!
//! A cold or incomplete session-visibility projection cannot distinguish "you
//! have no sandboxes" from "I do not yet know which sandboxes are yours". An
//! empty `200` would assert the first while only the second is true, and an
//! operator staring at an empty operations page during a restart is exactly the
//! incident this `503` exists to prevent. It gates `accessible` ONLY: a global
//! administrator's `all` request needs no session context, so a registry outage
//! must not take the complete fleet view down with it.
//!
//! ## Why step 7 comes after step 6
//!
//! The normative order puts registry readiness before the inventory read, and
//! "there is no backend to read" is a property of that read. Deciding it earlier
//! would let a regular caller on a cold registry learn the deployment's backend
//! configuration from the error code they get back.
//!
//! ## What this handler never does
//!
//! It never calls PostHog, the audit relay, GitHub, `list_fleet()`, a per-runtime
//! `status_summary()`, Pod logs, or exec. It never mutates a runtime, never
//! refreshes a last-pending marker, and never answers a process cache as a fresh
//! `200` after a backend failure — there is no cache to answer from.

use std::time::Instant;

use axum::extract::State;
use axum::http::{header, Extensions};
use axum::Json;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::arguments::operations::{SafeOperationsListSandboxes, SandboxScope};
use crate::audit::arguments::{record_safe, AuditedQuery};
use crate::error::{AppError, ErrorEnvelope};
use crate::operations::sandbox::{
    self, BackendLabel, InventoryResult, SandboxInventoryRequest, SandboxRejectionReason,
    ScopeLabel,
};
use crate::runtime_identity::RuntimeBackendKind;
use crate::session_access::{
    authorize_session_visibility, resolve_operations_scope, AuthenticatedViewer, RequestedScope,
    ScopeRequest, ViewerScope,
};
use crate::session_backend::inventory::RuntimeLifetimePolicy;
use crate::state::AppState;

use super::sandbox_dto::{
    filters_view, response_from_inventory, SandboxEffectiveScope, SandboxInventoryResponse,
};
use super::sandbox_query::{normalize, NormalizedSandboxRequest, SandboxQueryParams};

/// Fixed client text for an unresolvable session. Identical whether the id is
/// unknown or merely not this caller's — see [`preauthorize_session`].
const SESSION_NOT_FOUND: &str = "no such session";

/// The `Cache-Control` every snapshot carries.
///
/// A live inventory is true for exactly one instant. Any shared or browser cache
/// would hand a later reader a fleet that no longer exists, and — worse — could
/// hand it to a DIFFERENT reader whose authorization differs. The frontend owns
/// last-good display state; the API only ever states what it just observed.
const NO_STORE: &str = "no-store";

/// Serve one authorized live sandbox snapshot.
#[utoipa::path(
    get,
    path = "/operations/sandboxes",
    tag = "operations",
    operation_id = "operations_list_sandboxes",
    params(SandboxQueryParams),
    responses(
        (status = 200, description = "The complete live snapshot the caller is authorized to see", body = SandboxInventoryResponse),
        (status = 400, description = "A filter value outside its closed vocabulary", body = ErrorEnvelope),
        (status = 401, description = "Missing or invalid GitHub identity", body = ErrorEnvelope),
        (status = 403, description = "A regular caller selected the global scope (`operations_scope_forbidden`), or is not admitted by the deployment access policy", body = ErrorEnvelope),
        (status = 404, description = "The exact `session_id` is unknown or not visible to this caller (`sandbox_not_found`); the two are indistinguishable by design", body = ErrorEnvelope),
        (status = 503, description = "The session-visibility projection is cold (`session_visibility_unavailable`), no runtime backend is configured (`sandbox_inventory_disabled`), the backend failed or timed out (`sandbox_inventory_unavailable`), or the inventory cannot be answered completely within a configured ceiling (`sandbox_inventory_too_large`)", body = ErrorEnvelope),
    )
)]
async fn operations_list_sandboxes(
    State(state): State<AppState>,
    extensions: Extensions,
    // Identity is extracted FIRST so the documented gate order is the order axum
    // actually runs: extractors execute in declaration order, so a query-parse
    // rejection declared ahead of the viewer would answer `400` to a request that
    // carries no identity at all.
    viewer: AuthenticatedViewer,
    AuditedQuery(params): AuditedQuery<SandboxQueryParams>,
) -> Result<
    (
        [(header::HeaderName, &'static str); 1],
        Json<SandboxInventoryResponse>,
    ),
    AppError,
> {
    let started = Instant::now();
    // Known without any call, so even a refused request is counted under the
    // backend it would have been served by.
    let backend_kind = state
        .session_backend
        .as_ref()
        .map(|backend| backend.backend_kind());
    let telemetry = Telemetry {
        metrics: state.operations.sandbox_metrics.clone(),
        backend: BackendLabel::of(backend_kind),
        started,
    };

    let request = normalize(&params).inspect_err(|_| {
        telemetry.reject(
            natural_scope(&viewer),
            InventoryResult::InvalidRequest,
            SandboxRejectionReason::InvalidFilter,
        );
    })?;

    let resolved = resolve_operations_scope(
        &viewer,
        ScopeRequest::new(request.requested_scope),
        &state.session_access.scope_metrics,
    );
    // Recorded on BOTH paths, exactly once: a refused probe is as much a fact
    // worth auditing as an allowed read.
    record_safe(&extensions, &safe_arguments(&request, &resolved, &viewer));
    let scope = resolved.inspect_err(|_| {
        telemetry.reject(
            natural_scope(&viewer),
            InventoryResult::Forbidden,
            SandboxRejectionReason::GlobalScope,
        );
    })?;
    let scope_label = scope_label(&scope);

    preauthorize_session(&state, &viewer, &scope, &request, &telemetry)?;
    require_visibility(&state, &scope, &telemetry)?;

    let Some(backend) = state.session_backend.as_ref() else {
        tracing::info!(
            "operations: sandbox inventory requested with no runtime backend configured"
        );
        telemetry.finish(scope_label, InventoryResult::Disabled);
        return Err(AppError::SandboxInventoryDisabled(
            "this deployment has no runtime backend to inventory".to_string(),
        ));
    };

    let inventory = sandbox::run(
        backend.as_ref(),
        &SandboxInventoryRequest {
            viewer: &viewer,
            scope: &scope,
            access: &state.config.access,
            legacy_log_admins: &state.config.log.admins,
            registry: &state.session_access.registry,
            filters: &request.filters,
            lifetime: RuntimeLifetimePolicy::from_reconcile_config(&state.config.reconcile),
            max_result_items: state.config.sandbox.max_result_items,
            timeout: state.config.sandbox.timeout(),
        },
    )
    .await
    .inspect_err(|error| telemetry.finish(scope_label, result_of(error)))?;

    telemetry.items(scope_label, inventory.items.len() as u64);
    telemetry.finish(scope_label, InventoryResult::Success);
    Ok((
        [(header::CACHE_CONTROL, NO_STORE)],
        Json(response_from_inventory(
            &inventory,
            effective_scope(&scope),
            viewer.is_global_admin(),
            filters_view(&request),
        )),
    ))
}

/// Step 5: authorize an exact `session_id` BEFORE the runtime backend is called.
///
/// A missing, unknown, and unauthorized id all answer the SAME
/// `404 sandbox_not_found`, and all of them cost zero backend calls.
/// Distinguishing them — by code, by body, or by doing work in one case and not
/// the other — would turn the endpoint into a session-existence oracle
/// (epic `SBOX-06`).
///
/// A global administrator in `all` scope needs no preauthorization: they may see
/// every managed runtime including orphans with no registry context at all, so
/// requiring the filter to resolve would HIDE exactly the rows that scope exists
/// to show.
fn preauthorize_session(
    state: &AppState,
    viewer: &AuthenticatedViewer,
    scope: &ViewerScope,
    request: &NormalizedSandboxRequest,
    telemetry: &Telemetry,
) -> Result<(), AppError> {
    if scope.is_global() {
        return Ok(());
    }
    let Some(session_id) = request.session_id() else {
        return Ok(());
    };
    authorize_session_visibility(
        &state.session_access.registry,
        viewer,
        scope,
        &state.config.access,
        &state.config.log.admins,
        session_id,
    )
    .map(|_decision| ())
    .map_err(|error| match error {
        // The gate's own anti-enumeration `404` is re-coded to this endpoint's
        // stable one; a cold projection keeps its distinct `503`, because "retry
        // shortly" and "no such session" need different remedies.
        AppError::NotFound(_) => {
            telemetry.reject(
                ScopeLabel::Accessible,
                InventoryResult::NotFound,
                SandboxRejectionReason::SessionNotFound,
            );
            AppError::SandboxNotFound(SESSION_NOT_FOUND.to_string())
        }
        other => {
            telemetry.reject(
                ScopeLabel::Accessible,
                InventoryResult::VisibilityUnavailable,
                SandboxRejectionReason::VisibilityUnavailable,
            );
            other
        }
    })
}

/// Step 6: `accessible` needs a trustworthy projection; `all` does not.
fn require_visibility(
    state: &AppState,
    scope: &ViewerScope,
    telemetry: &Telemetry,
) -> Result<(), AppError> {
    if scope.is_global() || state.session_access.registry.is_ready() {
        return Ok(());
    }
    tracing::info!("operations: sandbox inventory refused while session visibility is not ready");
    telemetry.reject(
        ScopeLabel::Accessible,
        InventoryResult::VisibilityUnavailable,
        SandboxRejectionReason::VisibilityUnavailable,
    );
    Err(AppError::SessionVisibilityUnavailable(
        "session visibility is still recovering; retry shortly".to_string(),
    ))
}

/// Bounded telemetry for one request. Holds no identity: every field is a closed
/// enum or a duration.
struct Telemetry {
    metrics: sandbox::SandboxMetrics,
    backend: BackendLabel,
    started: Instant,
}

impl Telemetry {
    /// Count one terminal outcome and its duration.
    fn finish(&self, scope: ScopeLabel, result: InventoryResult) {
        let elapsed = u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
        self.metrics
            .record_request(self.backend, scope, result, elapsed);
    }

    /// Count one refusal that happened before the backend was touched, with its
    /// bounded reason.
    fn reject(&self, scope: ScopeLabel, result: InventoryResult, reason: SandboxRejectionReason) {
        self.metrics.record_rejection(reason);
        self.finish(scope, result);
    }

    /// Publish the authorized result size.
    fn items(&self, scope: ScopeLabel, items: u64) {
        self.metrics.record_items(self.backend, scope, items);
    }
}

/// The scope a caller has WITHOUT asking for anything — what a refused request is
/// recorded and counted under.
fn natural_scope(viewer: &AuthenticatedViewer) -> ScopeLabel {
    if viewer.is_global_admin() {
        ScopeLabel::All
    } else {
        ScopeLabel::Accessible
    }
}

fn scope_label(scope: &ViewerScope) -> ScopeLabel {
    if scope.is_global() {
        ScopeLabel::All
    } else {
        ScopeLabel::Accessible
    }
}

fn effective_scope(scope: &ViewerScope) -> SandboxEffectiveScope {
    if scope.is_global() {
        SandboxEffectiveScope::All
    } else {
        SandboxEffectiveScope::Accessible
    }
}

/// The bounded result an inventory failure is counted under.
fn result_of(error: &AppError) -> InventoryResult {
    match error {
        AppError::SandboxInventoryTooLarge(_) => InventoryResult::TooLarge,
        AppError::SessionVisibilityUnavailable(_) => InventoryResult::VisibilityUnavailable,
        _ => InventoryResult::Unavailable,
    }
}

/// Project the normalized request onto its reviewed safe-argument boundary.
///
/// The EFFECTIVE scope is the resolved one when the request was allowed, and the
/// caller's natural scope when it was refused — so a denial reads as "this
/// caller, whose scope is `accessible`, asked for `all`". Nothing here is a raw
/// value: every filter was validated before it became a property, and an invalid
/// one never reached this function at all.
fn safe_arguments(
    request: &NormalizedSandboxRequest,
    resolved: &Result<ViewerScope, AppError>,
    viewer: &AuthenticatedViewer,
) -> SafeOperationsListSandboxes {
    let effective = match resolved {
        Ok(scope) if scope.is_global() => SandboxScope::All,
        Ok(_) => SandboxScope::Accessible,
        Err(_) if viewer.is_global_admin() => SandboxScope::All,
        Err(_) => SandboxScope::Accessible,
    };
    let requested = request.requested_scope.map(|requested| match requested {
        RequestedScope::Global => SandboxScope::All,
        RequestedScope::Personal => SandboxScope::Accessible,
    });
    let filters = &request.filters;
    SafeOperationsListSandboxes {
        scope: effective,
        // Recorded only when it DIFFERS from the effective scope: an identical
        // pair carries no information, and a refused probe is exactly the case
        // where the two diverge.
        requested_scope: requested.filter(|requested| *requested != effective),
        session_id: filters.session_id.clone(),
        repo_full_name: filters.repo_full_name.clone(),
        trigger_issue: filters.trigger_issue,
        status: filters.status.map(|status| status.as_str().to_string()),
        backend: filters
            .backend
            .map(|backend: RuntimeBackendKind| backend.as_str().to_string()),
        creator_id: filters.creator_id,
        creator_login: filters.creator_login.clone(),
        attribution_source: filters
            .attribution_source
            .map(|source| source.as_str().to_string()),
    }
}

/// The sandbox route, merged into the operations subtree.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(operations_list_sandboxes))
}

#[cfg(test)]
#[path = "sandboxes_tests.rs"]
mod tests;
