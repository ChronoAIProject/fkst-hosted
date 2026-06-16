//! The reassignment driver: redo a dead/draining worker's in-flight work on a
//! live worker (#135). Both the abrupt-death and graceful-drain paths converge
//! on [`ClaimMap::reassign`] (fence-bumped) — they differ only in latency and
//! checkpoint freshness:
//!
//! - **abrupt death** ([`ReassignDriver::on_worker_dead`]): the registry reports
//!   a worker past its heartbeat TTL. Reassign immediately. A brief double-
//!   compute window is possible (old worker's last beat → TTL expiry) but SAFE:
//!   fencing + the engine's git-idempotency fence-reject the old engine's late
//!   writes (no journal corruption ever).
//! - **graceful drain** ([`ReassignDriver::on_worker_draining`] →
//!   [`ReassignDriver::on_session_released`]): a worker being scaled down sends
//!   `Draining`; we WAIT for its `Released` ack (engine confirmed stopped) for a
//!   session before reassigning THAT session, collapsing the overlap to ~zero. A
//!   `Released` carries one `session_id`, so the graceful path is per-session.
//!
//! "Dispatching the redo" is a PUSH in the active path (#151): [`ClaimMap::reassign`]
//! bumps the fence + re-owns the claim to the new worker, then the
//! [`SecretRedispatch`] seam re-resolves the session's dispatch and queues it to
//! the new worker's outbound control channel (delivered on its next heartbeat).
//! The seam is async; the default is a no-op (the byte-identical posture when
//! dispatch is not wired), and [`crate::sessions::DispatchRedispatch`] is the
//! real implementation injected behind `FKST_DISPATCH_MODE`.

use std::sync::Arc;

use async_trait::async_trait;
use fkst_shared::protocol::LifecycleState;

use crate::controller::claims::ClaimMap;
use crate::controller::placement::{select_worker, WorkerLoad};
use crate::controller::registry::WorkerRegistry;

/// Seam for re-dispatching a session's run to its NEW worker on reassignment.
///
/// Async because the real implementation ([`crate::sessions::DispatchRedispatch`],
/// #140) re-resolves a fresh [`fkst_shared::protocol::ResolvedDispatch`] (an async
/// token mint + env/codex/ornn resolution) and queues it to the new worker's
/// outbound control channel — both awaits. `new_fence` is the bumped fence
/// [`ClaimMap::reassign`] just allocated; the resolver stamps it onto the
/// dispatch so the new worker's mid-run channels echo the current fence (a
/// superseded worker, still on the old fence, is fence-rejected). The default is
/// a no-op so a controller without dispatch wired (the byte-identical posture
/// until activation) does nothing.
#[async_trait]
pub trait SecretRedispatch: Send + Sync {
    async fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str, new_fence: i64);
}

/// Default no-op seam used when dispatch is not wired (the byte-identical
/// posture until activation injects the real [`crate::sessions::DispatchRedispatch`]).
#[derive(Debug, Default, Clone)]
pub struct NoopSecretRedispatch;

#[async_trait]
impl SecretRedispatch for NoopSecretRedispatch {
    async fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str, new_fence: i64) {
        tracing::debug!(
            session_id = %session_id,
            new_worker,
            new_fence,
            "secret re-dispatch seam (no-op; dispatch not wired)"
        );
    }
}

/// Drives reassignment over the controller's claim authority + worker registry.
#[derive(Clone)]
pub struct ReassignDriver {
    claims: Arc<ClaimMap>,
    registry: WorkerRegistry,
    max_load: u64,
    secrets: Arc<dyn SecretRedispatch>,
}

impl ReassignDriver {
    pub fn new(
        claims: Arc<ClaimMap>,
        registry: WorkerRegistry,
        max_load: u64,
        secrets: Arc<dyn SecretRedispatch>,
    ) -> Self {
        Self {
            claims,
            registry,
            max_load,
            secrets,
        }
    }

    /// Snapshot of ACTIVE live workers + their controller-authoritative load (the
    /// claim map's active count, immediate — not heartbeat-lagged), optionally
    /// excluding one worker (the dead/draining one being redistributed off).
    ///
    /// Only `LifecycleState::Active` workers are candidates: a Draining worker is
    /// shedding load, so reassigning ONTO it would just have to move the work
    /// again — it must never be a reassignment target. This mirrors the same
    /// Active filter [`crate::controller::ControllerHandle::snapshot_loads`]
    /// applies to NEW placement, so both placement and reassignment agree on the
    /// candidate set.
    async fn live_worker_loads(&self, exclude: Option<&str>) -> Vec<WorkerLoad> {
        self.registry
            .live_workers()
            .await
            .into_iter()
            .filter(|w| w.lifecycle_state == LifecycleState::Active)
            .filter(|w| Some(w.worker_id.as_str()) != exclude)
            .map(|w| WorkerLoad {
                active_sessions: self.claims.active_load(&w.worker_id),
                worker_id: w.worker_id,
            })
            .collect()
    }

    /// Reassign every ACTIVE entry owned by `worker_id` onto a live worker.
    /// Returns the count actually reassigned. An entry that finds no live worker
    /// with capacity is LEFT in place (still owned by `worker_id`, retriable on
    /// the next tick) — never lost, never errored. The fence is bumped BEFORE
    /// the redo is considered dispatched (the superseded-worker guard).
    async fn reassign_owned(&self, worker_id: &str) -> usize {
        let owned = self.claims.owned_by(worker_id);
        let mut reassigned = 0;
        for entry in owned {
            let loads = self.live_worker_loads(Some(worker_id)).await;
            let Some(chosen) = select_worker(&loads, self.max_load).cloned() else {
                tracing::warn!(
                    lease_key = %entry.lease_key,
                    from_worker = worker_id,
                    "reassignment found no live worker with capacity; left pending + retriable"
                );
                continue;
            };
            if let Some(reassigned_entry) =
                self.claims.reassign(&entry.lease_key, &chosen.worker_id)
            {
                // The claim is now owned by the new worker with a freshly bumped
                // fence. Re-dispatch the session to that worker stamped with the
                // NEW fence so its mid-run channels echo the current fence (the
                // superseded-worker guard). The returned entry carries the bumped
                // fence; pass it through.
                self.secrets
                    .re_dispatch(
                        entry.session_id,
                        &chosen.worker_id,
                        reassigned_entry.fencing_id,
                    )
                    .await;
                reassigned += 1;
            }
        }
        reassigned
    }

    /// Per-session graceful-drain entry point (#140): reassign exactly the ONE
    /// session a worker's `Released` ack named (its engine confirmed stopped) onto
    /// a live worker, returning whether it reassigned. Unlike
    /// [`reassign_owned`](Self::reassign_owned) (which sweeps a whole worker), a
    /// `Released` carries a single `session_id`, so this resolves it to its
    /// `lease_key` and reassigns just that claim. An unknown session (already
    /// reassigned, or terminal) or a fleet with no live capacity is a no-op
    /// returning `false` (the entry, if any, stays put and is retriable on the
    /// next tick) — never lost, never errored.
    pub async fn on_session_released(&self, session_id: bson::Uuid) -> bool {
        let Some(lease_key) = self.claims.lease_key_for_session(session_id) else {
            tracing::debug!(
                session_id = %session_id,
                "released session has no live claim; nothing to reassign"
            );
            return false;
        };
        // The CURRENT owner is excluded from candidate selection so the released
        // worker is never picked as its own replacement.
        let current_owner = self.claims.get(&lease_key).map(|e| e.owner_worker);
        let loads = self.live_worker_loads(current_owner.as_deref()).await;
        let Some(chosen) = select_worker(&loads, self.max_load).cloned() else {
            tracing::warn!(
                lease_key = %lease_key,
                session_id = %session_id,
                "released session found no live worker with capacity; left pending + retriable"
            );
            return false;
        };
        let Some(reassigned_entry) = self.claims.reassign(&lease_key, &chosen.worker_id) else {
            // Raced: the entry vanished between the lookup and the reassign.
            tracing::debug!(
                lease_key = %lease_key,
                session_id = %session_id,
                "released session's claim vanished before reassign"
            );
            return false;
        };
        self.secrets
            .re_dispatch(session_id, &chosen.worker_id, reassigned_entry.fencing_id)
            .await;
        true
    }

    /// Abrupt-death entry point: reassign all of the dead worker's in-flight work
    /// immediately. Returns the count reassigned.
    pub async fn on_worker_dead(&self, worker_id: &str) -> usize {
        tracing::warn!(
            worker_id,
            "worker dead (heartbeat TTL exceeded); reassigning its work"
        );
        self.reassign_owned(worker_id).await
    }

    /// Graceful-drain entry point: the worker announced `Draining`. Do NOT
    /// reassign yet — the worker is flushing/handing off its live sessions and
    /// will send a per-session `Released` ack (engine confirmed stopped) for each,
    /// at which point [`Self::on_session_released`] reassigns that one session,
    /// collapsing the double-run window to ~zero. Marking the worker `Draining`
    /// in the registry ALSO removes it from the placement + reassignment candidate
    /// set (the Active filter), so it stops receiving new work immediately.
    pub async fn on_worker_draining(&self, worker_id: &str) {
        tracing::info!(
            worker_id,
            "worker draining; awaiting per-session Released acks before reassigning"
        );
    }
}

#[cfg(test)]
#[path = "reassign_tests.rs"]
mod tests;
