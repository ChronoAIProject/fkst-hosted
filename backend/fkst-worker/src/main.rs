//! fkst-worker entrypoint — the worker deployable of the control-plane/worker
//! split.
//!
//! Thin shim: install the JSON tracing subscriber, load the fail-closed worker
//! config, and hand off to `fkst_worker::run_worker`. All the logic lives in the
//! library so it is unit-testable; the engine driver / re-adopt arrive in #136.

use std::process::ExitCode;

use fkst_worker::{run_worker, WorkerConfig};
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    // Install the JSON subscriber FIRST so even a config-load failure is logged
    // structurally (mirrors the control-plane bootstrap).
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

    let config = match WorkerConfig::load_from_env() {
        Ok(config) => config,
        Err(error) => {
            tracing::error!(error = %error, "failed to load worker configuration");
            return ExitCode::FAILURE;
        }
    };

    run_worker(config).await
}
