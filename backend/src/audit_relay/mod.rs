//! `fkst-audit-relay`: the durable outbox between stateless control-plane
//! replicas and self-hosted PostHog.
//!
//! ```text
//! control-plane replicas (stateless)
//!   -> internal authenticated HTTP        [http]      write token / read token
//!   -> fkst-audit-relay (single writer)
//!        -> SQLite WAL on its own PVC     [db]        commit BEFORE answering
//!        -> PostHog capture/batch API     [capture]   accepted != verified
//!        -> fixed PostHog query           [verify]    proven query-visible
//! ```
//!
//! ## Why this is a second process, not a module of the control plane
//!
//! An in-memory queue with retries cannot survive a process, Pod, or node
//! restart, and the control plane must stay stateless (epic `OPS-03`). Embedding
//! SQLite in every replica would give N writers to N different volumes and no
//! durable answer to "did this request happen". One relay with one volume gives
//! exactly one durable answer, and the control plane keeps no state at all.
//!
//! It is nevertheless the SAME crate: two `[[bin]]` targets, two module trees,
//! one set of audit contracts. That is deliberate — the event types, the
//! validation, the redaction rules, and the PostHog projection must be one
//! definition or the two processes would eventually disagree about what a record
//! means. The relay never constructs `AppState` and never builds the control
//! plane's router; the only shared code is contract code.
//!
//! ## What this process must never do
//!
//! Store a raw HTTP request, body, header, URI, credential, or upstream error
//! string; log a token or a record's content; expose a public API; delete an
//! unverified, incomplete, or dead-letter record; or call a capture `2xx`
//! "delivered".
//!
//! Module map:
//!
//! - [`protocol`] / [`query`] — the exact wire contract, shared with the client;
//! - [`record`] — the closed storage vocabulary and the delivery state machine;
//! - [`auth`] — the two constant-time-compared bearer credentials;
//! - [`db`] — one writer, a bounded reader pool, and the fail-closed health flag;
//! - [`http`] — the internal surface, committing before it answers;
//! - [`worker`] + [`capture`] + [`verify`] + [`closer`] — the background sweep;
//! - [`incomplete`] — what a request that never finished is allowed to say;
//! - [`projection`] — replaying a stored row through the shared capture format;
//! - [`metrics`] — bounded, closed-label telemetry (epic `OPS-04`).

use std::process::ExitCode;
use std::sync::Arc;

use tokio_util::sync::CancellationToken;

pub mod auth;
pub mod capture;
pub mod closer;
pub mod config;
pub mod db;
pub mod http;
pub mod incomplete;
pub mod metrics;
pub mod posthog;
pub mod projection;
pub mod protocol;
pub mod query;
pub mod record;
pub mod verify;
pub mod worker;

/// Shared, credential-free fixtures for this module's unit tests.
#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use config::RelayConfig;
pub use db::{Database, DatabaseSettings, DbError};
pub use http::{build_router, RelayState};
pub use metrics::RelayMetrics;
pub use record::{RecordState, RelayRecordKind};
pub use worker::RelayWorker;

/// Boot and serve the relay. The `fkst-audit-relay` binary is a thin wrapper
/// around this so the whole startup path is testable and lives beside the code
/// it starts.
///
/// Every step before the listener binds is fail-closed: a relay that could not
/// prove its storage is writable and migrated must not accept a record it might
/// be unable to keep.
pub async fn run() -> ExitCode {
    let config = match RelayConfig::load_from_env() {
        Ok(config) => Arc::new(config),
        Err(error) => {
            tracing::error!(error = %error, "audit relay: failed to load configuration");
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(
        bind_addr = %config.bind_addr,
        // The path is deployment topology, not a secret; the tokens never appear.
        db_path = %config.db_path.display(),
        capture = config.capture_configured(),
        verification = config.verification_configured(),
        verified_retention_days = config.verified_retention_days,
        audit_retention_days = config.audit_retention_days,
        "audit relay: configuration loaded"
    );

    let database = match Database::open(
        &config.db_path,
        DatabaseSettings {
            busy_timeout_ms: config.busy_timeout_ms,
            writer_queue_capacity: config.writer_queue_capacity,
            read_concurrency: config.max_read_concurrency,
            max_records: config.max_records,
        },
    ) {
        Ok(database) => database,
        Err(error) => {
            tracing::error!(reason = error.as_str(), "audit relay: storage unavailable");
            return ExitCode::FAILURE;
        }
    };

    let state = RelayState::new(database, config.clone(), RelayMetrics::new());
    let worker = match RelayWorker::new(&state) {
        Ok(worker) => worker,
        Err(error) => {
            tracing::error!(error = %error, "audit relay: failed to build the delivery worker");
            return ExitCode::FAILURE;
        }
    };
    let cancel = CancellationToken::new();
    let worker_cancel = cancel.clone();
    let worker_task = tokio::spawn(async move { worker.run(worker_cancel).await });

    let router = build_router(state);
    let listener = match tokio::net::TcpListener::bind(config.bind_addr).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(error = %error, addr = %config.bind_addr, "audit relay: failed to bind");
            cancel.cancel();
            return ExitCode::FAILURE;
        }
    };
    tracing::info!(addr = %config.bind_addr, "audit relay: listening");

    let serve = axum::serve(listener, router.into_make_service())
        .with_graceful_shutdown(shutdown_signal())
        .await;

    // Stop the sweep only AFTER in-flight requests drained, so the last
    // committed record is already durable before the process exits.
    cancel.cancel();
    let _ = worker_task.await;

    if let Err(error) = serve {
        tracing::error!(error = %error, "audit relay: server error");
        return ExitCode::FAILURE;
    }
    tracing::info!("audit relay: stopped");
    ExitCode::SUCCESS
}

/// Resolve on SIGTERM (how Kubernetes terminates Pods) or Ctrl-C.
async fn shutdown_signal() {
    let ctrl_c = async {
        if tokio::signal::ctrl_c().await.is_err() {
            tracing::error!("audit relay: could not install the SIGINT handler");
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut signal) => {
                signal.recv().await;
            }
            Err(_) => {
                tracing::error!("audit relay: could not install the SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
    tracing::info!("audit relay: shutdown signal received");
}
