//! Worker orchestration: register up to the controller, then run the heartbeat
//! + pull loops alongside the local health server until shutdown.
//!
//! The worker NEVER connects to MongoDB and this module imports no control-plane
//! code — it is the lean "hands" half of the split.

use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::time::MissedTickBehavior;
use tokio_util::sync::CancellationToken;

use fkst_shared::protocol::LifecycleState;

use crate::agent::WorkerAgent;
use crate::config::WorkerConfig;
use crate::server::{health_router, shutdown_signal};

/// Minimum age a stray runtime dir must reach before the re-adopt scan reaps it,
/// so a dir created the same instant a sibling worker is mid-spawn is never
/// deleted out from under it. The engine's stop grace is the natural lower bound.
const READOPT_MIN_AGE: Duration = Duration::from_secs(60);

/// Run the worker to completion. Returns FAILURE only on a fatal startup
/// problem (bad secret / incompatible protocol / un-bindable health port).
pub async fn run_worker(config: WorkerConfig) -> ExitCode {
    tracing::info!(config = ?config, "worker starting");
    let agent = Arc::new(WorkerAgent::from_config(&config));

    // Register (retries transient failures forever; fatal on wrong secret).
    let reg = match agent.register().await {
        Ok(reg) => reg,
        Err(error) => {
            tracing::error!(error = %error, "fatal: worker could not register with the controller");
            return ExitCode::FAILURE;
        }
    };

    // Re-adopt any live engines this worker left running across a restart, BEFORE
    // the steady pull/heartbeat loop, so the heartbeat's first `running_sessions`
    // already reflects them. Each adopted engine is supervised with a PARKED
    // refresh servicer (it must re-establish a live fence from the controller
    // before it may mint — see `WorkerAgent::scan_and_readopt`). Dormant in prod
    // until activation: no engines exist to adopt, so this is a no-op scan.
    agent.scan_and_readopt(READOPT_MIN_AGE, SystemTime::now());

    // Two tokens so the SIGTERM path can DRAIN before the loops stop. `serve_stop`
    // takes the health server down the instant the signal fires (k8s has already
    // removed the pod from endpoints, so liveness no longer matters). `cancel`
    // stops the heartbeat + pull loops, but ONLY AFTER the drain finishes — the
    // controller needs the worker to keep heartbeating (so it is not expired) and
    // to keep receiving its `Released` acks throughout the drain (#140a).
    let cancel = CancellationToken::new();
    let serve_stop = CancellationToken::new();

    // Heartbeat loop on the controller-dictated cadence.
    let hb_interval = reg.heartbeat_interval_secs.max(1);
    let hb_agent = agent.clone();
    let hb_cancel = cancel.clone();
    let heartbeat = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(hb_interval));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    if let Err(error) = hb_agent.heartbeat(LifecycleState::Active).await {
                        tracing::warn!(error = %error, "heartbeat failed (will retry next tick)");
                    }
                }
                _ = hb_cancel.cancelled() => break,
            }
        }
    });

    // Pull loop (empty assignments until #135 grants claim authority).
    let pull_interval = config.pull_interval_secs.max(1);
    let pull_agent = agent.clone();
    let pull_cancel = cancel.clone();
    let capacity = config.capacity;
    let pull = tokio::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(pull_interval));
        interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = interval.tick() => {
                    // Once draining, STOP requesting new work — the worker is
                    // handing off, not taking on (#140a). Keep looping so the
                    // cancel arm can still break the loop on final shutdown.
                    if pull_agent.lifecycle() != LifecycleState::Active {
                        tracing::debug!("draining: skipping pull");
                        continue;
                    }
                    match pull_agent.pull(capacity).await {
                        Ok(resp) => tracing::debug!(assignments = resp.assignments.len(), "pull (no claim authority yet)"),
                        Err(error) => tracing::warn!(error = %error, "pull failed (will retry next tick)"),
                    }
                }
                _ = pull_cancel.cancelled() => break,
            }
        }
    });

    // Serve the local health endpoint. It is taken down by `serve_stop` (flipped
    // the instant the OS shutdown signal fires) — BEFORE the drain runs — because
    // k8s has already pulled the pod from its Service endpoints by the time it
    // sends SIGTERM, so liveness no longer matters during the drain.
    let addr = format!("{}:{}", config.bind_addr, config.port);
    let listener = match tokio::net::TcpListener::bind(&addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(error = %error, addr = %addr, "fatal: worker could not bind the health port");
            cancel.cancel();
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(addr = %addr, "worker health server listening");

    let serve_token = serve_stop.clone();
    let serve = tokio::spawn(async move {
        let server = axum::serve(listener, health_router())
            .with_graceful_shutdown(async move { serve_token.cancelled_owned().await });
        if let Err(error) = server.await {
            tracing::error!(error = %error, "worker health server error");
        }
    });

    // Block here until the OS asks the worker to terminate. The heartbeat + pull
    // loops keep running in the background throughout (the controller still needs
    // liveness), and the health server stays up until we flip `serve_stop` below.
    shutdown_signal().await;
    tracing::info!("shutdown signal received; beginning graceful drain");

    // Take the health server down first (the pod is already out of rotation).
    serve_stop.cancel();

    // DRAIN while the heartbeat loop is STILL ALIVE: the controller must keep
    // seeing this worker (so it is not expired mid-handoff) and must receive each
    // session's `Released` during the drain. `run_drain` flips the pull gate
    // (begin_drain), announces `Draining`, then checkpoints + stops every
    // in-flight session within the bounded grace, emitting a `Released` apiece.
    let grace = Duration::from_secs(config.worker_drain_grace_secs);
    let outcome = crate::drain::run_drain(&agent, grace).await;
    tracing::info!(
        released = outcome.released,
        total = outcome.total,
        timed_out = outcome.timed_out,
        "drain finished; stopping background loops"
    );

    // ONLY NOW stop the heartbeat + pull loops and await everything. The drain
    // already sent `Draining` (no redundant final heartbeat needed).
    cancel.cancel();
    let _ = heartbeat.await;
    let _ = pull.await;
    let _ = serve.await;

    tracing::info!("worker stopped");
    ExitCode::SUCCESS
}
