//! The real re-dispatch seam (#140): when the controller reassigns a session to a
//! NEW worker (a dead worker swept, or a graceful-drain `Released` ack), this
//! re-resolves the session's full [`ResolvedDispatch`] and queues it to the new
//! worker's outbound control channel — exactly the resolve + enqueue the live
//! placement path (`create_for_goal`'s controller arm, #151 i7b) performs, but
//! triggered by a reassignment rather than a fresh trigger.
//!
//! It implements [`crate::controller::SecretRedispatch`] (the async seam the
//! [`crate::controller::ReassignDriver`] calls), and is injected behind
//! `FKST_DISPATCH_MODE` via [`super::SessionService::make_redispatch`]. With
//! dispatch off it is never constructed (the driver holds the
//! `NoopSecretRedispatch` default), so this code path is inert.
//!
//! ## Failure posture (load-bearing)
//! The claim is ALREADY reassigned (fence bumped, owner re-pointed) by the time
//! the seam runs — the `ReassignDriver` reassigns first, then re-dispatches. So a
//! re-dispatch resolution FAILURE must NOT unwind the reassign: the claim stays
//! claimed-but-unrun (owned by the new worker, on the new fence), which is
//! retriable on the next death/registration tick. The seam therefore logs the
//! error and returns; it never panics and never leaves the claim inconsistent.
//!
//! ## Secret hygiene
//! The resolved dispatch and the held raw token are `SecretString`-bearing and
//! are NEVER logged here — only non-secret identifiers (`session_id`,
//! `new_worker`, `new_fence`) and the (secret-free) `DispatchError`.

use std::sync::Arc;

use async_trait::async_trait;
use fkst_shared::protocol::ControlMessage;

use crate::controller::SecretRedispatch;

use super::service::Inner;

/// Re-resolves + re-queues a reassigned session's dispatch to its new worker.
///
/// Holds the same [`Inner`] the in-process driver and the live-placement
/// resolver use (so it resolves from one set of wiring) plus the worker registry
/// it enqueues onto. Both are cheap to clone (`Inner` behind an `Arc`, the
/// registry `Arc`-backed).
pub struct DispatchRedispatch {
    inner: Arc<Inner>,
    registry: crate::controller::WorkerRegistry,
}

impl DispatchRedispatch {
    /// Construct the seam from the service's shared internals + the registry that
    /// owns the per-worker outbound control queues. Crate-internal: built only by
    /// [`super::service::SessionService::make_redispatch`].
    pub(super) fn new(inner: Arc<Inner>, registry: crate::controller::WorkerRegistry) -> Self {
        Self { inner, registry }
    }
}

#[async_trait]
impl SecretRedispatch for DispatchRedispatch {
    async fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str, new_fence: i64) {
        // (a) Re-read the (authoritative) session document. A vanished document
        //     (a concurrent stop/delete) has nothing left to dispatch — the claim
        //     will converge on the next tick; just warn and return.
        let session = match self.inner.get_session(session_id).await {
            Ok(Some(session)) => session,
            Ok(None) => {
                tracing::warn!(
                    session_id = %session_id,
                    new_worker,
                    "re-dispatch: session document vanished; nothing to dispatch"
                );
                return;
            }
            Err(error) => {
                tracing::error!(
                    session_id = %session_id,
                    new_worker,
                    error = %error,
                    "re-dispatch: failed to load session; claim left pending (retriable)"
                );
                return;
            }
        };

        // (b) The triggering user's held raw token, if this controller still holds
        //     it (it is dropped on a session's terminal exit and lost on a
        //     controller restart). Absent => the resolver skips per-session NyxID
        //     provisioning, exactly as the in-process driver does without a token.
        //     Never logged.
        let raw_token = self.inner.held_token(session_id);

        // (c) Resolve the full dispatch stamped with the NEW worker + NEW fence,
        //     then queue it to the new worker (delivered on its next heartbeat).
        match super::dispatch::resolve_dispatch(
            &self.inner,
            &session,
            raw_token.as_ref(),
            new_worker,
            new_fence,
        )
        .await
        {
            Ok(dispatch) => {
                self.registry
                    .enqueue_control(
                        new_worker,
                        ControlMessage::ResolvedDispatch(Box::new(dispatch)),
                    )
                    .await;
                tracing::info!(
                    session_id = %session_id,
                    new_worker,
                    new_fence,
                    "re-dispatch resolved + queued to the new worker (delivered on next heartbeat)"
                );
            }
            Err(error) => {
                // The claim is already reassigned (fence bumped); a failed resolve
                // leaves it claimed-but-unrun, retriable on the next death/
                // registration tick. Do NOT unwind the reassign or panic. The
                // DispatchError carries no secret, so logging it is safe.
                tracing::error!(
                    session_id = %session_id,
                    new_worker,
                    new_fence,
                    error = %error,
                    "re-dispatch resolution failed; claim left pending (retriable)"
                );
            }
        }
    }
}
