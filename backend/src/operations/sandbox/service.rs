//! The normative inventory pipeline.
//!
//! The ORDER below is the contract, not an implementation detail. Every step
//! after authorization derives from rows the caller may already see, which is
//! what makes "a hidden runtime cannot change my response, my order, my count, my
//! warnings, or my status" true rather than merely intended:
//!
//! ```text
//! 1. ONE SessionBackend::list_runtime_inventory(), under a bounded budget
//! 2. incomplete source read?        -> 503 sandbox_inventory_too_large
//! 3. authorize EVERY row            -> drop the hidden ones
//! 4. apply the user filters         -> to the authorized survivors only
//! 5. project warnings               -> onto the surviving rows only
//! 6. sort                           -> a total order over each row's own keys
//! 7. item_count + result ceiling    -> counted from the surviving rows only
//! ```
//!
//! Steps 1–2 are the only place the complete fleet exists. It lives in a local
//! variable inside this trusted process: it is never logged, never cached, never
//! counted for a caller, and never serialized.
//!
//! ## The failure taxonomy, and why it has four codes
//!
//! - **disabled** — this deployment has no runtime backend. Permanent until an
//!   operator changes the configuration.
//! - **unavailable** — the backend failed or exceeded the budget. Retryable, and
//!   deliberately carries no upstream status, message, or URL.
//! - **too_large** — the answer could not be COMPLETE within a configured
//!   ceiling. Not a failure of the backend, and never a truncated list.
//! - **session_visibility_unavailable** — the authorization projection cannot
//!   answer. Blocks `accessible` only; a global administrator's `all` request is
//!   unaffected, because it needs no session context.
//!
//! Only the first three arise here; the fourth is decided by the route's
//! readiness gate and by [`super::authorize`].

use std::time::Duration;

use k8s_openapi::chrono::{DateTime, Utc};

use crate::access_policy::AccessPolicy;
use crate::error::AppError;
use crate::runtime_identity::RuntimeBackendKind;
use crate::session_access::{AuthenticatedViewer, SessionAccessRegistry, ViewerScope};
use crate::session_backend::inventory::{
    InventoryWarningCode, RuntimeInventoryItem, RuntimeInventorySnapshot, RuntimeLifetimePolicy,
};
use crate::session_backend::{BackendError, SessionBackend};

use super::authorize::RowAuthorizer;
use super::filters::SandboxFilters;
use super::order;
use super::warning::{self, SandboxWarningCode};

/// Everything one authorized inventory read needs, borrowed from the request.
pub struct SandboxInventoryRequest<'a> {
    pub viewer: &'a AuthenticatedViewer,
    /// The SERVER-resolved scope. Cannot be minted from request input.
    pub scope: &'a ViewerScope,
    pub access: &'a AccessPolicy,
    /// `FKST_LOG_ADMINS` — the legacy cross-session observability grant.
    pub legacy_log_admins: &'a [String],
    pub registry: &'a SessionAccessRegistry,
    pub filters: &'a SandboxFilters,
    /// The lifetime/idle policy every row is rendered against, plus the
    /// DEFENSIVE source ceiling (issue #5674).
    pub lifetime: RuntimeLifetimePolicy,
    /// The PUBLIC result ceiling, applied after authorization and filters.
    pub max_result_items: usize,
    /// The bounded budget for the one backend list.
    pub timeout: Duration,
}

/// One runtime the caller is authorized to see, with its public warnings.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorizedRuntime {
    pub item: RuntimeInventoryItem,
    pub warning_codes: Vec<SandboxWarningCode>,
}

/// The authorized, filtered, ordered result of one inventory read.
#[derive(Clone, Debug, PartialEq)]
pub struct AuthorizedInventory {
    /// The backend snapshot's own instant, verbatim. Never `now`, and never a
    /// cached value from an earlier read.
    pub observed_at: DateTime<Utc>,
    pub backend: RuntimeBackendKind,
    pub items: Vec<AuthorizedRuntime>,
    /// Response-scope codes. Derived from the returned rows, plus snapshot-scope
    /// codes for a global administrator.
    pub warning_codes: Vec<SandboxWarningCode>,
}

/// Run the pipeline against the configured runtime backend.
pub async fn run(
    backend: &dyn SessionBackend,
    request: &SandboxInventoryRequest<'_>,
) -> Result<AuthorizedInventory, AppError> {
    let snapshot = read_snapshot(backend, request).await?;
    reject_incomplete_source(&snapshot)?;

    // Step 3: authorization, on EVERY listed row, before anything else looks at
    // the list. `retain` cannot be used here because a projection failure must
    // abort the whole request rather than silently drop rows.
    let authorizer = RowAuthorizer::new(
        request.registry,
        request.viewer,
        request.scope,
        request.access,
        request.legacy_log_admins,
    );
    let mut visible: Vec<RuntimeInventoryItem> = Vec::new();
    for item in snapshot.items {
        if authorizer.decide_row(&item)?.is_none() {
            visible.push(item);
        }
    }

    // Step 4: the caller's own filters, on the authorized survivors only.
    visible.retain(|item| request.filters.matches(item));

    // Step 5: warnings, attached only to rows that are being returned.
    let mut items: Vec<AuthorizedRuntime> = visible
        .into_iter()
        .map(|item| {
            let warning_codes = warning::normalize(item_warnings(&snapshot.warnings, &item));
            AuthorizedRuntime {
                item,
                warning_codes,
            }
        })
        .collect();

    // Step 6: the documented total order.
    items.sort_by(|left, right| order::compare(&left.item, &right.item));

    // Step 7: the PUBLIC ceiling, counted from the returned rows only — a caller
    // is never failed because of fleet rows they cannot see.
    if items.len() > request.max_result_items {
        tracing::warn!(
            limit = request.max_result_items,
            scope = request.scope.as_str(),
            "operations: authorized sandbox inventory exceeds the configured result ceiling; \
             refusing to return a partial snapshot"
        );
        return Err(too_large());
    }

    let warning_codes = response_warnings(&items, &snapshot.warnings, request.scope);
    Ok(AuthorizedInventory {
        observed_at: snapshot.observed_at,
        backend: snapshot.backend,
        items,
        warning_codes,
    })
}

/// Step 1: the one backend list, under the route's bounded budget.
async fn read_snapshot(
    backend: &dyn SessionBackend,
    request: &SandboxInventoryRequest<'_>,
) -> Result<RuntimeInventorySnapshot, AppError> {
    let read = backend.list_runtime_inventory(&request.lifetime);
    match tokio::time::timeout(request.timeout, read).await {
        Ok(Ok(snapshot)) => Ok(snapshot),
        Ok(Err(BackendError::InventoryTooLarge { limit })) => {
            // The ceiling is an operator-configured constant, so logging it
            // reveals nothing about the fleet; the RESPONSE still carries no
            // number at all.
            tracing::error!(
                limit,
                "operations: runtime inventory exceeds the configured source ceiling"
            );
            Err(too_large())
        }
        Ok(Err(error)) => {
            // The backend's own text may name a namespace, a URL, or an
            // apiserver message. It is logged and never returned.
            tracing::error!(error = %error, "operations: runtime inventory read failed");
            Err(unavailable())
        }
        Err(_elapsed) => {
            tracing::error!(
                timeout_ms = request.timeout.as_millis(),
                "operations: runtime inventory read exceeded its bounded budget"
            );
            Err(unavailable())
        }
    }
}

/// Step 2: a clipped page walk means the fleet read was incomplete, so no
/// authorized answer derived from it can claim to be the complete matching set.
fn reject_incomplete_source(snapshot: &RuntimeInventorySnapshot) -> Result<(), AppError> {
    let truncated = snapshot
        .warnings
        .iter()
        .any(|warning| warning.code == InventoryWarningCode::SourceTruncated);
    if !truncated {
        return Ok(());
    }
    tracing::error!(
        backend = snapshot.backend.as_str(),
        "operations: runtime inventory page walk was clipped; refusing to serve an incomplete fleet"
    );
    Err(too_large())
}

/// The public codes of the warnings naming exactly this runtime.
fn item_warnings(
    warnings: &[crate::session_backend::inventory::BoundedInventoryWarning],
    item: &RuntimeInventoryItem,
) -> Vec<SandboxWarningCode> {
    warnings
        .iter()
        .filter(|warning| warning.runtime_id.as_deref() == Some(item.runtime_id.as_str()))
        .filter_map(|warning| warning::public_code(warning.code))
        .collect()
}

/// The response-scope codes.
///
/// For every caller these are the codes already attached to the rows being
/// returned — so the field summarizes the page rather than the fleet. A verified
/// global administrator ADDITIONALLY receives the snapshot-scope codes (warnings
/// naming no runtime), which are deployment health rather than row data and would
/// otherwise let a hidden runtime alter a regular caller's response.
fn response_warnings(
    items: &[AuthorizedRuntime],
    warnings: &[crate::session_backend::inventory::BoundedInventoryWarning],
    scope: &ViewerScope,
) -> Vec<SandboxWarningCode> {
    let mut codes: Vec<SandboxWarningCode> = items
        .iter()
        .flat_map(|item| item.warning_codes.iter().copied())
        .collect();
    if scope.is_global() {
        codes.extend(
            warnings
                .iter()
                .filter(|warning| warning.runtime_id.is_none())
                .filter_map(|warning| warning::public_code(warning.code)),
        );
    }
    warning::normalize(codes)
}

/// The stable, count-free capacity failure.
fn too_large() -> AppError {
    AppError::SandboxInventoryTooLarge(
        "the live sandbox inventory is too large to answer completely".to_string(),
    )
}

/// The stable, detail-free backend failure.
fn unavailable() -> AppError {
    AppError::SandboxInventoryUnavailable(
        "the runtime backend could not be read; retry shortly".to_string(),
    )
}

#[cfg(test)]
#[path = "service_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "service_capacity_tests.rs"]
mod capacity_tests;
