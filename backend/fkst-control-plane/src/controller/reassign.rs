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
//!   [`ReassignDriver::on_worker_released`]): a worker being scaled down sends
//!   `Draining`; we WAIT for its `Released` ack (engine confirmed stopped)
//!   before reassigning, collapsing the overlap to ~zero. The detailed drain
//!   message choreography lives in #140; this module exposes the primitive +
//!   entry points both paths call.
//!
//! "Dispatching the redo" is logical in the pull model: [`ClaimMap::reassign`]
//! leaves the claim `Pending` owned by the new worker, which pulls + spawns the
//! engine in #136. The in-memory secret re-dispatch is the [`SecretRedispatch`]
//! seam (a no-op default; #138 injects the real one).

use std::sync::Arc;

use crate::controller::claims::ClaimMap;
use crate::controller::placement::{select_worker, WorkerLoad};
use crate::controller::registry::WorkerRegistry;

/// Seam for re-dispatching a session's in-memory secrets to its NEW worker on
/// reassignment (#138 injects the real implementation; the default is a no-op).
pub trait SecretRedispatch: Send + Sync {
    fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str);
}

/// Default no-op seam used until #138 wires real secret dispatch.
#[derive(Debug, Default, Clone)]
pub struct NoopSecretRedispatch;

impl SecretRedispatch for NoopSecretRedispatch {
    fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str) {
        tracing::debug!(session_id = %session_id, new_worker, "secret re-dispatch seam (no-op until #138)");
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

    /// Snapshot of live workers + their controller-authoritative load (the claim
    /// map's active count, immediate — not heartbeat-lagged), optionally
    /// excluding one worker (the dead/draining one being redistributed off).
    async fn live_worker_loads(&self, exclude: Option<&str>) -> Vec<WorkerLoad> {
        self.registry
            .live_workers()
            .await
            .into_iter()
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
            if self
                .claims
                .reassign(&entry.lease_key, &chosen.worker_id)
                .is_some()
            {
                // Redo is now "dispatched": the claim is Pending owned by the new
                // worker (it pulls + spawns the engine in #136). Re-dispatch the
                // session's in-memory secrets to the new worker (#138 seam).
                self.secrets
                    .re_dispatch(entry.session_id, &chosen.worker_id);
                reassigned += 1;
            }
        }
        reassigned
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
    /// reassign yet — wait for the `Released` ack (engine confirmed stopped) via
    /// [`Self::on_worker_released`], collapsing the double-run window to ~zero.
    /// The stop-request + bounded-timeout-fallback-to-abrupt choreography is #140.
    pub async fn on_worker_draining(&self, worker_id: &str) {
        tracing::info!(
            worker_id,
            "worker draining; awaiting Released before reassigning"
        );
    }

    /// Graceful-drain completion: the worker's `Released` ack arrived (engine
    /// confirmed stopped). Reassign its in-flight work now (~zero overlap).
    pub async fn on_worker_released(&self, worker_id: &str) -> usize {
        tracing::info!(worker_id, "worker released; reassigning its drained work");
        self.reassign_owned(worker_id).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::Duration;

    use fkst_shared::protocol::{RegisterRequest, PROTOCOL_VERSION};

    fn reg(id: &str) -> RegisterRequest {
        RegisterRequest {
            worker_id: id.to_string(),
            protocol_version: PROTOCOL_VERSION,
            capacity: 100,
            engine_temp_root: "/tmp/e".to_string(),
        }
    }

    /// Spy seam recording every (session_id, new_worker) re-dispatch call.
    #[derive(Default)]
    struct SpySecrets {
        calls: Mutex<Vec<(bson::Uuid, String)>>,
    }
    impl SecretRedispatch for SpySecrets {
        fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str) {
            self.calls
                .lock()
                .unwrap()
                .push((session_id, new_worker.to_string()));
        }
    }

    async fn driver_with(
        claims: Arc<ClaimMap>,
        live: &[&str],
        secrets: Arc<dyn SecretRedispatch>,
    ) -> ReassignDriver {
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        for id in live {
            registry.register(&reg(id)).await;
        }
        ReassignDriver::new(claims, registry, 0, secrets)
    }

    #[tokio::test]
    async fn abrupt_death_reassigns_all_owned_active_entries_with_fresh_fences() {
        let claims = Arc::new(ClaimMap::new());
        let a = claims.claim("a", bson::Uuid::new(), None, "dead").unwrap();
        let b = claims.claim("b", bson::Uuid::new(), None, "dead").unwrap();
        let driver = driver_with(claims.clone(), &["live"], Arc::new(NoopSecretRedispatch)).await;

        let n = driver.on_worker_dead("dead").await;
        assert_eq!(n, 2);
        let ra = claims.get("a").unwrap();
        let rb = claims.get("b").unwrap();
        assert_eq!(ra.owner_worker, "live");
        assert_eq!(rb.owner_worker, "live");
        assert!(ra.fencing_id > a.fencing_id);
        assert!(rb.fencing_id > b.fencing_id);
    }

    #[tokio::test]
    async fn graceful_drain_waits_for_released_then_reassigns() {
        let claims = Arc::new(ClaimMap::new());
        claims.claim("a", bson::Uuid::new(), None, "drain").unwrap();
        let driver = driver_with(claims.clone(), &["live"], Arc::new(NoopSecretRedispatch)).await;

        // Draining alone must NOT reassign.
        driver.on_worker_draining("drain").await;
        assert_eq!(
            claims.get("a").unwrap().owner_worker,
            "drain",
            "no reassign before Released"
        );

        // The Released ack triggers the reassignment.
        let n = driver.on_worker_released("drain").await;
        assert_eq!(n, 1);
        assert_eq!(claims.get("a").unwrap().owner_worker, "live");
    }

    #[tokio::test]
    async fn reassign_calls_secret_redispatch_seam_for_new_worker() {
        let claims = Arc::new(ClaimMap::new());
        let sid = bson::Uuid::new();
        claims.claim("a", sid, None, "dead").unwrap();
        let spy = Arc::new(SpySecrets::default());
        let driver = driver_with(claims, &["live"], spy.clone()).await;

        driver.on_worker_dead("dead").await;
        let calls = spy.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0], (sid, "live".to_string()));
    }

    #[tokio::test]
    async fn no_capacity_leaves_entry_pending_and_retriable() {
        let claims = Arc::new(ClaimMap::new());
        claims.claim("a", bson::Uuid::new(), None, "dead").unwrap();
        // No live workers at all (the only worker is the dead one, excluded).
        let driver = driver_with(claims.clone(), &[], Arc::new(NoopSecretRedispatch)).await;
        let n = driver.on_worker_dead("dead").await;
        assert_eq!(n, 0, "nothing reassigned");
        // The entry survives, still owned by the dead worker, retriable.
        let e = claims.get("a").unwrap();
        assert_eq!(e.owner_worker, "dead");
        assert!(super::super::claims::is_active(e.status));

        // Bring a live worker up: the SAME entry now reassigns (retriable).
        driver.registry.register(&reg("late")).await;
        let n2 = driver.on_worker_dead("dead").await;
        assert_eq!(n2, 1);
        assert_eq!(claims.get("a").unwrap().owner_worker, "late");
    }

    #[tokio::test]
    async fn load_snapshot_excludes_only_the_named_worker() {
        let claims = Arc::new(ClaimMap::new());
        let driver = driver_with(claims, &["w1", "w2"], Arc::new(NoopSecretRedispatch)).await;
        let loads = driver.live_worker_loads(Some("w1")).await;
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].worker_id, "w2");
    }
}
