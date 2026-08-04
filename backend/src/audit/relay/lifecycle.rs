//! The lifecycle relay queue: durable delivery for the reconciler's effects.
//!
//! Sandbox lifecycle transitions are produced by a level-triggered reconciler on
//! a synchronous, non-async code path (`AuditHandle::submit_lifecycle`), so they
//! cannot await a durable acknowledgement the way an HTTP request can. They go
//! through a bounded in-process queue instead, drained by one task that POSTs
//! each event to the relay's idempotent `/internal/v1/audit/events`.
//!
//! ## Why an in-memory hop is acceptable HERE and not for requests
//!
//! A request's audit record is the only evidence the call happened; nothing can
//! reconstruct it. A lifecycle transition is different: the reconciler is
//! level-triggered and derives a DETERMINISTIC event id from
//! `(action, backend, session, incarnation)`, so an event lost to a restart is
//! re-emitted with the SAME id on the next sweep that observes the same state,
//! and the relay's idempotency turns the retry into one record rather than two.
//! That is why this queue may drop on overflow — loudly, with a metric — while a
//! request may not.
//!
//! Nothing here logs an event's content; a failure logs the bounded phase and
//! reason only.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::audit::lifecycle::SandboxLifecycleV1;
use crate::audit_relay::protocol::LifecycleEventV1;

use super::client::AuditRelayClient;

/// Depth of the bounded hand-off queue. Generous relative to a reconcile sweep's
/// effect count, so only a sustained relay outage can fill it.
const QUEUE_CAPACITY: usize = 1_024;

/// The cloneable handle the audit handle holds.
#[derive(Clone, Debug)]
pub struct LifecycleRelayQueue {
    sender: mpsc::Sender<LifecycleEventV1>,
}

impl LifecycleRelayQueue {
    /// Start the drain task and return its handle.
    pub fn spawn(client: Arc<AuditRelayClient>) -> Self {
        let (sender, receiver) = mpsc::channel(QUEUE_CAPACITY);
        tokio::spawn(drain(client, receiver));
        Self { sender }
    }

    /// Non-blocking admission. `false` means the queue was full or closed; the
    /// caller counts and logs the drop (a lifecycle hole must be visible).
    pub fn submit(&self, event: &SandboxLifecycleV1) -> bool {
        self.sender
            .try_send(LifecycleEventV1::from_domain(event))
            .is_ok()
    }
}

/// Drain the queue into the relay, one event at a time.
///
/// Serial on purpose: the relay is a single writer, and a burst of concurrent
/// POSTs would only queue behind each other there while costing this process a
/// task each.
async fn drain(client: Arc<AuditRelayClient>, mut receiver: mpsc::Receiver<LifecycleEventV1>) {
    while let Some(event) = receiver.recv().await {
        match client.submit_lifecycle(&event).await {
            Ok(_) => {}
            // A conflict means the id is ALREADY durable with different content —
            // for a deterministic id that is a genuine anomaly, not a duplicate,
            // so it is an error rather than a shrug.
            Err(error) => tracing::warn!(
                reason = error.kind(),
                "audit relay: a lifecycle event was not durably recorded"
            ),
        }
    }
    tracing::info!("audit relay: lifecycle queue closed");
}

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
