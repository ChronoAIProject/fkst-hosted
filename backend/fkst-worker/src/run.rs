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

    let cancel = CancellationToken::new();

    // Translate the OS shutdown signal into a cancellation everything observes.
    let signal_cancel = cancel.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal_cancel.cancel();
    });

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
                    match pull_agent.pull(capacity).await {
                        Ok(resp) => tracing::debug!(assignments = resp.assignments.len(), "pull (no claim authority yet)"),
                        Err(error) => tracing::warn!(error = %error, "pull failed (will retry next tick)"),
                    }
                }
                _ = pull_cancel.cancelled() => break,
            }
        }
    });

    // Serve the local health endpoint until the shutdown signal fires.
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

    let serve_cancel = cancel.clone();
    let serve = axum::serve(listener, health_router())
        .with_graceful_shutdown(async move { serve_cancel.cancelled_owned().await });
    if let Err(error) = serve.await {
        tracing::error!(error = %error, "worker health server error");
    }

    // Stop the loops, then send a best-effort final Draining heartbeat so the
    // controller marks the worker draining promptly (no real drain here — #140).
    cancel.cancel();
    let _ = heartbeat.await;
    let _ = pull.await;
    if let Err(error) = agent.heartbeat(LifecycleState::Draining).await {
        tracing::debug!(error = %error, "final draining heartbeat not delivered (controller may be gone)");
    }

    tracing::info!("worker stopped");
    ExitCode::SUCCESS
}
