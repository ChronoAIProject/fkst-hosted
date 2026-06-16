//! The worker-side supervise loop (issue #151, increments 5 + 6c).
//!
//! Once the worker spawns (or re-adopts) an engine, [`supervise_session`] drives
//! it for its lifetime — the worker-side mirror of the control-plane driver's
//! supervise loop (`fkst-control-plane/src/sessions/service.rs`, the
//! `SUPERVISE_POLL` / `MINT_REQUEST_POLL` `tokio::select!` loop), including
//! journaling (the engine's RAISED stdout + lifecycle transitions are recorded
//! direct to GitHub via the per-session journaler, mirroring the driver
//! verbatim — see `super::journal`). The loop is a `tokio::select!` over:
//!
//! - **a 500ms status tick** → on a `LiveStatus` change, send a [`StatusReport`];
//!   on a terminal status (Stopped/Failed), send a terminal report and exit;
//! - **a 200ms mint tick** → the credential-refresh servicer ([`RefreshState`]):
//!   JIT (helper request file) / periodic (pre-expiry) / reactive (a 401 flag);
//! - **the engine's stdout line stream** → scan each line for a GitHub auth
//!   failure to set the reactive-refresh flag, then JOURNAL the line (parse the
//!   `RAISED:` framing into a progress event / a malformed anomaly / debug-logged
//!   chatter);
//! - **a stop signal** (`watch::Receiver<bool>` the agent flips on `StopSession`)
//!   → journal Stopping, stop the engine, send a [`Released`], send a terminal
//!   report, journal the terminal lifecycle, exit.
//!
//! DORMANT: the loop runs only for a session the worker is actually driving, and
//! the controller emits no dispatch (and no journal plan) until activation, so
//! develop behaviour is byte-identical. The loop owns the `RunningSession` + its
//! on-disk guards + the per-session journaler and is the SOLE writer of the
//! token file (via the refresh servicer). Secrets are never logged.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, watch};
use tokio::time::MissedTickBehavior;

use fkst_engine::{LiveStatus, RunningSession};
use fkst_journal::{Journaler, Transition};
use fkst_shared::protocol::{SessionStatus, TerminalExit};

use crate::agent::WorkerAgent;
use crate::engine::executor::SessionGuards;
use crate::engine::journal;
use crate::engine::refresh::{RefreshOutcome, RefreshState};

/// Status-tick cadence — mirrors the driver's `SUPERVISE_POLL`.
const SUPERVISE_POLL: Duration = Duration::from_millis(500);

/// Mint-request servicer cadence — mirrors the driver's `MINT_REQUEST_POLL`.
const MINT_REQUEST_POLL: Duration = Duration::from_millis(200);

/// Everything the supervise loop borrows for one driven session.
///
/// The on-disk guards ([`SessionGuards`]) are DELIBERATELY not held here: they
/// are `Send` but the type-erased clone guard (`Box<dyn Any + Send>`) is not
/// `Sync`, so holding `&SuperviseContext` across an await would make the spawned
/// future non-`Send`. The guards are instead an owned local in
/// [`supervise_session`] that the loop never references — held purely to keep the
/// dirs alive, dropped when the loop returns.
pub struct SuperviseContext {
    pub agent: Arc<WorkerAgent>,
    pub session_id: String,
    /// The credential-refresh servicer (JIT / periodic / reactive), which is the
    /// SOLE writer of the token file. Parked until its fence is confirmed.
    pub refresh: RefreshState,
    /// The fencing id stamped on status reports for a FRESH dispatch (the
    /// controller placed it with a live fence). `None` for an ADOPTED session
    /// until its fence is confirmed — status reports are then skipped until the
    /// refresh servicer arms and yields the live fence.
    pub initial_fencing_id: Option<i64>,
}

/// Drive one engine to completion. Owns `running` (it `&mut`-drives `status()` /
/// `take_stdout()` / `stop()`), `guards` (held for the dirs' lifetime, never
/// referenced), `journaler` (the per-session GitHub progress record, `None` when
/// journaling is off), and `stop_rx` (the agent flips it on `StopSession`).
/// Returns when the engine is terminal or stopped — the caller (the agent) then
/// removes the session from its registry.
///
/// `journaler` is by-value (like `running` / `guards`) rather than in
/// [`SuperviseContext`] so the spawned future stays `Send` — the journaler is
/// `Send` and is only ever `&mut`-borrowed across awaits from this owned local.
pub async fn supervise_session(
    mut ctx: SuperviseContext,
    mut running: RunningSession,
    guards: SessionGuards,
    mut journaler: Option<Journaler>,
    mut stop_rx: watch::Receiver<bool>,
) {
    // Held for the session lifetime so the clone tree + CODEX_HOME outlive the
    // engine; never referenced (so the future stays `Send`).
    let _guards = guards;
    let session_id = ctx.session_id.clone();
    tracing::info!(session_id = %session_id, pid = running.pid, "supervise loop started");

    // The engine's line-framed stdout: BOTH the reactive-401 source AND the
    // journaling source. Leaving it untaken is safe (the drain task keeps the
    // pipe flowing); `None` after EOF parks the arm. Each line is scanned for a
    // 401 (reactive refresh) and journaled (the `RAISED:` framing).
    let mut stdout_rx = running.take_stdout();

    let mut status_tick = interval(SUPERVISE_POLL);
    let mut mint_tick = interval(MINT_REQUEST_POLL);

    // Initial status: the engine is up, so report Running once (best-effort; a
    // failed report retries on the next status transition), then journal the
    // Running lifecycle (mirrors the driver: report first, journal after).
    let mut last_reported: Option<SessionStatus> = None;
    report_transition(&ctx, &mut last_reported, SessionStatus::Running, None).await;
    journal::journal_lifecycle(&mut journaler, Transition::Running).await;

    loop {
        tokio::select! {
            _ = mint_tick.tick() => {
                let outcome = ctx.refresh.service_tick().await;
                if handle_refresh_outcome(&mut ctx, &mut running, &mut journaler, outcome).await {
                    return;
                }
            }
            line = next_stdout_line(&mut stdout_rx) => {
                match line {
                    // Scan for a GitHub auth failure to flag a reactive refresh,
                    // then journal the line (the `RAISED:` framing → a progress
                    // event / a malformed anomaly / debug-logged chatter).
                    Some(raw) => {
                        if is_github_auth_failure(&raw) {
                            ctx.refresh.flag_reactive_401();
                        }
                        journal::journal_stdout_line(&mut journaler, &raw).await;
                    }
                    // EOF/closed: park this arm so it never busy-loops.
                    None => stdout_rx = None,
                }
            }
            _ = stop_rx.changed() => {
                if *stop_rx.borrow() {
                    stop_and_release(&ctx, &mut running, &mut journaler, &mut last_reported).await;
                    return;
                }
            }
            _ = status_tick.tick() => {
                match running.status() {
                    LiveStatus::Running => {}
                    terminal => {
                        finish_terminal(&ctx, &mut running, &mut journaler, &mut last_reported, terminal).await;
                        return;
                    }
                }
            }
        }
    }
}

/// Apply a refresh outcome; return `true` when it terminates the loop.
async fn handle_refresh_outcome(
    ctx: &mut SuperviseContext,
    running: &mut RunningSession,
    journaler: &mut Option<Journaler>,
    outcome: RefreshOutcome,
) -> bool {
    match outcome {
        RefreshOutcome::StaleFence => {
            // Self-fence: a takeover owns the claim. Stop the local engine and
            // exit WITHOUT a Released, a status report, OR a journal write — the
            // new owner drives (and journals) it now; writing here would race the
            // takeover (mirrors the driver's lease-lost self-fence: kill the
            // engine, zero writes).
            tracing::warn!(
                session_id = %ctx.session_id,
                "self-fencing: stopping the local engine without status writes"
            );
            if let Err(error) = ctx.agent_runner_stop(running).await {
                tracing::error!(session_id = %ctx.session_id, error = %error, "engine stop failed after self-fence");
            }
            true
        }
        RefreshOutcome::Fatal { reason } => {
            // The credential can no longer be served (App gone, or token expired
            // after persistent failures). Fail loudly: stop the engine and send a
            // terminal Failed report — the worker never silently 401s.
            tracing::error!(session_id = %ctx.session_id, reason = %reason, "session failed: credential unrecoverable");
            let _ = ctx.agent_runner_stop(running).await;
            let mut last = Some(SessionStatus::Running);
            report_transition(
                ctx,
                &mut last,
                SessionStatus::Failed,
                Some(TerminalExit {
                    code: None,
                    signal: None,
                }),
            )
            .await;
            // Terminal journal (the reason is non-secret operator/credential
            // text; never a token).
            journal::journal_finish(
                journaler,
                Transition::Failed {
                    exit_code: None,
                    error: reason.clone(),
                },
            )
            .await;
            true
        }
        RefreshOutcome::Refreshed | RefreshOutcome::Skipped | RefreshOutcome::TransientFailure => {
            false
        }
    }
}

/// Stop the engine on a commanded stop, then send Released + a terminal Stopped
/// report. The supervise loop owns the actual `stop()` so a `StopSession` arm in
/// the agent only flips the watch signal and lets this drain run.
async fn stop_and_release(
    ctx: &SuperviseContext,
    running: &mut RunningSession,
    journaler: &mut Option<Journaler>,
    last_reported: &mut Option<SessionStatus>,
) {
    tracing::info!(session_id = %ctx.session_id, "stop signal received; stopping engine");
    // Journal Stopping BEFORE the engine stop (mirrors the driver's commanded-
    // stop order: journal Stopping forced, then stop, then finish Stopped).
    journal::journal_lifecycle(journaler, Transition::Stopping).await;
    let exit = match ctx.agent_runner_stop(running).await {
        Ok(()) => {
            tracing::info!(session_id = %ctx.session_id, "engine stopped on command");
            TerminalExit {
                code: Some(0),
                signal: None,
            }
        }
        Err(error) => {
            tracing::error!(session_id = %ctx.session_id, error = %error, "engine stop failed");
            TerminalExit {
                code: None,
                signal: None,
            }
        }
    };
    // Released first (so the controller can reassign), then the terminal report.
    if let Err(error) = ctx.agent.release(&ctx.session_id).await {
        tracing::warn!(session_id = %ctx.session_id, error = %error, "failed to send Released (will not retry; loop exiting)");
    }
    report_transition(ctx, last_reported, SessionStatus::Stopped, Some(exit)).await;
    // Terminal journal: a commanded stop is a clean Stopped exit.
    journal::journal_finish(
        journaler,
        Transition::Stopped {
            exit_code: exit.code,
        },
    )
    .await;
}

/// Handle an UNCOMMANDED terminal engine exit: reap the dirs, then send the
/// terminal report. A clean exit maps to Stopped; a non-zero/signalled exit to
/// Failed (the supervised contract: an uncommanded clean exit is still a failure
/// of the contract, but the engine's own exit code is the source of truth here).
async fn finish_terminal(
    ctx: &SuperviseContext,
    running: &mut RunningSession,
    journaler: &mut Option<Journaler>,
    last_reported: &mut Option<SessionStatus>,
    terminal: LiveStatus,
) {
    tracing::warn!(session_id = %ctx.session_id, ?terminal, "engine exited; finishing session");
    let (status, exit) = match terminal {
        LiveStatus::Stopped => (
            SessionStatus::Stopped,
            TerminalExit {
                code: Some(0),
                signal: None,
            },
        ),
        LiveStatus::Failed { code, signal } => {
            (SessionStatus::Failed, TerminalExit { code, signal })
        }
        // Unreachable (the caller only calls on a terminal status), but map it
        // defensively rather than panicking.
        LiveStatus::Running => (
            SessionStatus::Failed,
            TerminalExit {
                code: None,
                signal: None,
            },
        ),
    };
    // Reap/cleanup the dead engine's dirs (idempotent; the loop already observed
    // the exit, so this only cleans up).
    let _ = ctx.agent_runner_stop(running).await;
    report_transition(ctx, last_reported, status, Some(exit)).await;
    // Terminal journal: a clean exit is Stopped; anything else is Failed (using
    // the engine's own exit code as the source of truth).
    let transition = match status {
        SessionStatus::Stopped => Transition::Stopped {
            exit_code: exit.code,
        },
        _ => Transition::Failed {
            exit_code: exit.code,
            error: format!("engine exited: {terminal:?}"),
        },
    };
    journal::journal_finish(journaler, transition).await;
}

/// Report a status transition, best-effort. Suppresses a duplicate of the
/// last-reported status (a stable status produces no report). Skips entirely
/// when no live fencing id is available (an adopted session whose fence is not
/// yet confirmed — a report would be a controller no-op). A failed report is
/// logged and left for the next transition to retry; the supervise loop never
/// crashes on a reporting failure.
async fn report_transition(
    ctx: &SuperviseContext,
    last_reported: &mut Option<SessionStatus>,
    status: SessionStatus,
    terminal: Option<TerminalExit>,
) {
    if *last_reported == Some(status) {
        return;
    }
    let Some(fencing_id) = ctx.report_fencing_id() else {
        tracing::debug!(
            session_id = %ctx.session_id,
            ?status,
            "no confirmed fence yet; skipping status report"
        );
        return;
    };
    match ctx
        .agent
        .report_status(&ctx.session_id, fencing_id, status, terminal)
        .await
    {
        Ok(()) => *last_reported = Some(status),
        Err(error) => {
            // Best-effort: leave last_reported unchanged so the next tick retries.
            tracing::warn!(
                session_id = %ctx.session_id,
                ?status,
                error = %error,
                "status report failed; will retry on the next transition"
            );
        }
    }
}

impl SuperviseContext {
    /// The fencing id to stamp on status reports: the confirmed live fence once
    /// the refresh servicer is armed, else the fresh dispatch's initial fence,
    /// else `None` (adopted + unconfirmed).
    fn report_fencing_id(&self) -> Option<i64> {
        self.refresh.armed_fencing_id().or(self.initial_fencing_id)
    }

    /// Stop the engine through the agent's runner (the configured grace). A thin
    /// wrapper so every stop site goes through one place.
    async fn agent_runner_stop(
        &self,
        running: &mut RunningSession,
    ) -> Result<(), fkst_engine::RunnerError> {
        self.agent.engine_runner().stop(running).await
    }
}

/// Build an interval that delays (never bursts) on a missed tick.
fn interval(period: Duration) -> tokio::time::Interval {
    let mut interval = tokio::time::interval(period);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    interval
}

/// Await the next stdout line, parking forever when the stream is taken/closed
/// (so the select arm never busy-loops). Mirrors the driver's helper.
async fn next_stdout_line(rx: &mut Option<mpsc::Receiver<Vec<u8>>>) -> Option<Vec<u8>> {
    match rx {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

/// Detect a GitHub authentication/authorization failure in one engine stdout
/// line — the reactive-re-mint signal. Ported VERBATIM from the control-plane
/// driver's `is_github_auth_failure` so the two pollers detect the SAME markers.
/// Conservative by design: only unambiguous auth markers match, so ordinary
/// chatter never spuriously burns a refresh (the cooldown is the backstop).
fn is_github_auth_failure(raw: &[u8]) -> bool {
    let line = String::from_utf8_lossy(raw).to_ascii_lowercase();
    line.contains("bad credentials")
        || line.contains("401 unauthorized")
        || line.contains("http 401")
        || line.contains("requires authentication")
        || (line.contains("github") && line.contains("token") && line.contains("expired"))
}

#[cfg(test)]
#[path = "supervise_tests.rs"]
mod tests;
