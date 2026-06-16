//! In-memory worker registry: the controller's view of worker liveness.
//!
//! This issue (#134) only TRACKS liveness — no claim/placement authority yet
//! (#135). The registry is `Clone` via an `Arc`, so the internal router, the
//! heartbeat handler, and the background expiry sweeper all share one map.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::RwLock;

use fkst_shared::protocol::{Draining, Heartbeat, LifecycleState, RegisterRequest, Released};

/// One worker's tracked state.
#[derive(Debug, Clone)]
pub struct WorkerEntry {
    pub worker_id: String,
    pub capacity: u32,
    pub lifecycle_state: LifecycleState,
    pub last_seen: Instant,
    pub running_sessions: Vec<String>,
}

/// The controller's in-memory map of worker_id -> liveness. A worker whose
/// `last_seen` is older than `liveness_ttl` is considered dead.
#[derive(Clone)]
pub struct WorkerRegistry {
    inner: Arc<RwLock<HashMap<String, WorkerEntry>>>,
    liveness_ttl: Duration,
}

impl WorkerRegistry {
    pub fn new(liveness_ttl: Duration) -> Self {
        Self {
            inner: Arc::new(RwLock::new(HashMap::new())),
            liveness_ttl,
        }
    }

    /// Register (or re-register) a worker: reset `last_seen` and mark `Active`.
    pub async fn register(&self, req: &RegisterRequest) {
        let mut map = self.inner.write().await;
        map.insert(
            req.worker_id.clone(),
            WorkerEntry {
                worker_id: req.worker_id.clone(),
                capacity: req.capacity,
                lifecycle_state: LifecycleState::Active,
                last_seen: Instant::now(),
                running_sessions: Vec::new(),
            },
        );
        tracing::info!(worker_id = %req.worker_id, capacity = req.capacity, "worker registered");
    }

    /// Update a worker's liveness from a heartbeat. An unknown worker self-heals
    /// (re-registered with unknown capacity) so a controller restart recovers
    /// the fleet from the workers' own heartbeats.
    pub async fn heartbeat(&self, hb: &Heartbeat) {
        let mut map = self.inner.write().await;
        match map.get_mut(&hb.worker_id) {
            Some(entry) => {
                entry.last_seen = Instant::now();
                entry.lifecycle_state = hb.lifecycle_state;
                entry.running_sessions = hb.running_sessions.clone();
            }
            None => {
                tracing::warn!(
                    worker_id = %hb.worker_id,
                    "heartbeat from unknown worker; self-healing as a re-register"
                );
                map.insert(
                    hb.worker_id.clone(),
                    WorkerEntry {
                        worker_id: hb.worker_id.clone(),
                        capacity: 0,
                        lifecycle_state: hb.lifecycle_state,
                        last_seen: Instant::now(),
                        running_sessions: hb.running_sessions.clone(),
                    },
                );
            }
        }
    }

    /// Record that a worker has begun draining (no reassignment here — #140).
    pub async fn mark_draining(&self, d: &Draining) {
        let mut map = self.inner.write().await;
        if let Some(entry) = map.get_mut(&d.worker_id) {
            entry.lifecycle_state = LifecycleState::Draining;
            entry.last_seen = Instant::now();
        }
        tracing::info!(
            worker_id = %d.worker_id,
            sessions = d.sessions.len(),
            checkpoint_done = d.checkpoint_done,
            "worker draining"
        );
    }

    /// Record a drain/handoff release acknowledgement (log only here — #140
    /// attaches reassignment behaviour).
    pub async fn note_released(&self, r: &Released) {
        tracing::info!(worker_id = %r.worker_id, session_id = %r.session_id, "worker released session");
    }

    /// Snapshot of workers whose last heartbeat is within the liveness TTL.
    pub async fn live_workers(&self) -> Vec<WorkerEntry> {
        let map = self.inner.read().await;
        let now = Instant::now();
        map.values()
            .filter(|e| now.duration_since(e.last_seen) <= self.liveness_ttl)
            .cloned()
            .collect()
    }

    /// Remove and return the ids of workers past the liveness TTL (used by the
    /// background sweeper). Logs each removal.
    pub async fn expire_stale(&self) -> Vec<String> {
        let mut map = self.inner.write().await;
        let now = Instant::now();
        let stale: Vec<String> = map
            .iter()
            .filter(|(_, e)| now.duration_since(e.last_seen) > self.liveness_ttl)
            .map(|(id, _)| id.clone())
            .collect();
        for id in &stale {
            map.remove(id);
            tracing::warn!(worker_id = %id, "worker expired (no heartbeat past liveness TTL)");
        }
        stale
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reg(id: &str) -> RegisterRequest {
        RegisterRequest {
            worker_id: id.to_string(),
            protocol_version: fkst_shared::protocol::PROTOCOL_VERSION,
            capacity: 4,
            engine_temp_root: "/tmp/e".to_string(),
        }
    }

    fn hb(id: &str) -> Heartbeat {
        Heartbeat {
            worker_id: id.to_string(),
            protocol_version: fkst_shared::protocol::PROTOCOL_VERSION,
            lifecycle_state: LifecycleState::Active,
            running_sessions: vec!["s1".to_string()],
            timestamp_unix_ms: 0,
        }
    }

    #[tokio::test]
    async fn register_then_heartbeat_keeps_worker_live() {
        let r = WorkerRegistry::new(Duration::from_secs(10));
        r.register(&reg("w1")).await;
        r.heartbeat(&hb("w1")).await;
        let live = r.live_workers().await;
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].worker_id, "w1");
        assert_eq!(live[0].running_sessions, vec!["s1".to_string()]);
    }

    #[tokio::test]
    async fn stale_worker_is_excluded_and_expired() {
        let r = WorkerRegistry::new(Duration::from_millis(1));
        r.register(&reg("w1")).await;
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(r.live_workers().await.is_empty(), "must drop from live set");
        let expired = r.expire_stale().await;
        assert_eq!(expired, vec!["w1".to_string()]);
        assert!(r.live_workers().await.is_empty());
    }

    #[tokio::test]
    async fn unknown_worker_heartbeat_self_heals() {
        let r = WorkerRegistry::new(Duration::from_secs(10));
        // No register first — the heartbeat self-heals as a re-register.
        r.heartbeat(&hb("ghost")).await;
        let live = r.live_workers().await;
        assert_eq!(live.len(), 1);
        assert_eq!(live[0].worker_id, "ghost");
        assert_eq!(
            live[0].capacity, 0,
            "unknown capacity until a real register"
        );
    }

    #[tokio::test]
    async fn mark_draining_flips_lifecycle_state() {
        let r = WorkerRegistry::new(Duration::from_secs(10));
        r.register(&reg("w1")).await;
        r.mark_draining(&Draining {
            worker_id: "w1".to_string(),
            sessions: vec![],
            checkpoint_done: false,
        })
        .await;
        let live = r.live_workers().await;
        assert_eq!(live[0].lifecycle_state, LifecycleState::Draining);
    }
}
