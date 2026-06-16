//! `ControllerHandle` — the bundle the session service holds to route placement
//! through the controller's in-memory authority instead of the Mongo
//! distributor (#135).
//!
//! It carries the [`ClaimMap`] (the mutual-exclusion + fence authority) and the
//! [`WorkerRegistry`] (the live worker set), and snapshots the
//! controller-authoritative per-worker load (the claim map's active count —
//! immediate, not heartbeat-lagged) for selection.

use std::sync::Arc;

use fkst_shared::protocol::ControlMessage;

use crate::controller::claims::ClaimMap;
use crate::controller::placement::{self, Placement, PlacementError, WorkerLoad};
use crate::controller::registry::WorkerRegistry;

/// Cheap-to-clone bundle (`ClaimMap` behind an `Arc`, the registry already
/// `Arc`-backed) the session service routes placement through.
#[derive(Clone)]
pub struct ControllerHandle {
    claims: Arc<ClaimMap>,
    registry: WorkerRegistry,
    max_load: u64,
}

impl ControllerHandle {
    pub fn new(claims: Arc<ClaimMap>, registry: WorkerRegistry, max_load: u64) -> Self {
        Self {
            claims,
            registry,
            max_load,
        }
    }

    /// The shared claim authority (for the reassignment driver + reflection).
    pub fn claims(&self) -> &Arc<ClaimMap> {
        &self.claims
    }

    /// Snapshot every ACTIVE live worker with its controller-authoritative load.
    ///
    /// A Draining worker (#140) is shedding load, so it must stop receiving NEW
    /// placements — filtering it out here mirrors the same Active filter the
    /// reassignment driver applies to its candidate set, so placement and
    /// reassignment agree on which workers can take work.
    pub async fn snapshot_loads(&self) -> Vec<WorkerLoad> {
        self.registry
            .live_workers()
            .await
            .into_iter()
            .filter(|w| w.lifecycle_state == fkst_shared::protocol::LifecycleState::Active)
            .map(|w| WorkerLoad {
                active_sessions: self.claims.active_load(&w.worker_id),
                worker_id: w.worker_id,
            })
            .collect()
    }

    /// Queue a control message for `worker_id`, delivered on its next heartbeat
    /// (#151 i7b). The activation path uses this to hand a placed worker its
    /// fully-resolved [`ControlMessage::ResolvedDispatch`]; the message may
    /// carry secrets, so the registry never logs its body — only kind/count.
    pub async fn enqueue_dispatch(&self, worker_id: &str, message: ControlMessage) {
        self.registry.enqueue_control(worker_id, message).await;
    }

    /// Place a session on the least-loaded live worker via the claim authority.
    pub async fn place(
        &self,
        lease_key: &str,
        session_id: bson::Uuid,
        goal_id: Option<bson::Uuid>,
    ) -> Result<Placement, PlacementError> {
        let loads = self.snapshot_loads().await;
        placement::place(
            &self.claims,
            &loads,
            self.max_load,
            lease_key,
            session_id,
            goal_id,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    use fkst_shared::protocol::{RegisterRequest, PROTOCOL_VERSION};

    fn reg(id: &str) -> RegisterRequest {
        RegisterRequest {
            worker_id: id.to_string(),
            protocol_version: PROTOCOL_VERSION,
            capacity: 0,
            engine_temp_root: "/tmp/e".to_string(),
        }
    }

    #[tokio::test]
    async fn place_routes_through_the_claim_authority_and_picks_least_loaded() {
        let claims = Arc::new(ClaimMap::new());
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        registry.register(&reg("w1")).await;
        registry.register(&reg("w2")).await;
        // Load w1 so w2 (idle) is least-loaded.
        claims
            .claim("other", bson::Uuid::new(), None, "w1")
            .unwrap();

        let handle = ControllerHandle::new(claims.clone(), registry, 0);
        let p = handle.place("pkg", bson::Uuid::new(), None).await.unwrap();
        assert_eq!(p.worker_id, "w2");
        assert_eq!(handle.claims().get("pkg").unwrap().owner_worker, "w2");
    }

    #[tokio::test]
    async fn place_returns_no_capacity_with_no_live_workers() {
        let claims = Arc::new(ClaimMap::new());
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        let handle = ControllerHandle::new(claims, registry, 0);
        let err = handle
            .place("pkg", bson::Uuid::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, PlacementError::NoCapacity));
    }

    #[tokio::test]
    async fn place_skips_a_draining_worker() {
        use fkst_shared::protocol::Draining;

        let claims = Arc::new(ClaimMap::new());
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        registry.register(&reg("keep")).await;
        registry.register(&reg("drain")).await;
        // Mark `drain` Draining: it must drop out of the placement candidate set.
        registry
            .mark_draining(&Draining {
                worker_id: "drain".to_string(),
                sessions: vec![],
                checkpoint_done: false,
            })
            .await;
        let handle = ControllerHandle::new(claims, registry, 0);

        // The snapshot must exclude the Draining worker entirely.
        let loads = handle.snapshot_loads().await;
        assert_eq!(loads.len(), 1);
        assert_eq!(loads[0].worker_id, "keep");

        // Placement therefore lands on the Active worker, never the Draining one.
        let p = handle.place("pkg", bson::Uuid::new(), None).await.unwrap();
        assert_eq!(p.worker_id, "keep");
    }

    #[tokio::test]
    async fn place_returns_no_capacity_when_all_workers_are_draining() {
        use fkst_shared::protocol::Draining;

        let claims = Arc::new(ClaimMap::new());
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        registry.register(&reg("drain")).await;
        registry
            .mark_draining(&Draining {
                worker_id: "drain".to_string(),
                sessions: vec![],
                checkpoint_done: false,
            })
            .await;
        let handle = ControllerHandle::new(claims, registry, 0);
        // The only live worker is Draining, so there is no Active capacity.
        let err = handle
            .place("pkg", bson::Uuid::new(), None)
            .await
            .unwrap_err();
        assert!(matches!(err, PlacementError::NoCapacity));
    }

    #[tokio::test]
    async fn enqueue_dispatch_queues_to_the_registry_for_the_worker() {
        use fkst_shared::protocol::ControlMessage;

        let claims = Arc::new(ClaimMap::new());
        let registry = WorkerRegistry::new(Duration::from_secs(60));
        registry.register(&reg("w1")).await;
        // Clone the registry so the test can drain the same shared outbound queue
        // the handle enqueues into (the handle moves its copy in).
        let handle = ControllerHandle::new(claims, registry.clone(), 0);

        handle
            .enqueue_dispatch(
                "w1",
                ControlMessage::StopSession {
                    session_id: "s1".to_string(),
                    reason: "queued".to_string(),
                },
            )
            .await;

        // The heartbeat handler drains via `take_control`; assert the message
        // landed on w1's queue (and nowhere else).
        let drained = registry.take_control("w1").await;
        assert_eq!(drained.len(), 1, "exactly the one enqueued message");
        assert!(
            matches!(&drained[0], ControlMessage::StopSession { session_id, .. } if session_id == "s1")
        );
        assert!(
            registry.take_control("w2").await.is_empty(),
            "no message for an unrelated worker"
        );
    }
}
