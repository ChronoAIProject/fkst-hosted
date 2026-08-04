//! `fkst-audit-relay` entrypoint: JSON tracing init, then
//! [`fkst_control_plane::audit_relay::run`].
//!
//! Deliberately thin. Everything that can fail — configuration, storage,
//! migration, the delivery worker, the listener — lives in the library module
//! beside the code it starts, so the whole startup path is unit-testable and this
//! file has nothing in it that could drift.
//!
//! This binary never constructs the control plane's `AppState` and never builds
//! its router: the two deployables share contract code and nothing else.

use std::process::ExitCode;

use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> ExitCode {
    // Install the subscriber FIRST so even a configuration failure is logged
    // structurally. The directive is read straight from the environment because
    // the subscriber must exist before any config load runs.
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
    tracing::info!("audit relay: subscriber initialized");

    fkst_control_plane::audit_relay::run().await
}
