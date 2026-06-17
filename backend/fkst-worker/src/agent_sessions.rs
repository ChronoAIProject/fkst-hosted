//! The worker agent's session-driving methods (issue #151, increment 5).
//!
//! Split out of `agent.rs` (kept under the 500-line budget) as a child module so
//! it can reach `WorkerAgent`'s private session registry + engine config. These
//! methods turn a controller `ResolvedDispatch` (or a re-adopted engine) into a
//! supervised session: spawn the engine, start its supervise loop, register the
//! stop signal, and drain it on a `StopSession`. DORMANT in prod until the
//! activation increment emits a dispatch; until then no session is ever driven.

use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tokio::sync::watch;

use fkst_engine::{scan_and_adopt, RunningSession, SessionRunner};
use fkst_journal::Journaler;
use fkst_shared::protocol::ResolvedDispatch;

use super::{LiveSession, WorkerAgent};
use crate::engine::execute_dispatch;
use crate::engine::executor::SessionGuards;
use crate::engine::refresh::{AlreadyConfirmed, FenceConfirmer, NeverConfirmed, RefreshState};
use crate::engine::supervise::{supervise_session, SuperviseContext};

impl WorkerAgent {
    /// Spawn the engine for a resolved dispatch and start its supervise loop
    /// (#151). A spawn failure is logged LOUDLY (never a secret) and swallowed —
    /// a single bad dispatch must NOT crash the worker. Idempotent: a dispatch
    /// for a session already driven is ignored (the controller's claim is the
    /// authoritative dedupe).
    pub(crate) async fn handle_resolved_dispatch(self: &Arc<Self>, dispatch: &ResolvedDispatch) {
        let session_id = dispatch.session_id.clone();
        if self.is_driving(&session_id) {
            tracing::debug!(session_id = %session_id, "dispatch for an already-running session; ignoring");
            return;
        }
        // `execute_dispatch` is awaited to completion (the lock is NOT held across
        // it); the supervise loop is spawned + registered afterwards.
        match execute_dispatch(self.engine_config(), dispatch, self.http_client()).await {
            Ok(session) => {
                // A FRESH dispatch arrives already fence-confirmed (the controller
                // placed it with this live fencing id), so the refresh servicer is
                // armed from the first tick.
                let confirmer: Arc<dyn FenceConfirmer> =
                    Arc::new(AlreadyConfirmed(dispatch.fencing_id));
                let repo_ref = format!("{}/{}", dispatch.goal.repo.owner, dispatch.goal.repo.name);
                let expires_at = unix_ms_to_system_time(dispatch.github_token_expires_at_unix_ms);
                let (running, guards, journaler) = session.into_parts();
                self.spawn_supervise(
                    session_id.clone(),
                    running,
                    guards,
                    journaler,
                    repo_ref,
                    expires_at,
                    Some(dispatch.fencing_id),
                    confirmer,
                );
            }
            Err(error) => {
                // The error never carries a secret (see ExecError); log it loudly
                // and keep serving — the worker stays up.
                tracing::error!(session_id = %session_id, error = %error, "failed to execute dispatch; session NOT driven");
            }
        }
    }

    /// Wire a running engine + its on-disk guards to a supervise loop and register
    /// it. Shared by the dispatch handler (fresh, already-confirmed — guards from
    /// the clone/codex render) and the startup re-adopt (no guards, parked until
    /// the controller confirms the fence). The supervise task owns the running
    /// engine + its guards + the refresh servicer; the registry holds only the
    /// stop signal + the join handle. Returns `false` (dropping the engine, which
    /// is then orphaned-but-supervisable on the next restart) when a session with
    /// this id is already registered — the at-most-one-loop guard.
    #[allow(clippy::too_many_arguments)]
    pub fn spawn_supervise(
        self: &Arc<Self>,
        session_id: String,
        running: RunningSession,
        guards: SessionGuards,
        // The per-session journaler (#151 i6c), `Some` only for a FRESH dispatch
        // that carried a `JournalPlan`. `None` for an adopted session (no
        // dispatch/clone in hand to rebuild it). The supervise loop journals the
        // engine's RAISED stdout + lifecycle transitions through it.
        journaler: Option<Journaler>,
        repo_ref: String,
        token_expires_at: SystemTime,
        initial_fencing_id: Option<i64>,
        confirmer: Arc<dyn FenceConfirmer>,
    ) -> bool {
        if self.is_driving(&session_id) {
            tracing::debug!(session_id = %session_id, "supervise already running for this session; not double-spawning");
            return false;
        }
        let pid = running.pid;
        let runtime_dir = running.runtime_dir.clone();
        let refresh = RefreshState::new(
            self.clone(),
            session_id.clone(),
            repo_ref,
            runtime_dir,
            token_expires_at,
            confirmer,
        );
        let ctx = SuperviseContext {
            agent: self.clone(),
            session_id: session_id.clone(),
            refresh,
            initial_fencing_id,
        };
        let (stop_tx, stop_rx) = watch::channel(false);
        let handle = tokio::spawn(supervise_session(ctx, running, guards, journaler, stop_rx));
        self.insert_live_session(session_id.clone(), LiveSession::new(stop_tx, handle));
        tracing::info!(session_id = %session_id, pid, "engine supervised and session registered");
        true
    }

    /// Command a session's supervise loop to stop: flip its stop signal (the loop
    /// performs the engine stop + sends Released + the terminal report), await the
    /// loop's drain, then remove it from the registry. An unknown session still
    /// completes the protocol round-trip with a best-effort Released so the
    /// controller can reassign (e.g. a stop for a session this worker never had,
    /// or already finished).
    pub(crate) async fn stop_session(&self, session_id: &str) {
        match self.take_live_session(session_id) {
            Some(live) => {
                // Flip the stop signal; the supervise loop owns the engine stop +
                // Released + terminal report. Await the loop so the controller's
                // round-trip reflects a genuinely-stopped engine.
                let (stop, handle) = live.into_parts();
                let _ = stop.send(true);
                if let Err(error) = handle.await {
                    tracing::warn!(session_id = %session_id, error = %error, "supervise loop join failed on stop");
                    // The loop panicked before releasing — release here so the
                    // controller can still reassign without a double-run.
                    if let Err(e) = self.release(session_id).await {
                        tracing::warn!(session_id = %session_id, error = %e, "failed to send Released after loop join failure");
                    }
                }
            }
            None => {
                tracing::info!(session_id = %session_id, "StopSession for an unknown session; sending best-effort Released");
                if let Err(error) = self.release(session_id).await {
                    tracing::warn!(session_id = %session_id, error = %error, "failed to send Released for unknown session");
                }
            }
        }
    }

    /// A fresh [`SessionRunner`] over the worker's engine config — the supervise
    /// loop's stop site. `SessionRunner` is a cheap `EngineConfig` wrapper, so
    /// building one per stop is fine and keeps no shared mutable runner state.
    pub(crate) fn engine_runner(&self) -> SessionRunner {
        SessionRunner::new(self.engine_config().clone())
    }

    /// Re-adopt any live engines this worker left running across a restart, and
    /// start a PARKED supervise loop for each (#151). Called ONCE at startup,
    /// BEFORE the steady pull/heartbeat loop.
    ///
    /// CRITICAL re-adopt fencing: an adopted engine has a `.mint-nonce` written by
    /// the DEAD worker and a `ClaimMap` the controller may have moved/restarted,
    /// so it MUST re-establish a CURRENT `fencing_id` before it may mint. Each
    /// adopted session is therefore supervised with a [`NeverConfirmed`] fence-
    /// confirmer (the unit-tested seam): its refresh servicer is PARKED — no mint,
    /// no token write — until the controller confirms a live fence. The adopted
    /// `session_id`s ride the heartbeat's `running_sessions` (already wired), so
    /// the controller can push a `StopSession` for any whose claim is gone.
    ///
    /// TODO(#151 activation): the controller re-issues the live `fencing_id` for
    /// an adopted session (riding a `ControlMessage` on the heartbeat round-trip),
    /// at which point the parked servicer is armed with the confirmed fence AND the
    /// session's `repo_ref` / token expiry. Until then the seam defaults to "not
    /// confirmed" so an adopted engine NEVER mints on the dead worker's stale fence.
    pub fn scan_and_readopt(self: &Arc<Self>, min_age: Duration, now: SystemTime) {
        let runner = self.engine_runner();
        let report = scan_and_adopt(
            self.engine_temp_root_path(),
            self.worker_id(),
            &runner,
            min_age,
            now,
        );
        let adopted = report.adopted.len();
        for running in report.adopted {
            let session_id = adopted_session_id(&running);
            if session_id.is_empty() {
                tracing::warn!(
                    pid = running.pid,
                    "adopted engine has no session id breadcrumb; cannot supervise"
                );
                continue;
            }
            // Parked: NeverConfirmed defers the fence (and thus minting) to the
            // controller. repo_ref/token-expiry are placeholders until the
            // activation increment supplies them with the confirmed fence — they
            // are unused while parked (the servicer never mints).
            let confirmer: Arc<dyn FenceConfirmer> = Arc::new(NeverConfirmed);
            // No journaler: an adopted engine carries no dispatch/clone to
            // rebuild the per-session journaler from (the activation increment
            // re-supplies a plan with the confirmed fence).
            self.spawn_supervise(
                session_id,
                running,
                SessionGuards::none(),
                None,
                String::new(),
                now,
                None,
                confirmer,
            );
        }
        tracing::info!(
            adopted,
            "re-adopt scan complete; parked supervise loops started"
        );
    }
}

/// Convert a non-negative unix-ms timestamp to a `SystemTime` (saturating at the
/// epoch for a non-positive value, so a clock-skewed dispatch never panics).
fn unix_ms_to_system_time(unix_ms: i64) -> SystemTime {
    if unix_ms <= 0 {
        return SystemTime::UNIX_EPOCH;
    }
    UNIX_EPOCH + Duration::from_millis(unix_ms as u64)
}

/// The controller-assigned session id of an adopted engine, read from its owner
/// breadcrumb (the engine does not carry it on the handle). Empty on an
/// absent/malformed breadcrumb — the caller skips a session it cannot identify.
fn adopted_session_id(running: &RunningSession) -> String {
    match fkst_engine::breadcrumb::read_owner_breadcrumb(&running.runtime_dir) {
        Ok(Some(bc)) => bc.session_id,
        _ => String::new(),
    }
}
