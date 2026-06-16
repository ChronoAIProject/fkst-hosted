//! Unit tests for [`super::ReassignDriver`] (split out via `#[path] mod tests;`
//! to keep `reassign.rs` under the 500-line file budget). `super::*` resolves to
//! the `reassign` module's items.
//!
//! The seam is exercised with a [`SpySecrets`] fake (records every re-dispatch
//! call's `(session_id, new_worker, new_fence)`), so the reassignment LOGIC is
//! tested without a Mongo-backed `DispatchRedispatch`. The real re-dispatch
//! (resolve + enqueue) is covered end-to-end by the testcontainers integration
//! test `tests/reassign_redispatch.rs`.

use super::*;
use std::sync::Mutex;
use std::time::Duration;

use fkst_shared::protocol::{Draining, RegisterRequest, PROTOCOL_VERSION};

fn reg(id: &str) -> RegisterRequest {
    RegisterRequest {
        worker_id: id.to_string(),
        protocol_version: PROTOCOL_VERSION,
        capacity: 100,
        engine_temp_root: "/tmp/e".to_string(),
    }
}

/// Spy seam recording every `(session_id, new_worker, new_fence)` re-dispatch
/// call, so a test can assert the seam fired with the BUMPED fence.
#[derive(Default)]
struct SpySecrets {
    calls: Mutex<Vec<(bson::Uuid, String, i64)>>,
}

#[async_trait]
impl SecretRedispatch for SpySecrets {
    async fn re_dispatch(&self, session_id: bson::Uuid, new_worker: &str, new_fence: i64) {
        self.calls
            .lock()
            .unwrap()
            .push((session_id, new_worker.to_string(), new_fence));
    }
}

/// Build a driver over `claims` with each id in `live` registered ACTIVE.
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
async fn reassign_calls_secret_redispatch_seam_with_the_bumped_fence() {
    let claims = Arc::new(ClaimMap::new());
    let sid = bson::Uuid::new();
    let original = claims.claim("a", sid, None, "dead").unwrap();
    let spy = Arc::new(SpySecrets::default());
    let driver = driver_with(claims.clone(), &["live"], spy.clone()).await;

    driver.on_worker_dead("dead").await;
    let new_fence = claims.get("a").unwrap().fencing_id;
    assert!(new_fence > original.fencing_id, "fence bumped on reassign");
    let calls = spy.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    // The seam is handed the NEW worker AND the BUMPED fence (so the re-resolved
    // dispatch stamps the current fence the new worker echoes).
    assert_eq!(calls[0], (sid, "live".to_string(), new_fence));
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

#[tokio::test]
async fn on_session_released_reassigns_one_session_with_bumped_fence() {
    let claims = Arc::new(ClaimMap::new());
    let sid = bson::Uuid::new();
    let original = claims.claim("a", sid, None, "drain").unwrap();
    // A second session on the same draining worker must NOT move when only `sid`
    // is released (the graceful path is per-session, not per-worker).
    let other = bson::Uuid::new();
    claims.claim("b", other, None, "drain").unwrap();
    let spy = Arc::new(SpySecrets::default());
    let driver = driver_with(claims.clone(), &["live"], spy.clone()).await;

    // Draining alone reassigns nothing.
    driver.on_worker_draining("drain").await;
    assert_eq!(claims.get("a").unwrap().owner_worker, "drain");

    // The per-session Released ack reassigns exactly that session.
    let reassigned = driver.on_session_released(sid).await;
    assert!(reassigned, "the released session reassigned");
    let moved = claims.get("a").unwrap();
    assert_eq!(moved.owner_worker, "live");
    assert!(moved.fencing_id > original.fencing_id, "fence bumped");
    // The OTHER session stayed on the draining worker.
    assert_eq!(claims.get("b").unwrap().owner_worker, "drain");

    // The seam fired once with the bumped fence for the released session only.
    let calls = spy.calls.lock().unwrap();
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], (sid, "live".to_string(), moved.fencing_id));
}

#[tokio::test]
async fn on_session_released_unknown_session_is_a_noop() {
    let claims = Arc::new(ClaimMap::new());
    let spy = Arc::new(SpySecrets::default());
    let driver = driver_with(claims, &["live"], spy.clone()).await;
    // No claim for this session: a no-op, no seam call, no panic.
    assert!(!driver.on_session_released(bson::Uuid::new()).await);
    assert!(spy.calls.lock().unwrap().is_empty());
}

#[tokio::test]
async fn draining_worker_is_skipped_by_placement_and_reassign() {
    // Two live workers: `keep` (Active) and `drain` (about to be marked Draining).
    let claims = Arc::new(ClaimMap::new());
    let sid = bson::Uuid::new();
    claims.claim("a", sid, None, "dead").unwrap();
    let registry = WorkerRegistry::new(Duration::from_secs(60));
    registry.register(&reg("keep")).await;
    registry.register(&reg("drain")).await;
    // Mark `drain` Draining: it must drop out of the reassignment candidate set.
    registry
        .mark_draining(&Draining {
            worker_id: "drain".to_string(),
            sessions: vec![],
            checkpoint_done: false,
        })
        .await;
    let driver = ReassignDriver::new(claims.clone(), registry, 0, Arc::new(NoopSecretRedispatch));

    // The dead worker's session must land on the ACTIVE worker, never the
    // Draining one (even though both are "live" by heartbeat).
    let loads = driver.live_worker_loads(Some("dead")).await;
    assert_eq!(loads.len(), 1, "only the Active worker is a candidate");
    assert_eq!(loads[0].worker_id, "keep");

    let n = driver.on_worker_dead("dead").await;
    assert_eq!(n, 1);
    assert_eq!(
        claims.get("a").unwrap().owner_worker,
        "keep",
        "reassigned onto the Active worker, not the Draining one"
    );
}

#[tokio::test]
async fn dead_worker_sweep_reassigns_all_then_removes_entry() {
    // Emulate the sweeper loop end-to-end: a worker goes stale, `expire_stale`
    // removes it, and `on_worker_dead` (the per-expired-id call the sweeper now
    // makes under dispatch mode) reassigns ALL of its in-flight work.
    let claims = Arc::new(ClaimMap::new());
    claims.claim("a", bson::Uuid::new(), None, "dead").unwrap();
    claims.claim("b", bson::Uuid::new(), None, "dead").unwrap();
    // A short TTL so `dead` can be swept. `live` is registered AFTER the stale
    // window, so a single shared TTL still leaves it fresh at sweep time (one TTL
    // applies to every entry, so the survivor must be registered last).
    let registry = WorkerRegistry::new(Duration::from_millis(20));
    registry.register(&reg("dead")).await;
    let driver = ReassignDriver::new(
        claims.clone(),
        registry.clone(),
        0,
        Arc::new(NoopSecretRedispatch),
    );

    // Let `dead` go stale, THEN register the fresh survivor so it is within TTL.
    tokio::time::sleep(Duration::from_millis(60)).await;
    registry.register(&reg("live")).await;

    // Sweep: only `dead` is past TTL; `live` (just registered) survives.
    let expired = registry.expire_stale().await;
    assert_eq!(expired, vec!["dead".to_string()], "only the stale worker");

    // Reassign each expired worker's work (what the sweeper does per id).
    let mut total = 0;
    for id in &expired {
        total += driver.on_worker_dead(id).await;
    }
    assert_eq!(total, 2, "both of the dead worker's sessions reassigned");
    assert_eq!(claims.get("a").unwrap().owner_worker, "live");
    assert_eq!(claims.get("b").unwrap().owner_worker, "live");
    // The dead worker's registry entry is gone (swept).
    assert!(
        registry
            .live_workers()
            .await
            .iter()
            .all(|w| w.worker_id != "dead"),
        "dead worker removed from the registry"
    );
}

#[tokio::test]
async fn reused_worker_id_does_not_reassign() {
    // A worker re-registering under the SAME id (e.g. a restarted pod reusing its
    // worker_id) must NOT trigger any reassignment — register only overwrites the
    // registry entry as Active; it never touches the claim map. The driver has no
    // hook on register, so a re-register is inert by construction; this pins it.
    let claims = Arc::new(ClaimMap::new());
    let sid = bson::Uuid::new();
    let original = claims.claim("a", sid, None, "w1").unwrap();
    let spy = Arc::new(SpySecrets::default());
    let driver = driver_with(claims.clone(), &["w1"], spy.clone()).await;

    // Re-register the SAME id (reused worker_id).
    driver.registry.register(&reg("w1")).await;

    // The claim is untouched: same owner, same fence, and the seam never fired.
    let after = claims.get("a").unwrap();
    assert_eq!(after.owner_worker, "w1");
    assert_eq!(after.fencing_id, original.fencing_id, "fence unchanged");
    assert!(
        spy.calls.lock().unwrap().is_empty(),
        "a reused worker_id must not re-dispatch anything"
    );
}
