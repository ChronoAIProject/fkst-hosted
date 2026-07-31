//! The three RUNTIME effect verbs the reconciler drives a live session through:
//! refresh-pending, stop, and terminal cleanup.
//!
//! Split out of [`super::execute`] because these are the only effects that
//! change whether a runtime EXISTS, and because each is now paired with the
//! lifecycle records that make that change part of the deployment's permanent
//! history. Keeping them together makes "what can delete a session, and what
//! does it write down when it does" one short file.
//!
//! Every verb keeps the executor's discipline: idempotent, 404-tolerant, and
//! never propagating — one failed effect must not stall the rest of the repo's
//! reconcile.

use crate::audit::lifecycle::{LifecycleAction, LifecycleReason};
use crate::reconcile::desired::KillReason;
use crate::reconcile::execute::ReconcileCtx;
use crate::reconcile::lifecycle_audit::{self, SessionLifecycleFacts};
use crate::session_backend::BackendError;

// --- Pod lifecycle effects ---------------------------------------------------

/// Refresh a live pod's `last-pending-at` annotation to now (via the backend).
/// 404-tolerant: a pod deleted between the plan and the patch is a benign no-op.
pub(crate) async fn touch_pending(session_id: &str, ctx: &ReconcileCtx) {
    match ctx.backend.mark_pending(session_id).await {
        Ok(()) => tracing::debug!(session_id = %session_id, "reconcile: touched last-pending-at"),
        Err(BackendError::NotFound) => {}
        Err(error) => {
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: touch last-pending-at failed")
        }
    }
}

/// Stop a pod for `reason`, honouring the configured termination grace (via the
/// backend). 404-tolerant (already gone).
///
/// `deleted` is recorded for BOTH a successful delete and an already-absent
/// runtime: the contract is confirmed absence, and an idempotent no-op confirms
/// it just as well as a delete does.
pub(crate) async fn kill(session_id: &str, reason: KillReason, ctx: &ReconcileCtx) {
    tracing::info!(session_id = %session_id, ?reason, "reconcile: killing session pod");
    let facts = SessionLifecycleFacts::from_session_id(session_id);
    let reason_code = lifecycle_audit::kill_reason(reason);
    lifecycle_audit::emit(
        ctx,
        LifecycleAction::DeleteRequested,
        &facts,
        Some(reason_code),
    );
    match ctx.backend.stop_session(session_id, reason).await {
        Ok(()) => lifecycle_audit::emit(ctx, LifecycleAction::Deleted, &facts, Some(reason_code)),
        Err(BackendError::NotFound) => lifecycle_audit::emit(
            ctx,
            LifecycleAction::Deleted,
            &facts,
            Some(LifecycleReason::RuntimeNotFound),
        ),
        Err(error) => {
            lifecycle_audit::emit(
                ctx,
                LifecycleAction::DeleteFailed,
                &facts,
                Some(LifecycleReason::BackendUnavailable),
            );
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: kill delete failed")
        }
    }
}

/// GC a terminal pod (its owner-referenced Secret cascades away in the background,
/// via the backend). 404-tolerant.
pub(crate) async fn cleanup_terminal(session_id: &str, ctx: &ReconcileCtx) {
    let facts = SessionLifecycleFacts::from_session_id(session_id);
    lifecycle_audit::emit(
        ctx,
        LifecycleAction::DeleteRequested,
        &facts,
        Some(LifecycleReason::TerminalCleanup),
    );
    match ctx.backend.remove_terminal(session_id).await {
        Ok(()) => {
            lifecycle_audit::emit(
                ctx,
                LifecycleAction::Deleted,
                &facts,
                Some(LifecycleReason::TerminalCleanup),
            );
            tracing::info!(session_id = %session_id, "reconcile: cleaned up terminal session pod")
        }
        Err(BackendError::NotFound) => lifecycle_audit::emit(
            ctx,
            LifecycleAction::Deleted,
            &facts,
            Some(LifecycleReason::RuntimeNotFound),
        ),
        Err(error) => {
            lifecycle_audit::emit(
                ctx,
                LifecycleAction::DeleteFailed,
                &facts,
                Some(LifecycleReason::BackendUnavailable),
            );
            tracing::warn!(session_id = %session_id, error = %error, "reconcile: terminal cleanup failed")
        }
    }
}

#[cfg(test)]
#[path = "execute_runtime_tests.rs"]
mod tests;
