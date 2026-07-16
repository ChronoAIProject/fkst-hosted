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

pub mod announce;
pub mod automerge;
pub mod desired;
pub mod execute;
mod execute_comments;
pub mod hashing;
// Pure three-tier authorization for on-demand session-log downloads (author /
// per-issue `### Log Access Allowlist` allow-list / global admins). No I/O; consumed by the
// identity-gated `/api/v1/logs/{session_id}` endpoint.
pub mod log_authz;
mod loops;
pub mod pending;
pub mod reachability;
pub mod registry;
pub mod repo;
pub mod retire;
pub mod seed_issue;
pub mod templates;
pub mod work_ack;
pub mod work_labels;

// Shared executor/loop test fixtures (the recording GitHub transport, the ctx
// builder, and a re-export of the shared session-backend fake). Reconcile-wide so
// both the executor tests and the loop tests can build a `ReconcileCtx`.
#[cfg(test)]
pub(crate) mod execute_test_support;

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
pub use templates::ensure_issue_templates;

/// The label the reconciler latches onto a trigger issue whose body fails to parse
/// (or whose package refs are unreachable). The presence of this label on an issue
/// is the "already flagged" signal the planner reads to avoid re-commenting; its
/// removal ([`ReconcileAction::ClearInvalid`]) is how a fixed issue is un-flagged.
pub const SUBSTRATE_INVALID_LABEL: &str = "fkst-substrate-invalid";

/// The DURABLE label the reconciler latches onto a trigger issue once it has posted
/// the one-time session-registration announcement ([`ReconcileAction::AnnounceSession`]).
/// Because it lives on the issue (not in process memory), a control-plane restart
/// re-reads it and never re-posts the announcement. Config changes to an already
/// announced trigger do NOT re-announce in v1 (the label stays) — an accepted
/// limitation; there is no clear/removal path (unlike [`SUBSTRATE_INVALID_LABEL`]).
pub const SUBSTRATE_ANNOUNCED_LABEL: &str = "fkst-substrate-active";

/// The DURABLE latch label the reconciler adds to a WORK issue (one carrying a
/// session's `work_label`) once it has posted the one-time "picked up" acknowledgment
/// ([`work_ack::ack_open_work_issues`]). A work issue is otherwise often silent from
/// GitHub's side — the pod's output (e.g. the codex-triage package) lands elsewhere —
/// so the author has no signal it was claimed. Mirrors [`SUBSTRATE_ANNOUNCED_LABEL`]:
/// because the latch lives on the issue (not process memory), a control-plane restart
/// re-reads it and never re-acks. There is no clear/removal path (like the announce
/// latch) — an acknowledged work issue stays acknowledged for its lifetime.
pub const WORK_PICKED_UP_LABEL: &str = "fkst-picked-up";

/// The DURABLE latch label the reconciler adds to a WORK issue when its session is
/// RETIRED — i.e. the session's trigger issue was closed, so the orphan pod is killed
/// ([`desired::plan_repo`]'s orphan branch emits [`ReconcileAction::RetireWorkIssues`]).
/// The still-open work issue is left OPEN but is no longer worked, so the executor
/// comments "session retired, no longer worked", latches THIS label, and drops the now
/// stale [`WORK_PICKED_UP_LABEL`]. Because the latch lives on the issue (not process
/// memory) it is read back each reconcile: an already-retired issue is skipped, so the
/// ~60s the orphan pod lingers before deletion never re-notifies. Mirrors the one-way
/// [`SUBSTRATE_ANNOUNCED_LABEL`]/[`WORK_PICKED_UP_LABEL`] latches — there is no
/// clear/removal path (unlike [`SUBSTRATE_INVALID_LABEL`]).
pub const SUBSTRATE_RETIRED_LABEL: &str = "fkst-session-retired";

/// The label the session-health scrape ([`crate::k8s::health_scrape`]) latches onto
/// a trigger issue while its pod looks degraded. Unlike the parse-time invalid latch,
/// this one is package-AGNOSTIC: it is derived only from the two signals every
/// package shares (the K8s pod status and the framework's OWN structured log
/// severity), and it is CLEARED (comment + label removal) the moment the pod reads
/// healthy again — so the label's presence is the "already warned" dedupe signal
/// that keeps the scrape from re-posting every cycle. Mirrors the clearable
/// [`SUBSTRATE_INVALID_LABEL`] rather than the one-way announce/pick-up latches.
pub const SUBSTRATE_DEGRADED_LABEL: &str = "fkst-degraded";

/// The DURABLE latch label the reconciler adds to a trigger issue when it REJECTS a
/// config edit (config is immutable once a session exists —
/// [`desired::plan_repo`] emits [`ReconcileAction::RejectConfigChange`]). Its presence
/// is the "already told them" dedupe signal that keeps the reconciler from re-posting
/// the rejection every cycle while the edited (and ignored) config sits on the issue.
/// Mirrors the one-way announce/pick-up latches — there is no clear/removal path
/// (unlike [`SUBSTRATE_INVALID_LABEL`]): the only way to change config is to close the
/// session and open a new one.
pub const SUBSTRATE_CONFIG_REJECTED_LABEL: &str = "fkst-config-rejected";

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

/// TTL after which a repo's issue templates are re-checked against GitHub even at
/// the same recorded version (~6h). Bounds how long a manually-reverted template
/// lingers before the ensure repairs it, without paying a round-trip every
/// reconcile (which fire on every repo-touching webhook + sweep + full-resync).
pub const ENSURED_TEMPLATES_TTL: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// What the issue-template ensure last recorded for a repo: the version it
/// confirmed present and WHEN it confirmed it (monotonic [`std::time::Instant`]).
#[derive(Debug, Clone)]
pub struct EnsuredMark {
    pub version: u32,
    pub checked_at: std::time::Instant,
}

/// Per-repo issue-template ensure gate: at most one GitHub round-trip per repo
/// per `(version, TTL)`. Shared (cheap `Arc<Mutex>`) across the reconciler loops,
/// mirroring [`ActiveRepos`]. Re-checked only when the bundled version is newer
/// than the recorded one, or the record is older than [`ENSURED_TEMPLATES_TTL`].
pub type EnsuredTemplates =
    std::sync::Arc<std::sync::Mutex<std::collections::HashMap<RepoKey, EnsuredMark>>>;

/// A fresh, empty [`EnsuredTemplates`] for the reconciler to share across loops.
pub fn new_ensured_templates() -> EnsuredTemplates {
    std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()))
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
