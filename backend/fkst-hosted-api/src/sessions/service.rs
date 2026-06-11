//! Single-pod session orchestration: one detached driver task per session
//! supervises one engine process and advances the session document through
//! `pending -> validating -> running -> stopping -> stopped | failed`.
//!
//! v1 posture (superseded by the multi-pod lease takeover work, #24/#26):
//! - The driver is a detached `tokio::spawn` on THIS pod; a pod restart
//!   orphans every in-flight session (swept to `failed` at startup by
//!   [`SessionRepo::fail_orphans`]).
//! - No lease is taken; one-live-session-per-package is NOT enforced here.
//!
//! Concurrency rules (load-bearing):
//! - Every status write goes through the repository CAS
//!   ([`SessionRepo::transition`]); a CAS miss means a concurrent stop won
//!   and the driver converges instead of overwriting.
//! - The in-memory registry `Mutex` is sync and NEVER held across an await.
//! - `SessionRunner::start` is awaited to completion, never select-cancelled
//!   (a cancelled start would leak intent mid-spawn; stop-vs-start races are
//!   resolved AFTER start returns, via the `validating -> running` CAS).

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use bson::doc;
use tokio::sync::watch;

use crate::engine::{
    EngineConfig, LiveStatus, PreparedPackage, RunnerError, RunningSession, SessionRunner,
};
use crate::error::AppError;
use crate::models::{SessionDoc, SessionStatus};
use crate::packages::{is_valid_name, PackageRepository};
use crate::sessions::repo::{status_bson, SessionRepo};

/// Maximum stored byte length of a `package_name` (matches the packages
/// domain bound).
const MAX_PACKAGE_NAME_BYTES: usize = 128;

/// Cap on the `error` field persisted to a failed session (truncated at a
/// char boundary; the full text is logged).
const MAX_ERROR_BYTES: usize = 4096;

/// Poll cadence of the driver's supervise loop.
const SUPERVISE_POLL: Duration = Duration::from_millis(500);

/// Poll cadence while waiting for driver tasks to drain during shutdown.
const SHUTDOWN_POLL: Duration = Duration::from_millis(100);

/// Extra headroom on top of the engine stop grace for the shutdown drain.
const SHUTDOWN_HEADROOM_SECS: u64 = 10;

/// Shared internals behind the clonable service handle.
struct Inner {
    repo: SessionRepo,
    packages: PackageRepository,
    runner: SessionRunner,
    /// Per-session stop signal; entries are inserted by `create` and removed
    /// by the owning driver task on every exit path. The lock is sync and
    /// never held across an await.
    registry: Mutex<HashMap<bson::Uuid, watch::Sender<bool>>>,
    /// Advisory pod identity stamped onto sessions this pod drives.
    pod_id: Option<String>,
    /// Bound for the shutdown drain (engine stop grace + headroom).
    shutdown_bound: Duration,
}

/// Clonable orchestration service: create / get / stop sessions and drive
/// their engine processes on this pod.
#[derive(Clone)]
pub struct SessionService {
    inner: Arc<Inner>,
}

impl SessionService {
    /// Build the service. `pod_id` is taken from `HOSTNAME` when present
    /// (the Kubernetes pod name); absent locally, the advisory field stays
    /// `null` until the first driver write.
    pub fn new(repo: SessionRepo, packages: PackageRepository, engine: EngineConfig) -> Self {
        let shutdown_bound = Duration::from_secs(engine.stop_grace_secs + SHUTDOWN_HEADROOM_SECS);
        Self {
            inner: Arc::new(Inner {
                repo,
                packages,
                runner: SessionRunner::new(engine),
                registry: Mutex::new(HashMap::new()),
                pod_id: std::env::var("HOSTNAME").ok(),
                shutdown_bound,
            }),
        }
    }

    /// The repository handle (startup hooks: orphan sweep).
    pub fn repo(&self) -> &SessionRepo {
        &self.inner.repo
    }

    /// Create a session for `package_name`: validate, check the package
    /// exists, insert the `pending` document, and spawn the detached driver
    /// task. Returns the created document immediately (the driver advances
    /// it asynchronously).
    pub async fn create(&self, package_name: &str) -> Result<SessionDoc, AppError> {
        if !is_valid_name(package_name) || package_name.len() > MAX_PACKAGE_NAME_BYTES {
            tracing::warn!(
                package_name_bytes = package_name.len(),
                "session create rejected: invalid package name"
            );
            return Err(AppError::Validation(
                "invalid package name: must fully match [A-Za-z0-9_-]+ and be at most 128 bytes"
                    .to_string(),
            ));
        }
        if !self.inner.packages.exists(package_name).await? {
            tracing::warn!(package_name = %package_name, "session create rejected: unknown package");
            return Err(AppError::NotFound(format!(
                "package not found: {package_name}"
            )));
        }

        let session = SessionDoc {
            id: bson::Uuid::new(),
            package_name: package_name.to_string(),
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            created_at: bson::DateTime::now(),
            started_at: None,
            stopped_at: None,
        };
        self.inner.repo.insert(&session).await?;

        let (stop_tx, stop_rx) = watch::channel(false);
        self.inner
            .registry
            .lock()
            .expect("session registry lock poisoned")
            .insert(session.id, stop_tx);

        // Detached driver task — the swap point for the future pool-manager
        // (#24): it will claim documents instead of being spawned inline.
        let inner = Arc::clone(&self.inner);
        let id = session.id;
        let package_name = session.package_name.clone();
        tokio::spawn(async move {
            drive(Arc::clone(&inner), id, package_name, stop_rx).await;
            inner
                .registry
                .lock()
                .expect("session registry lock poisoned")
                .remove(&id);
            tracing::debug!(session_id = %id, "driver task exited");
        });

        tracing::info!(session_id = %session.id, package_name = %session.package_name, "session driver spawned");
        Ok(session)
    }

    /// Fetch one session document (pure status projection).
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<SessionDoc>, AppError> {
        self.inner.repo.get(id).await
    }

    /// Request a stop. The document CAS runs FIRST (so the intent is durable
    /// even if this pod dies immediately after), then the in-memory driver is
    /// signalled best-effort. Idempotent: a session already `stopping` or
    /// terminal answers Ok without a state change; an absent id is NotFound.
    pub async fn request_stop(&self, id: bson::Uuid) -> Result<(), AppError> {
        let transitioned = self
            .inner
            .repo
            .transition(
                id,
                &[
                    SessionStatus::Pending,
                    SessionStatus::Validating,
                    SessionStatus::Running,
                ],
                doc! { "status": status_bson(SessionStatus::Stopping) },
            )
            .await?;

        // Best-effort: wake the driver if it lives on this pod. The CAS
        // outcome above stays authoritative either way.
        if let Some(sender) = self
            .inner
            .registry
            .lock()
            .expect("session registry lock poisoned")
            .get(&id)
        {
            let _ = sender.send(true);
        }

        match transitioned {
            Some(session) => {
                tracing::info!(session_id = %id, status = ?session.status, "session stop requested");
                Ok(())
            }
            None => match self.inner.repo.get(id).await? {
                Some(session) => {
                    tracing::debug!(session_id = %id, status = ?session.status, "session stop no-op");
                    Ok(())
                }
                None => Err(AppError::NotFound(format!("session not found: {id}"))),
            },
        }
    }

    /// Signal every live driver to stop and wait (bounded) for the registry
    /// to drain. Called after `axum::serve` returns so in-flight engines get
    /// a graceful SIGTERM before the pod exits.
    pub async fn shutdown(&self) {
        let live: usize = {
            let registry = self
                .inner
                .registry
                .lock()
                .expect("session registry lock poisoned");
            for sender in registry.values() {
                let _ = sender.send(true);
            }
            registry.len()
        };
        if live == 0 {
            tracing::info!("session shutdown: no live drivers");
            return;
        }
        tracing::info!(live, "session shutdown: stop signalled to all drivers");

        let deadline = tokio::time::Instant::now() + self.inner.shutdown_bound;
        loop {
            let remaining = self
                .inner
                .registry
                .lock()
                .expect("session registry lock poisoned")
                .len();
            if remaining == 0 {
                tracing::info!("session shutdown: all drivers drained");
                return;
            }
            if tokio::time::Instant::now() >= deadline {
                tracing::warn!(remaining, "session shutdown: drain deadline reached");
                return;
            }
            tokio::time::sleep(SHUTDOWN_POLL).await;
        }
    }
}

/// Truncate driver-produced error text at a char boundary so the stored
/// document stays bounded (full text is in the logs).
fn truncate_error(text: &str) -> String {
    if text.len() <= MAX_ERROR_BYTES {
        return text.to_string();
    }
    let mut end = MAX_ERROR_BYTES;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{} [truncated]", &text[..end])
}

/// Render a start failure for the persisted `error` field: the captured
/// stderr detail matters more than the bare variant name.
fn describe_runner_error(error: &RunnerError) -> String {
    match error {
        RunnerError::ConformanceFailed { code, stderr } => {
            format!("conformance failed (exit {code}): {stderr}")
        }
        RunnerError::StartupFailed { stderr } => format!("engine startup failed: {stderr}"),
        other => other.to_string(),
    }
}

/// Describe a terminal [`LiveStatus`] for the persisted `error` field.
fn describe_exit(status: LiveStatus, session: &RunningSession) -> String {
    let exit = match status {
        LiveStatus::Stopped => "engine exited unexpectedly (clean exit)".to_string(),
        LiveStatus::Failed { code, signal } => match (code, signal) {
            (Some(code), _) => format!("engine exited unexpectedly (code {code})"),
            (None, Some(signal)) => format!("engine killed by signal {signal}"),
            (None, None) => "engine exited unexpectedly".to_string(),
        },
        LiveStatus::Running => "engine state inconsistent".to_string(),
    };
    match session.tail_logs() {
        Some(tail) if !tail.is_empty() => format!("{exit}\nlast engine logs:\n{tail}"),
        _ => exit,
    }
}

/// The driver state machine for one session. Every status write is a CAS;
/// every exit path leaves the document terminal or owned by a concurrent
/// stop request that this driver then completes.
async fn drive(
    inner: Arc<Inner>,
    id: bson::Uuid,
    package_name: String,
    mut stop_rx: watch::Receiver<bool>,
) {
    let now = bson::DateTime::now;

    // (A) pending -> validating, stamping this pod's identity.
    let claimed = inner
        .repo
        .transition(
            id,
            &[SessionStatus::Pending],
            doc! {
                "status": status_bson(SessionStatus::Validating),
                "pod_id": inner.pod_id.as_deref().map(bson::Bson::from).unwrap_or(bson::Bson::Null),
            },
        )
        .await;
    match claimed {
        Ok(Some(_)) => {}
        Ok(None) => {
            // Raced: a stop arrived before we claimed, or the doc vanished.
            match inner.repo.get(id).await {
                Ok(Some(session)) if session.status == SessionStatus::Stopping => {
                    let _ = inner
                        .repo
                        .transition(
                            id,
                            &[SessionStatus::Stopping],
                            doc! {
                                "status": status_bson(SessionStatus::Stopped),
                                "stopped_at": now(),
                            },
                        )
                        .await;
                    tracing::info!(session_id = %id, "session stopped before validation began");
                }
                other => {
                    tracing::warn!(session_id = %id, state = ?other.map(|s| s.map(|d| d.status)), "driver lost the pending claim; exiting");
                }
            }
            return;
        }
        Err(error) => {
            tracing::error!(session_id = %id, error = %error, "driver failed to claim session");
            return;
        }
    }

    // (B) Load the package (it may have been deleted since create).
    let package = match inner.packages.get(&package_name).await {
        Ok(Some(package)) => package,
        Ok(None) => {
            fail_session(&inner, id, "package disappeared before start").await;
            return;
        }
        Err(error) => {
            tracing::error!(session_id = %id, error = %error, "driver failed to load package");
            fail_session(&inner, id, &format!("failed to load package: {error}")).await;
            return;
        }
    };
    let prepared = PreparedPackage::from(package);

    // (C) Start the engine. This await runs to completion (never
    // select-cancelled); a stop that raced in is honored right after.
    let mut session = match inner.runner.start(&prepared).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(session_id = %id, error = %error, "engine start failed");
            fail_session(&inner, id, &describe_runner_error(&error)).await;
            return;
        }
    };

    let stop_already_requested = *stop_rx.borrow();
    let promoted = if stop_already_requested {
        None
    } else {
        inner
            .repo
            .transition(
                id,
                &[SessionStatus::Validating],
                doc! {
                    "status": status_bson(SessionStatus::Running),
                    "pid": session.pid,
                    "runtime_dir": session.runtime_dir.display().to_string(),
                    "started_at": now(),
                },
            )
            .await
            .unwrap_or_else(|error| {
                tracing::error!(session_id = %id, error = %error, "running CAS failed");
                None
            })
    };
    if promoted.is_none() {
        // A stop won the race (or Mongo failed): never leave a live engine.
        tracing::info!(session_id = %id, "stop raced engine start; stopping the fresh engine");
        let stop_result = inner.runner.stop(&mut session).await;
        if let Err(error) = &stop_result {
            tracing::error!(session_id = %id, error = %error, "failed to stop freshly-started engine");
        }
        let _ = inner
            .repo
            .transition(
                id,
                &[SessionStatus::Stopping, SessionStatus::Validating],
                doc! {
                    "status": status_bson(SessionStatus::Stopped),
                    "stopped_at": now(),
                },
            )
            .await;
        return;
    }
    tracing::info!(session_id = %id, pid = session.pid, "session running");

    // (D) Supervise: react to stop requests and watch engine liveness.
    let mut tick = tokio::time::interval(SUPERVISE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            // A closed channel (service dropped) also reads as a stop: this
            // pod is going away and the engine must not be orphaned silently.
            _ = stop_rx.changed() => {
                tracing::info!(session_id = %id, "stop signal received; stopping engine");
                match inner.runner.stop(&mut session).await {
                    Ok(()) => {
                        // `running` is in the from-set because a graceful
                        // pod shutdown signals the driver WITHOUT an HTTP
                        // stop having CAS'd the document to `stopping`; a
                        // commanded stop arrives as `stopping`. Either way
                        // this stop was driver-performed and is `stopped`.
                        let _ = inner
                            .repo
                            .transition(
                                id,
                                &[SessionStatus::Stopping, SessionStatus::Running],
                                doc! {
                                    "status": status_bson(SessionStatus::Stopped),
                                    "stopped_at": now(),
                                },
                            )
                            .await;
                        tracing::info!(session_id = %id, "session stopped");
                    }
                    Err(error) => {
                        tracing::error!(session_id = %id, error = %error, "engine stop failed");
                        let _ = inner
                            .repo
                            .transition(
                                id,
                                &[SessionStatus::Stopping, SessionStatus::Running],
                                doc! {
                                    "status": status_bson(SessionStatus::Failed),
                                    "error": truncate_error(&format!("engine stop failed: {error}")),
                                    "stopped_at": now(),
                                },
                            )
                            .await;
                    }
                }
                return;
            }
            _ = tick.tick() => {
                let live = session.status();
                if live == LiveStatus::Running {
                    continue;
                }
                // Terminal engine state. A commanded stop converges to
                // `stopped`; an uncommanded exit (even a clean one) is a
                // failure of the supervised contract.
                if *stop_rx.borrow() {
                    // `running` in the from-set: a shutdown-driven stop
                    // signal never CAS'd the document to `stopping`.
                    let _ = inner
                        .repo
                        .transition(
                            id,
                            &[SessionStatus::Stopping, SessionStatus::Running],
                            doc! {
                                "status": status_bson(SessionStatus::Stopped),
                                "stopped_at": now(),
                            },
                        )
                        .await;
                    tracing::info!(session_id = %id, "session stopped (engine exit after stop request)");
                    return;
                }
                let error = describe_exit(live, &session);
                tracing::warn!(session_id = %id, ?live, "engine exited uncommanded");
                let failed = inner
                    .repo
                    .transition(
                        id,
                        &[SessionStatus::Running],
                        doc! {
                            "status": status_bson(SessionStatus::Failed),
                            "error": truncate_error(&error),
                            "stopped_at": now(),
                        },
                    )
                    .await;
                if matches!(failed, Ok(None)) {
                    // A stop request slipped in between the exit and the
                    // CAS: honor it as a stop, not a failure.
                    let _ = inner
                        .repo
                        .transition(
                            id,
                            &[SessionStatus::Stopping],
                            doc! {
                                "status": status_bson(SessionStatus::Stopped),
                                "stopped_at": now(),
                            },
                        )
                        .await;
                }
                // Reap/cleanup the dead engine's dirs.
                let _ = inner.runner.stop(&mut session).await;
                return;
            }
        }
    }
}

/// Converge a pre-`running` failure: CAS `validating|stopping -> failed`
/// with the (truncated) error and a stop timestamp.
async fn fail_session(inner: &Inner, id: bson::Uuid, error: &str) {
    let result = inner
        .repo
        .transition(
            id,
            &[SessionStatus::Validating, SessionStatus::Stopping],
            doc! {
                "status": status_bson(SessionStatus::Failed),
                "error": truncate_error(error),
                "stopped_at": bson::DateTime::now(),
            },
        )
        .await;
    match result {
        Ok(Some(_)) => tracing::info!(session_id = %id, "session failed"),
        Ok(None) => tracing::warn!(session_id = %id, "fail CAS missed; session already terminal"),
        Err(err) => tracing::error!(session_id = %id, error = %err, "fail CAS errored"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_keeps_short_text_verbatim() {
        assert_eq!(truncate_error("boom"), "boom");
    }

    #[test]
    fn truncate_error_caps_long_text_at_a_char_boundary() {
        let long = "α".repeat(MAX_ERROR_BYTES); // 2 bytes per char
        let truncated = truncate_error(&long);
        assert!(truncated.ends_with(" [truncated]"));
        assert!(truncated.len() <= MAX_ERROR_BYTES + " [truncated]".len());
        // Still valid UTF-8 by construction (String), no panic on slicing.
    }
}
