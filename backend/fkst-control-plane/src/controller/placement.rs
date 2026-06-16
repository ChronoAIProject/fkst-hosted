//! Placement: which live worker runs a new session, via the controller's
//! in-memory claim authority instead of Mongo (#135).
//!
//! `select_worker` is a verbatim-in-spirit port of
//! `distribution::distributor::select_pod` (same least-loaded + lowest-id
//! tie-break + cap semantics), against the controller's worker registry rather
//! than a Mongo aggregation. `place` is the orchestration: the claim is THE
//! mutual-exclusion gate; an idempotent replay returns the existing placement
//! without re-selecting, and an exhausted fleet leaves the session pending
//! (retried) rather than erroring.

use crate::controller::claims::{is_active, ClaimError, ClaimMap, FencingId};
use crate::engine::is_valid_name;

/// A live worker and its current active-session load (mirror of `PodLoad`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkerLoad {
    pub worker_id: String,
    pub active_sessions: u64,
}

/// Outcome of placing a session: the worker + fence that now own the run
/// (mirror of `distribution::distributor::Placement`, with `worker_id`/
/// `fencing_id` replacing `pod_id`/`fencing_token`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Placement {
    pub session_id: bson::Uuid,
    pub lease_key: String,
    pub worker_id: String,
    pub fencing_id: FencingId,
}

impl Placement {
    fn from_entry(entry: &crate::controller::claims::ClaimEntry) -> Self {
        Placement {
            session_id: entry.session_id,
            lease_key: entry.lease_key.clone(),
            worker_id: entry.owner_worker.clone(),
            fencing_id: entry.fencing_id,
        }
    }
}

/// Errors surfaced by placement. `AlreadyRunning`/`NoCapacity` are expected
/// operational outcomes (409 / stay-pending); the rest are validation. No
/// Mongo/Lease variants (database-free).
#[derive(Debug, thiserror::Error)]
pub enum PlacementError {
    /// A live claim already exists for the lease key, bound to a different
    /// session. The Sessions API maps this to `409 Conflict`.
    #[error("lease key {0} already has a live claim")]
    AlreadyRunning(String),
    /// No live worker has capacity (retriable; the session stays pending).
    #[error("no live worker has capacity")]
    NoCapacity,
    /// The lease key failed re-validation (defense in depth).
    #[error("invalid lease key")]
    InvalidLeaseKey,
    /// A non-conflict claim-layer failure (forward-compat; none today).
    #[error(transparent)]
    Claim(#[from] ClaimError),
}

/// Pure, deterministic worker selection: minimum `active_sessions`, ties broken
/// by lowest `worker_id`. `max_load > 0` discards workers at/over cap;
/// `max_load == 0` is uncapped. `None` when empty or all at cap. Identical
/// semantics to `select_pod`.
pub fn select_worker(workers: &[WorkerLoad], max_load: u64) -> Option<&WorkerLoad> {
    workers
        .iter()
        .filter(|w| max_load == 0 || w.active_sessions < max_load)
        .min_by(|a, b| {
            (a.active_sessions, a.worker_id.as_str())
                .cmp(&(b.active_sessions, b.worker_id.as_str()))
        })
}

/// Place a session on the least-loaded live worker through the claim authority.
///
/// Validate the lease key, then take the idempotent/conflict fast path (an
/// existing active claim for THIS session returns its placement; a different
/// session's active claim is the conflict), select a worker (`NoCapacity` if
/// none), and finally `claim` (the atomic gate) and return the placement.
/// `claim` resolves any concurrent race — the first inserter wins; the loser
/// sees `AlreadyClaimed`.
pub fn place(
    claims: &ClaimMap,
    worker_loads: &[WorkerLoad],
    max_load: u64,
    lease_key: &str,
    session_id: bson::Uuid,
    goal_id: Option<bson::Uuid>,
) -> Result<Placement, PlacementError> {
    if !is_valid_name(lease_key) {
        tracing::warn!(
            lease_key_bytes = lease_key.len(),
            "placement rejected: invalid lease key"
        );
        return Err(PlacementError::InvalidLeaseKey);
    }

    // Idempotent / conflict fast path — return an existing placement even when
    // the fleet is now at capacity (mirrors Distributor::check_live_lease).
    if let Some(existing) = claims.get(lease_key) {
        if is_active(existing.status) {
            if existing.session_id == session_id {
                return Ok(Placement::from_entry(&existing));
            }
            return Err(PlacementError::AlreadyRunning(lease_key.to_string()));
        }
    }

    let Some(chosen) = select_worker(worker_loads, max_load) else {
        tracing::warn!(
            lease_key,
            session = %session_id,
            workers = worker_loads.len(),
            max_load,
            "placement found no live worker with capacity"
        );
        return Err(PlacementError::NoCapacity);
    };

    match claims.claim(lease_key, session_id, goal_id, &chosen.worker_id) {
        Ok(entry) => {
            tracing::info!(
                session_id = %session_id,
                lease_key,
                worker_id = %entry.owner_worker,
                fencing_id = entry.fencing_id,
                "placement assigned"
            );
            Ok(Placement::from_entry(&entry))
        }
        Err(ClaimError::AlreadyClaimed(key)) => Err(PlacementError::AlreadyRunning(key)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(worker_id: &str, active_sessions: u64) -> WorkerLoad {
        WorkerLoad {
            worker_id: worker_id.to_string(),
            active_sessions,
        }
    }

    // --- select_worker: the ported select_pod tests --------------------------

    #[test]
    fn empty_input_selects_nothing() {
        assert_eq!(select_worker(&[], 0), None);
        assert_eq!(select_worker(&[], 5), None);
    }

    #[test]
    fn single_worker_is_selected() {
        let ws = [w("w-a", 7)];
        assert_eq!(select_worker(&ws, 0), Some(&ws[0]));
    }

    #[test]
    fn least_loaded_worker_wins() {
        let ws = [w("w-a", 2), w("w-b", 0), w("w-c", 1)];
        assert_eq!(
            select_worker(&ws, 0).map(|x| x.worker_id.as_str()),
            Some("w-b")
        );
    }

    #[test]
    fn load_tie_breaks_by_lowest_worker_id() {
        let ws = [w("w-z", 1), w("w-b", 1), w("w-m", 1)];
        assert_eq!(
            select_worker(&ws, 0).map(|x| x.worker_id.as_str()),
            Some("w-b")
        );
    }

    #[test]
    fn workers_at_cap_are_discarded() {
        let ws = [w("w-a", 3), w("w-b", 2), w("w-c", 3)];
        assert_eq!(
            select_worker(&ws, 3).map(|x| x.worker_id.as_str()),
            Some("w-b")
        );
    }

    #[test]
    fn all_at_cap_selects_nothing() {
        let ws = [w("w-a", 3), w("w-b", 4)];
        assert_eq!(select_worker(&ws, 3), None);
    }

    #[test]
    fn cap_zero_means_uncapped() {
        let ws = [w("w-a", u64::MAX), w("w-b", u64::MAX - 1)];
        assert_eq!(
            select_worker(&ws, 0).map(|x| x.worker_id.as_str()),
            Some("w-b")
        );
    }

    // --- place orchestration -------------------------------------------------

    #[test]
    fn place_returns_no_capacity_when_all_at_cap() {
        let claims = ClaimMap::new();
        let loads = [w("w-a", 3)];
        let err = place(&claims, &loads, 3, "pkg", bson::Uuid::new(), None).unwrap_err();
        assert!(matches!(err, PlacementError::NoCapacity));
    }

    #[test]
    fn place_is_idempotent_for_same_session() {
        let claims = ClaimMap::new();
        let loads = [w("w-a", 0)];
        let s = bson::Uuid::new();
        let first = place(&claims, &loads, 0, "pkg", s, None).unwrap();
        // Even with the fleet now "full", the same session replays its placement.
        let full = [w("w-a", 99)];
        let again = place(&claims, &full, 5, "pkg", s, None).unwrap();
        assert_eq!(first, again);
    }

    #[test]
    fn place_conflicts_for_different_session() {
        let claims = ClaimMap::new();
        let loads = [w("w-a", 0)];
        place(&claims, &loads, 0, "pkg", bson::Uuid::new(), None).unwrap();
        let err = place(&claims, &loads, 0, "pkg", bson::Uuid::new(), None).unwrap_err();
        assert!(matches!(err, PlacementError::AlreadyRunning(k) if k == "pkg"));
    }

    #[test]
    fn place_rejects_invalid_lease_key() {
        let claims = ClaimMap::new();
        let loads = [w("w-a", 0)];
        let err = place(&claims, &loads, 0, "bad key!", bson::Uuid::new(), None).unwrap_err();
        assert!(matches!(err, PlacementError::InvalidLeaseKey));
    }
}
