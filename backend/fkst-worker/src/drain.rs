//! Graceful worker drain on SIGTERM / preStop (issue #140a).
//!
//! When the pod is told to terminate, the worker must NOT silently drop the
//! sessions it is driving: the controller would otherwise have to wait out a
//! heartbeat-expiry before it could reassign them. Instead the worker drains
//! GRACEFULLY:
//!
//! 1. flip its own lifecycle to `Draining` (the pull loop reads this gate and
//!    stops requesting new work — see `run.rs`);
//! 2. tell the controller it is `Draining` (one best-effort message carrying the
//!    in-flight session ids);
//! 3. checkpoint + cleanly stop EACH in-flight session, emitting a `Released`
//!    per session so the controller can reassign without a double-run.
//!
//! All of this runs within a bounded `grace` deadline so a hung engine can never
//! wedge the shutdown past the pod's `terminationGracePeriodSeconds` (owned by
//! #144).
//!
//! ## Reuse, not re-implementation
//!
//! Post-#151 the running engines + per-session journalers are owned by the
//! supervise tasks (`engine::supervise::supervise_session`), NOT by the agent's
//! registry. The drain therefore drives each session's EXISTING stop path:
//! [`WorkerAgent::stop_session`] flips the supervise loop's stop signal and
//! awaits it, and that loop's `stop_and_release` already journals `Stopping` +
//! `finish(Stopped)` (the checkpoint — a forced GitHub flush) and sends the
//! `Released` before exiting. So calling `stop_session` per live session IS the
//! checkpoint-and-handoff; this module orchestrates the fan-out + the deadline
//! and never touches engine `Child` / pgid / journaler handles directly.
//!
//! No secrets exist in this layer, so nothing here logs one.

use std::sync::Arc;
use std::time::Duration;

use tokio::task::JoinSet;

use crate::agent::WorkerAgent;

/// The drain's phase, used purely for clarity in logs / reasoning. The drain is
/// linear (each phase runs once, in order); the enum documents the sequence
/// rather than driving a state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrainState {
    /// Pre-drain: still accepting + running work.
    Active,
    /// Drain begun: the pull gate is closed and the controller has been told.
    Draining,
    /// Stopping each in-flight session (the per-session checkpoint flush).
    FlushingCheckpoints,
    /// All stops issued; awaiting their `Released` acks (bounded by `grace`).
    AwaitingAcks,
    /// Drain complete (cleanly or on the deadline).
    Terminated,
}

/// The result of a drain pass. `released` is how many sessions completed their
/// stop (checkpoint + `Released`) before the deadline; `total` is how many were
/// in flight when the drain started; `timed_out` is whether the overall `grace`
/// deadline was hit before every session finished.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DrainOutcome {
    /// Sessions whose stop completed (checkpoint flushed + `Released` sent).
    pub released: usize,
    /// Sessions in flight when the drain began.
    pub total: usize,
    /// Whether the `grace` deadline elapsed before all stops completed.
    pub timed_out: bool,
}

/// Drive a graceful drain to completion (or the `grace` deadline), returning the
/// outcome. Steps:
///
/// 1. flip the worker's lifecycle to `Draining` (idempotent; closes the pull
///    gate the pull loop reads);
/// 2. snapshot the in-flight session ids — an empty snapshot is a fast, clean
///    exit (one best-effort `Draining([], true)` so the controller sees the
///    intent, then return);
/// 3. send ONE `Draining` message carrying the ids (`checkpoint_done = true`:
///    each session's `stop_session` forces the journal `finish` flush before its
///    `Released`, and journaling is never load-bearing, so a flush failure is
///    swallowed and the session is still released + reassignable);
/// 4. fan the per-session stops out concurrently under a single `grace`
///    deadline, counting how many complete before it elapses.
///
/// `grace` MUST be smaller than the pod's `terminationGracePeriodSeconds` so the
/// kubelet's own SIGKILL never races this drain (the margin is owned by #144).
pub async fn run_drain(agent: &Arc<WorkerAgent>, grace: Duration) -> DrainOutcome {
    // (a) Close the pull gate. Idempotent — the run loop may have already called
    // this before invoking the drain; a second flip is a no-op.
    agent.begin_drain();

    // (b) Snapshot the sessions to drain. Taken once here so the `Draining`
    // message and the stop fan-out operate on the SAME set (a session that
    // finishes on its own mid-drain simply makes its `stop_session` a no-op).
    let ids = agent.running_session_ids();
    let total = ids.len();

    if ids.is_empty() {
        // Nothing in flight: still announce the drain so the controller marks the
        // worker draining promptly, then return clean. Best-effort.
        announce_draining(agent, &[]).await;
        tracing::info!(
            worker_id = %agent.worker_id(),
            state = ?DrainState::Terminated,
            released = 0,
            total = 0,
            timed_out = false,
            "drain complete: no in-flight sessions"
        );
        return DrainOutcome {
            released: 0,
            total: 0,
            timed_out: false,
        };
    }

    tracing::info!(
        worker_id = %agent.worker_id(),
        state = ?DrainState::Draining,
        sessions = total,
        grace_secs = grace.as_secs(),
        "drain begun: closing pull gate and notifying controller"
    );

    // (c) One `Draining` message for the whole set. `checkpoint_done = true` is
    // honest: each `stop_session` forces the journal flush before its `Released`.
    announce_draining(agent, &ids).await;

    // (d) Stop every session concurrently under the overall `grace` deadline.
    let outcome = stop_all_within_grace(agent, ids, total, grace).await;

    tracing::info!(
        worker_id = %agent.worker_id(),
        state = ?DrainState::Terminated,
        released = outcome.released,
        total = outcome.total,
        timed_out = outcome.timed_out,
        "drain complete"
    );
    outcome
}

/// Send the single best-effort `Draining` message. A delivery failure (the
/// controller may already be gone) is logged at debug and swallowed — the drain
/// proceeds to stop the sessions regardless, since the `Released` per session is
/// the load-bearing reassignment signal.
async fn announce_draining(agent: &Arc<WorkerAgent>, ids: &[String]) {
    if let Err(error) = agent.send_draining(ids, true).await {
        tracing::debug!(
            error = %error,
            "Draining message not delivered (controller may be gone); continuing"
        );
    }
}

/// Fan the per-session stops out as concurrent tasks and await them under one
/// `grace` deadline. Each task clones the `Arc<WorkerAgent>` and calls
/// [`WorkerAgent::stop_session`], which flips the supervise loop's stop signal,
/// awaits its checkpoint-flush + `Released`, and removes the registry entry.
///
/// On the deadline the still-running tasks are dropped (their `JoinSet` is
/// dropped, which aborts them) and the outcome carries `timed_out = true`; the
/// function ALWAYS returns rather than hanging on a wedged engine.
async fn stop_all_within_grace(
    agent: &Arc<WorkerAgent>,
    ids: Vec<String>,
    total: usize,
    grace: Duration,
) -> DrainOutcome {
    tracing::debug!(
        state = ?DrainState::FlushingCheckpoints,
        sessions = total,
        "stopping in-flight sessions"
    );

    let mut set: JoinSet<()> = JoinSet::new();
    for id in ids {
        let agent = agent.clone();
        set.spawn(async move {
            agent.stop_session(&id).await;
        });
    }

    tracing::debug!(state = ?DrainState::AwaitingAcks, "awaiting per-session Released");

    let mut released = 0usize;
    // `timeout` over the whole join: each completed task is one drained session.
    // On the deadline we stop counting and report the timeout; dropping `set`
    // aborts whatever is still mid-stop.
    let drained = tokio::time::timeout(grace, async {
        while let Some(joined) = set.join_next().await {
            match joined {
                Ok(()) => released += 1,
                Err(error) => {
                    // A stop task panicked. `stop_session` itself never panics
                    // (it logs + best-effort releases on a join failure), so this
                    // is only reachable on an abort/panic; log and keep draining.
                    tracing::warn!(error = %error, "a session stop task did not complete cleanly");
                }
            }
        }
    })
    .await;

    let timed_out = drained.is_err();
    if timed_out {
        tracing::warn!(
            released,
            total,
            grace_secs = grace.as_secs(),
            "drain grace deadline hit before all sessions stopped; aborting the rest"
        );
    }
    // Dropping `set` here aborts any tasks still running past the deadline so the
    // drain never leaves orphaned stop tasks behind.
    drop(set);

    DrainOutcome {
        released,
        total,
        timed_out,
    }
}

#[cfg(test)]
#[path = "drain_tests.rs"]
mod tests;
