//! The bounded metric FAMILIES rendered into the `/metrics` exposition body.
//!
//! Split out of [`super::metrics`] so the handler file stays about the request
//! and the recovery block, and so each family's closed-label reasoning sits
//! next to the loop that enumerates it.
//!
//! One rule governs every renderer here: a label may only ever be a closed Rust
//! enum, so the series count is finite by construction. Session ids, runtime
//! ids, repositories, creators, actors, and trigger issues are structured-log
//! fields, never labels and never values (epic `OPS-04`). Every label tuple is
//! rendered even when its counter is zero, so a quiet series is visibly quiet
//! rather than silently absent.

use crate::audit::lifecycle::LifecycleAction;
use crate::audit::AuditMetricsSnapshot;
use crate::runtime_identity::metrics::{
    IdentityOperationResult, LifecycleEmitResult, RuntimeTelemetrySnapshot,
};
use crate::runtime_identity::RuntimeBackendKind;
use crate::session_access::{RegistrySnapshot, RegistryState, ScopeMetricsSnapshot, ScopeOutcome};

/// Render the runtime attribution and sandbox-lifecycle series.
///
/// Every label is a closed Rust enum, so the series count is finite by
/// construction: backends × identity results, and backends × lifecycle actions ×
/// emission results. Session ids, runtime ids, repositories, creators, and
/// trigger issues never appear (epic `OPS-04`).
pub(super) fn render_runtime_metrics(runtime: &RuntimeTelemetrySnapshot) -> String {
    let mut body = String::from(
        "# HELP fkst_runtime_identity_operations_total Runtime attribution operations by bounded backend and result.\n\
         # TYPE fkst_runtime_identity_operations_total counter\n",
    );
    for backend in RuntimeBackendKind::ALL {
        for result in IdentityOperationResult::ALL {
            body.push_str(&format!(
                "fkst_runtime_identity_operations_total{{backend=\"{}\",result=\"{}\"}} {}\n",
                backend.as_str(),
                result.as_str(),
                runtime.identity(backend, result),
            ));
        }
    }
    body.push_str(
        "# HELP fkst_sandbox_lifecycle_events_total Sandbox lifecycle events by bounded backend, action, and emission result.\n\
         # TYPE fkst_sandbox_lifecycle_events_total counter\n",
    );
    for backend in RuntimeBackendKind::ALL {
        for action in LifecycleAction::ALL {
            for result in LifecycleEmitResult::ALL {
                body.push_str(&format!(
                    "fkst_sandbox_lifecycle_events_total{{backend=\"{}\",action=\"{}\",result=\"{}\"}} {}\n",
                    backend.as_str(),
                    action.as_str(),
                    result.as_str(),
                    runtime.lifecycle(backend, action, result),
                ));
            }
        }
    }
    body
}

/// Render the session-access projection and operations-scope series.
///
/// Every label is a closed enum: the registry's three lifecycle states and the
/// fixed `(scope, result, reason)` triples. Session ids, repositories, actor ids,
/// logins, and configured entries are never labels and never values here — only
/// bounded counts (epic `OPS-04`).
pub(super) fn render_session_access_metrics(
    registry: &RegistrySnapshot,
    scope: &ScopeMetricsSnapshot,
) -> String {
    let mut body = format!(
        "# HELP fkst_session_access_registry_sessions Sessions in the published visibility generation.\n\
         # TYPE fkst_session_access_registry_sessions gauge\n\
         fkst_session_access_registry_sessions {}\n\
         # HELP fkst_session_access_registry_pending_repositories Repositories a staged generation still expects.\n\
         # TYPE fkst_session_access_registry_pending_repositories gauge\n\
         fkst_session_access_registry_pending_repositories {}\n\
         # HELP fkst_session_access_registry_generation Published generation number of the visibility projection.\n\
         # TYPE fkst_session_access_registry_generation gauge\n\
         fkst_session_access_registry_generation {}\n\
         # HELP fkst_session_access_registry_generation_state Current bounded readiness state of the projection.\n\
         # TYPE fkst_session_access_registry_generation_state gauge\n",
        registry.sessions, registry.pending_repositories, registry.generation,
    );
    for state in [
        RegistryState::Cold,
        RegistryState::Recovering,
        RegistryState::Ready,
    ] {
        body.push_str(&format!(
            "fkst_session_access_registry_generation_state{{state=\"{}\"}} {}\n",
            state.as_str(),
            u8::from(registry.state == state),
        ));
    }
    body.push_str(
        "# HELP fkst_operations_scope_decisions_total Operations scope selections by bounded outcome.\n\
         # TYPE fkst_operations_scope_decisions_total counter\n",
    );
    for outcome in ScopeOutcome::ALL {
        let (scope_label, result, reason) = outcome.labels();
        body.push_str(&format!(
            "fkst_operations_scope_decisions_total{{scope=\"{scope_label}\",result=\"{result}\",reason=\"{reason}\"}} {}\n",
            scope.count(outcome),
        ));
    }
    body
}

/// Render the audit-delivery series.
///
/// Every label is a closed enum (`accepted` / `full` / `retryable` / `permanent`
/// / `disabled`, and the bounded drop reasons): actor, session, repository,
/// request, and event ids are structured-log fields only, never labels (epic
/// `OPS-04`). Capture success is `accepted` — a PostHog `200` means accepted by
/// capture, never proven query-visible.
pub(super) fn render_audit_metrics(audit: &AuditMetricsSnapshot) -> String {
    format!(
        "# HELP fkst_audit_queue_depth Events admitted to the audit queue but not yet batched.\n\
         # TYPE fkst_audit_queue_depth gauge\n\
         fkst_audit_queue_depth {}\n\
         # HELP fkst_audit_events_enqueued_total Audit events by bounded admission result.\n\
         # TYPE fkst_audit_events_enqueued_total counter\n\
         fkst_audit_events_enqueued_total{{result=\"accepted\"}} {}\n\
         fkst_audit_events_enqueued_total{{result=\"full\"}} {}\n\
         fkst_audit_events_enqueued_total{{result=\"disabled\"}} {}\n\
         # HELP fkst_audit_batches_total Audit batches by bounded terminal result.\n\
         # TYPE fkst_audit_batches_total counter\n\
         fkst_audit_batches_total{{result=\"accepted\"}} {}\n\
         fkst_audit_batches_total{{result=\"retryable\"}} {}\n\
         fkst_audit_batches_total{{result=\"permanent\"}} {}\n\
         # HELP fkst_audit_delivery_attempts_total Individual capture attempts by bounded result.\n\
         # TYPE fkst_audit_delivery_attempts_total counter\n\
         fkst_audit_delivery_attempts_total{{result=\"accepted\"}} {}\n\
         fkst_audit_delivery_attempts_total{{result=\"retryable\"}} {}\n\
         fkst_audit_delivery_attempts_total{{result=\"permanent\"}} {}\n\
         # HELP fkst_audit_delivery_duration_seconds Time spent in capture attempts.\n\
         # TYPE fkst_audit_delivery_duration_seconds summary\n\
         fkst_audit_delivery_duration_seconds_sum {}\n\
         fkst_audit_delivery_duration_seconds_count {}\n\
         # HELP fkst_audit_events_dropped_total Audit events that will never reach capture, by bounded reason.\n\
         # TYPE fkst_audit_events_dropped_total counter\n\
         fkst_audit_events_dropped_total{{reason=\"queue_full\"}} {}\n\
         fkst_audit_events_dropped_total{{reason=\"invalid\"}} {}\n\
         fkst_audit_events_dropped_total{{reason=\"oversized\"}} {}\n\
         fkst_audit_events_dropped_total{{reason=\"retryable\"}} {}\n\
         fkst_audit_events_dropped_total{{reason=\"permanent\"}} {}\n\
         fkst_audit_events_dropped_total{{reason=\"shutdown\"}} {}\n\
         # HELP fkst_audit_shutdown_remaining_events Events still undelivered when the last drain ended.\n\
         # TYPE fkst_audit_shutdown_remaining_events gauge\n\
         fkst_audit_shutdown_remaining_events {}\n\
         # HELP fkst_audit_context_conflicts_total Conflicting write-once writes to a request's audit context.\n\
         # TYPE fkst_audit_context_conflicts_total counter\n\
         fkst_audit_context_conflicts_total {}\n",
        audit.queue_depth,
        audit.enqueued_accepted,
        audit.enqueued_full,
        audit.enqueued_disabled,
        audit.batches_accepted,
        audit.batches_retryable,
        audit.batches_permanent,
        audit.attempts_accepted,
        audit.attempts_retryable,
        audit.attempts_permanent,
        audit.delivery_duration_seconds_sum,
        audit.delivery_duration_count,
        audit.dropped_queue_full,
        audit.dropped_invalid,
        audit.dropped_oversized,
        audit.dropped_retryable,
        audit.dropped_permanent,
        audit.dropped_shutdown,
        audit.shutdown_remaining,
        audit.context_conflicts,
    )
}
