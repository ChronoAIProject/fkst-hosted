//! fkst-worker entrypoint — the worker deployable of the control-plane/worker
//! split (issue #145).
//!
//! At scaffolding time this is a compiling SKELETON: it installs the JSON
//! tracing subscriber, logs the role it is running as, then waits for a
//! shutdown signal (SIGTERM — how Kubernetes terminates pods — or Ctrl-C) and
//! exits 0. The engine driver, the controller registry client, and the work
//! pull loop arrive in later database-free issues (#134/#136/#140); this file
//! exists now so the worker can be built and deployed as its own image.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    // Install the JSON subscriber FIRST so even an early failure is logged
    // structurally. The directive is read straight from the environment because
    // the subscriber must exist before anything else runs. This mirrors the
    // control-plane's tracing bootstrap.
    let raw_directive =
        std::env::var("FKST_HOSTED_LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
    let (filter, directive_ok) = match EnvFilter::try_new(&raw_directive) {
        Ok(filter) => (filter, true),
        Err(_) => (EnvFilter::new("info"), false),
    };
    tracing_subscriber::fmt()
        .json()
        .with_env_filter(filter)
        .init();
    if !directive_ok {
        tracing::warn!(directive = %raw_directive, "invalid log directive; falling back to info");
    }

    // The role is informational at scaffolding time. A worker is always started
    // as `FKST_ROLE=worker`; we log whatever is set so a misconfigured Deployment
    // is visible in the logs.
    let role = std::env::var("FKST_ROLE").unwrap_or_else(|_| "worker".to_string());
    tracing::info!(role = %role, "fkst-worker skeleton starting");

    shutdown_signal().await;

    tracing::info!("fkst-worker skeleton stopped");
    ExitCode::SUCCESS
}

/// Resolve when either SIGTERM (how Kubernetes terminates pods) or Ctrl-C
/// (SIGINT) arrives. Shared shutdown semantics with the control-plane so both
/// deployables drain identically under a rolling restart.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install SIGINT handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }

    tracing::info!("shutdown signal received");
}
