//! In-memory claim authority — the controller's replacement for the Mongo
//! `leases` collection + the `transition_guarded` CAS (database-free pivot,
//! #135).
//!
//! Because exactly ONE controller replica is authoritative, mutual exclusion
//! (at-most-one engine per package/goal) is just this controller checking its
//! own map under a `Mutex` — no lease store, no cross-pod atomic CAS. The
//! `fencing_id` survives only as a controller-issued **monotonic per-run id**
//! used for **journaling idempotency / superseded-worker rejection**, NOT for
//! cross-pod arbitration.
//!
//! ## Fence-bump-before-dispatch invariant (load-bearing)
//! [`ClaimMap::reassign`] bumps the fence BEFORE the caller dispatches the redo.
//! A superseded worker's engine still runs on the OLD fence, so its late
//! [`ClaimMap::set_status`] / journal writes fail the `expected_fencing_id`
//! guard and become no-ops — the single-writer guarantee that makes a redo safe
//! even during the brief abrupt-death double-compute window.
//!
//! ## Double-run trade-off
//! - **graceful drain** waits for the worker's `Released` ack (engine confirmed
//!   stopped) before reassigning → ~zero overlap.
//! - **abrupt death** reassigns on heartbeat-timeout → a brief possible
//!   double-compute, but SAFE via fencing + the engine's git-idempotency (late
//!   writes are fence-rejected; no journal corruption ever).
//!
//! Secret material is NEVER stored in a [`ClaimEntry`] (held by #138; only
//! re-dispatched on reassignment via the seam in `reassign_driver`).

use std::collections::HashMap;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

use crate::models::SessionStatus;

/// Controller-issued monotonic per-run fence. `i64` to stay wire-compatible
/// with `SessionDoc::fencing_token` and the engine driver's fence document.
pub type FencingId = i64;

/// The controller-authoritative session lifecycle state. Reuses the wire
/// vocabulary of [`SessionStatus`] so the label reflected to the goal issue
/// (#137) maps 1:1.
pub type ClaimStatus = SessionStatus;

/// `Pending`/`Validating`/`Running`/`Stopping` are active; `Stopped`/`Failed`
/// are terminal. Mirrors `distribution::health::ACTIVE_STATUSES`.
pub fn is_active(status: ClaimStatus) -> bool {
    matches!(
        status,
        SessionStatus::Pending
            | SessionStatus::Validating
            | SessionStatus::Running
            | SessionStatus::Stopping
    )
}

/// One claimed run, keyed in the map by `lease_key`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClaimEntry {
    /// Map key (`package_name` classic / `goal-<uuid>` goal), duplicated here.
    pub lease_key: String,
    pub session_id: bson::Uuid,
    /// `worker_id` of the assigned worker.
    pub owner_worker: String,
    /// Controller-authoritative lifecycle state.
    pub status: ClaimStatus,
    /// Current run fence; bumped on reassignment. Journaling-idempotency only.
    pub fencing_id: FencingId,
    /// Present for goal sessions.
    pub goal_id: Option<bson::Uuid>,
    pub claimed_at: Instant,
    pub updated_at: Instant,
}

/// Errors at the claim boundary. No Mongo variant (database-free).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ClaimError {
    /// A live claim for this `lease_key` is bound to a DIFFERENT session.
    /// Maps to `409 Conflict` at the API edge (replaces
    /// `PlacementError::AlreadyRunning`).
    #[error("lease key {0} already has a live claim")]
    AlreadyClaimed(String),
}

/// The in-memory claim authority. Cheap to clone is achieved by wrapping in an
/// `Arc` at the controller; the map itself is `Mutex`-guarded.
pub struct ClaimMap {
    inner: Mutex<HashMap<String, ClaimEntry>>,
    /// Strictly monotonic fence allocator. Never resets for the controller's
    /// lifetime, so ids are globally monotonic per incarnation. On controller
    /// restart it restarts at 1 — the rebuild-from-worker-self-reports step
    /// (#134) MUST seed this above any fence a still-live worker reports, so a
    /// post-restart redo always out-fences a survivor (see [`seed_fencing`]).
    fencing: AtomicI64,
}

impl Default for ClaimMap {
    fn default() -> Self {
        Self::new()
    }
}

impl ClaimMap {
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
            // Start so the first `next_fencing_id()` returns 1.
            fencing: AtomicI64::new(0),
        }
    }

    /// Allocate the next strictly-increasing fence id (start at 1).
    pub fn next_fencing_id(&self) -> FencingId {
        self.fencing.fetch_add(1, Ordering::SeqCst) + 1
    }

    /// Seed the fence allocator above `floor` (rebuild-after-restart). The next
    /// allocation will exceed `floor`, so a redo out-fences any survivor that
    /// reported a fence up to `floor`.
    pub fn seed_fencing(&self, floor: FencingId) {
        self.fencing.fetch_max(floor, Ordering::SeqCst);
    }

    /// THE mutual-exclusion gate (single locked check-and-insert), replacing the
    /// Mongo `_id` unique index + `transition_guarded` CAS:
    /// - a live claim for a DIFFERENT active session -> `AlreadyClaimed` (409);
    /// - a claim for the SAME `session_id` -> returned unchanged (idempotent
    ///   replay, no new fence);
    /// - otherwise allocate a fence and insert a fresh `Pending` entry.
    pub fn claim(
        &self,
        lease_key: &str,
        session_id: bson::Uuid,
        goal_id: Option<bson::Uuid>,
        owner_worker: &str,
    ) -> Result<ClaimEntry, ClaimError> {
        let mut map = self.inner.lock().expect("claim map poisoned");
        if let Some(existing) = map.get(lease_key) {
            if existing.session_id == session_id {
                return Ok(existing.clone());
            }
            if is_active(existing.status) {
                tracing::info!(
                    lease_key,
                    held_by = %existing.session_id,
                    requested_by = %session_id,
                    "claim conflicts with a live claim"
                );
                return Err(ClaimError::AlreadyClaimed(lease_key.to_string()));
            }
            // The existing entry is terminal — overwrite (re-claim after stop).
        }
        let now = Instant::now();
        let entry = ClaimEntry {
            lease_key: lease_key.to_string(),
            session_id,
            owner_worker: owner_worker.to_string(),
            status: ClaimStatus::Pending,
            fencing_id: self.next_fencing_id(),
            goal_id,
            claimed_at: now,
            updated_at: now,
        };
        tracing::info!(
            lease_key,
            session_id = %session_id,
            owner_worker,
            fencing_id = entry.fencing_id,
            "claim acquired"
        );
        map.insert(lease_key.to_string(), entry.clone());
        Ok(entry)
    }

    /// The in-memory CAS replacing `SessionRepo::transition_guarded`: apply the
    /// status change ONLY if the entry exists, its `fencing_id` matches
    /// `expected_fencing_id`, and its current status is in `from`. `Ok(false)`
    /// on a guard miss (caller re-reads + converges); the `expected_fencing_id`
    /// guard makes a superseded worker's late write a no-op.
    pub fn set_status(
        &self,
        lease_key: &str,
        expected_fencing_id: FencingId,
        from: &[ClaimStatus],
        to: ClaimStatus,
    ) -> bool {
        let mut map = self.inner.lock().expect("claim map poisoned");
        match map.get_mut(lease_key) {
            Some(entry)
                if entry.fencing_id == expected_fencing_id && from.contains(&entry.status) =>
            {
                entry.status = to;
                entry.updated_at = Instant::now();
                true
            }
            _ => false,
        }
    }

    /// Remove the entry ONLY if its `fencing_id` matches (equality-pinned, like
    /// `LeaseStore::release`). Idempotent (already-gone is `false`).
    pub fn release(&self, lease_key: &str, fencing_id: FencingId) -> bool {
        let mut map = self.inner.lock().expect("claim map poisoned");
        match map.get(lease_key) {
            Some(entry) if entry.fencing_id == fencing_id => {
                map.remove(lease_key);
                tracing::info!(lease_key, fencing_id, "claim released");
                true
            }
            _ => false,
        }
    }

    pub fn get(&self, lease_key: &str) -> Option<ClaimEntry> {
        self.inner
            .lock()
            .expect("claim map poisoned")
            .get(lease_key)
            .cloned()
    }

    pub fn list(&self) -> Vec<ClaimEntry> {
        self.inner
            .lock()
            .expect("claim map poisoned")
            .values()
            .cloned()
            .collect()
    }

    /// Every ACTIVE entry owned by `worker_id` (used by the reassignment driver
    /// to enumerate a dead/draining worker's in-flight work).
    pub fn owned_by(&self, worker_id: &str) -> Vec<ClaimEntry> {
        self.inner
            .lock()
            .expect("claim map poisoned")
            .values()
            .filter(|e| e.owner_worker == worker_id && is_active(e.status))
            .cloned()
            .collect()
    }

    /// Count of active entries owned by `worker_id` — the controller-authoritative
    /// load used by placement (immediate, unlike heartbeat-lagged reports).
    pub fn active_load(&self, worker_id: &str) -> u64 {
        self.inner
            .lock()
            .expect("claim map poisoned")
            .values()
            .filter(|e| e.owner_worker == worker_id && is_active(e.status))
            .count() as u64
    }

    /// THE reassignment primitive both the abrupt and graceful paths call.
    /// Under the lock, for the entry at `lease_key`: (a) bump the fence FIRST,
    /// (b) set the new owner, (c) reset status to `Pending` (re-dispatchable),
    /// (d) touch `updated_at`. Returns the updated clone, or `None` if absent.
    /// The fence-first ordering is load-bearing — see the module doc.
    pub fn reassign(&self, lease_key: &str, new_owner_worker: &str) -> Option<ClaimEntry> {
        let new_fence = self.next_fencing_id();
        let mut map = self.inner.lock().expect("claim map poisoned");
        let entry = map.get_mut(lease_key)?;
        entry.fencing_id = new_fence;
        entry.owner_worker = new_owner_worker.to_string();
        entry.status = ClaimStatus::Pending;
        entry.updated_at = Instant::now();
        tracing::warn!(
            lease_key,
            session_id = %entry.session_id,
            new_owner_worker,
            fencing_id = new_fence,
            "claim reassigned (fence bumped before redo dispatch)"
        );
        Some(entry.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid() -> bson::Uuid {
        bson::Uuid::new()
    }

    #[test]
    fn claim_then_conflict_for_different_session() {
        let m = ClaimMap::new();
        let s1 = uuid();
        m.claim("pkg", s1, None, "w1").unwrap();
        let err = m.claim("pkg", uuid(), None, "w1").unwrap_err();
        assert_eq!(err, ClaimError::AlreadyClaimed("pkg".to_string()));
    }

    #[test]
    fn claim_idempotent_for_same_session() {
        let m = ClaimMap::new();
        let s1 = uuid();
        let first = m.claim("pkg", s1, None, "w1").unwrap();
        let again = m.claim("pkg", s1, None, "w1").unwrap();
        assert_eq!(first, again);
        assert_eq!(first.fencing_id, again.fencing_id, "no new fence on replay");
    }

    #[test]
    fn claim_succeeds_after_release() {
        let m = ClaimMap::new();
        let s1 = uuid();
        let e1 = m.claim("pkg", s1, None, "w1").unwrap();
        assert!(m.release("pkg", e1.fencing_id));
        // A different session can now claim the freed key.
        assert!(m.claim("pkg", uuid(), None, "w1").is_ok());
    }

    #[test]
    fn fencing_id_is_strictly_monotonic() {
        let m = ClaimMap::new();
        let mut last = 0;
        for i in 0..5 {
            let e = m.claim(&format!("pkg-{i}"), uuid(), None, "w1").unwrap();
            assert!(e.fencing_id > last, "fence must strictly increase");
            last = e.fencing_id;
        }
        let r = m.reassign("pkg-0", "w2").unwrap();
        assert!(r.fencing_id > last);
    }

    #[test]
    fn set_status_guard_miss_on_stale_fence() {
        let m = ClaimMap::new();
        let e = m.claim("pkg", uuid(), None, "w1").unwrap();
        let stale_fence = e.fencing_id;
        m.reassign("pkg", "w2").unwrap(); // bumps the fence
                                          // A superseded worker writing with the pre-reassign fence is rejected.
        assert!(!m.set_status(
            "pkg",
            stale_fence,
            &[ClaimStatus::Pending],
            ClaimStatus::Running
        ));
    }

    #[test]
    fn set_status_respects_from_set() {
        let m = ClaimMap::new();
        let e = m.claim("pkg", uuid(), None, "w1").unwrap();
        // Wrong `from` set -> miss.
        assert!(!m.set_status(
            "pkg",
            e.fencing_id,
            &[ClaimStatus::Running],
            ClaimStatus::Stopping
        ));
        // Correct `from` -> apply.
        assert!(m.set_status(
            "pkg",
            e.fencing_id,
            &[ClaimStatus::Pending],
            ClaimStatus::Validating
        ));
        assert_eq!(m.get("pkg").unwrap().status, ClaimStatus::Validating);
    }

    #[test]
    fn release_is_equality_pinned() {
        let m = ClaimMap::new();
        let e = m.claim("pkg", uuid(), None, "w1").unwrap();
        assert!(
            !m.release("pkg", e.fencing_id + 999),
            "stale fence removes nothing"
        );
        assert!(m.get("pkg").is_some());
    }

    #[test]
    fn reassign_bumps_fence_changes_owner_resets_status() {
        let m = ClaimMap::new();
        let e = m.claim("pkg", uuid(), None, "w1").unwrap();
        m.set_status(
            "pkg",
            e.fencing_id,
            &[ClaimStatus::Pending],
            ClaimStatus::Running,
        );
        let r = m.reassign("pkg", "w2").unwrap();
        assert!(r.fencing_id > e.fencing_id);
        assert_eq!(r.owner_worker, "w2");
        assert_eq!(r.status, ClaimStatus::Pending);
    }

    #[test]
    fn owned_by_lists_only_active_entries_for_worker() {
        let m = ClaimMap::new();
        let a = m.claim("a", uuid(), None, "w1").unwrap();
        m.claim("b", uuid(), None, "w1").unwrap();
        m.claim("c", uuid(), None, "w2").unwrap();
        // Make `a` terminal.
        m.set_status(
            "a",
            a.fencing_id,
            &[ClaimStatus::Pending],
            ClaimStatus::Stopped,
        );
        let owned: Vec<String> = m.owned_by("w1").into_iter().map(|e| e.lease_key).collect();
        assert_eq!(owned, vec!["b".to_string()]);
        assert_eq!(m.active_load("w1"), 1);
        assert_eq!(m.active_load("w2"), 1);
    }

    #[test]
    fn seed_fencing_raises_floor_for_post_restart_redo() {
        let m = ClaimMap::new();
        m.seed_fencing(100);
        let e = m.claim("pkg", uuid(), None, "w1").unwrap();
        assert!(
            e.fencing_id > 100,
            "next fence must exceed the seeded floor"
        );
    }
}
