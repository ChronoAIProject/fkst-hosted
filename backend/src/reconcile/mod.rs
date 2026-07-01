//! Model B reconciler (issue #359 §4, PR5a core + PR5b wiring).
//!
//! The reconciler drives the whole Model B session fleet toward the state the
//! GitHub trigger issues declare. It is split into a PURE half and an EFFECTFUL
//! half so the decision logic is exhaustively unit-testable without a cluster:
//!
//! - PURE ([`desired`] + [`registry`], PR5a): the desired-state types, the
//!   event→action planner ([`desired::plan_repo`]), and the trigger-issue →
//!   registration parse. No Kubernetes/GitHub I/O.
//! - EFFECTFUL (PR5b): the reachability pre-flight ([`reachability`]), the action
//!   executor ([`execute`]), the per-repo driver ([`repo::reconcile_repo`]), and
//!   the queue + sweep/full-resync loops below.
//!
//! ADDITIVE + GATED: nothing spawns the loops unless `FKST_POD_DISPATCH` is on, and
//! the webhook is NOT rewired to enqueue here yet — that is the PR6 flip. Model A
//! (the Job launcher + webhook trigger) is untouched.

pub mod desired;
pub mod execute;
mod loops;
pub mod pending;
pub mod reachability;
pub mod registry;
pub mod repo;

use tokio::sync::mpsc;

use crate::models::RepoRef;

pub use desired::{
    config_hash, plan_repo, KillReason, LivePod, PodLiveness, ReconcileAction, SessionDef,
    SessionRegistration,
};
pub use execute::{execute, ReconcileCtx};
pub use loops::{run_full_resync_loop, run_reconcile_loop, run_sweep_loop};
pub use registry::parse_registration;
pub use repo::reconcile_repo;

/// The label the reconciler latches onto a trigger issue whose body fails to parse
/// (or whose package refs are unreachable). The presence of this label on an issue
/// is the "already flagged" signal the planner reads to avoid re-commenting; its
/// removal ([`ReconcileAction::ClearInvalid`]) is how a fixed issue is un-flagged.
pub const SUBSTRATE_INVALID_LABEL: &str = "fkst-substrate-invalid";

/// The identity of one repository to reconcile: `(installation_id, repo)`. The
/// installation id scopes the GitHub App token; the repo names the work.
pub type RepoKey = (i64, RepoRef);

/// The set of repos currently carrying ≥1 open trigger-issue registration, shared
/// (cheap `Arc<Mutex>`) between the per-repo reconcile that MAINTAINS it and the
/// sweep that re-enqueues every member each tick. It closes the first-spawn gap:
/// without it, a repo with a registration but no pod yet is re-reconciled ONLY by
/// the slow full-resync, so a just-labelled work issue would stall for up to
/// `pod_full_resync_interval_secs` whenever the triggering webhook raced GitHub's
/// search index (a consistently-lagging index in practice). With the repo in this
/// set the 30s sweep re-checks its pending work, so the spawn lands within a sweep.
pub type ActiveRepos = std::sync::Arc<std::sync::Mutex<std::collections::HashSet<RepoKey>>>;

/// A fresh, empty [`ActiveRepos`] for the reconciler to share across its loops.
pub fn new_active_repos() -> ActiveRepos {
    std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashSet::new()))
}

/// A clonable handle for enqueuing repositories onto the reconcile queue. The
/// webhook (PR6), the sweep, and the full-resync all push `RepoKey`s through this;
/// the single [`run_reconcile_loop`] consumer drains + dedups them.
#[derive(Clone)]
pub struct ReconcileHandle {
    tx: mpsc::Sender<RepoKey>,
}

impl ReconcileHandle {
    /// Enqueue a repo for reconciliation. BEST-EFFORT: a full queue drops the
    /// enqueue with a warning rather than blocking the caller (the periodic sweep +
    /// full-resync re-add it, so a dropped enqueue is at worst a bounded delay).
    pub fn enqueue(&self, key: RepoKey) {
        match self.tx.try_send(key) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(dropped)) => {
                tracing::warn!(
                    installation = dropped.0,
                    owner = %dropped.1.owner,
                    name = %dropped.1.name,
                    "reconcile queue full; dropping enqueue (next sweep re-adds it)"
                );
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                tracing::warn!("reconcile queue closed; enqueue dropped (loop stopped)");
            }
        }
    }
}

/// Create the reconcile queue: a [`ReconcileHandle`] for producers + the receiver
/// the single [`run_reconcile_loop`] consumer owns. `capacity` bounds the queue;
/// an overflow is dropped by [`ReconcileHandle::enqueue`] (the sweep re-adds it).
pub fn reconcile_channel(capacity: usize) -> (ReconcileHandle, mpsc::Receiver<RepoKey>) {
    let (tx, rx) = mpsc::channel(capacity.max(1));
    (ReconcileHandle { tx }, rx)
}

#[cfg(test)]
mod mod_tests {
    use super::*;

    fn repo() -> RepoRef {
        RepoRef {
            owner: "acme".to_string(),
            name: "site".to_string(),
        }
    }

    #[tokio::test]
    async fn enqueue_delivers_onto_the_channel() {
        let (handle, mut rx) = reconcile_channel(4);
        handle.enqueue((42, repo()));
        let got = rx.recv().await.expect("one key");
        assert_eq!(got, (42, repo()));
    }

    #[tokio::test]
    async fn enqueue_drops_when_the_queue_is_full_without_blocking() {
        // Capacity 1: the first send fills it, the second overflows and is dropped
        // (best-effort) rather than blocking the producer.
        let (handle, mut rx) = reconcile_channel(1);
        handle.enqueue((1, repo()));
        handle.enqueue((2, repo())); // dropped, must not block
        let first = rx.recv().await.expect("first");
        assert_eq!(first.0, 1);
        // Nothing else is buffered (the overflow was dropped).
        assert!(rx.try_recv().is_err(), "overflow was dropped");
    }

    #[tokio::test]
    async fn enqueue_after_receiver_dropped_is_a_noop() {
        let (handle, rx) = reconcile_channel(4);
        drop(rx);
        // Must not panic — a closed channel is logged + dropped.
        handle.enqueue((7, repo()));
    }
}
