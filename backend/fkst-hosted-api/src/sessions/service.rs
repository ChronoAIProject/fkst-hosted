//! Session orchestration: one detached driver task per session supervises
//! one engine process and advances the session document through
//! `pending -> validating -> running -> stopping -> stopped | failed`.
//!
//! Two postures, selected at construction:
//! - **Single-pod (legacy, [`SessionService::new`])**: no lease is taken;
//!   the driver is spawned inline on create. Used by tests and
//!   non-distributed runs; behavior is unchanged from v1.
//! - **Distributed ([`SessionService::with_distribution`])**: create runs
//!   [`Distributor::place`] (lease acquire + ownership write) and only
//!   spawns a local driver when this pod was chosen; a live lease for
//!   another session surfaces as `409 Conflict`. The driver is **fenced**:
//!   its claim CAS pins `pod_id` + `fencing_token`, it renews the package
//!   lease on an interval and self-terminates WITHOUT touching the document
//!   when the lease is lost (a takeover pod owns the session now), and it
//!   releases the lease on every terminal exit.
//!
//! Concurrency rules (load-bearing):
//! - Every status write goes through the repository CAS
//!   ([`SessionRepo::transition`] / [`SessionRepo::transition_guarded`]); a
//!   CAS miss means a concurrent stop or takeover won and the driver
//!   converges instead of overwriting.
//! - The in-memory registry `Mutex` is sync and NEVER held across an await;
//!   [`SessionService::ensure_driver`]-style spawns are entry-guarded so two
//!   racing spawn requests start exactly one driver.
//! - `SessionRunner::start` is awaited to completion, never select-cancelled
//!   (a cancelled start would leak intent mid-spawn; stop-vs-start races are
//!   resolved AFTER start returns, via the `validating -> running` CAS).

use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use bson::{doc, Document};
use tokio::sync::watch;

use crate::distribution::{Distributor, DriverHost, PlacementError};
use crate::engine::{
    EngineConfig, LiveStatus, PreparedPackage, RunnerError, RunningSession, SessionRunner,
};
use crate::error::AppError;
use crate::leases::RenewOutcome;
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

/// Renew "interval" used when a driver has no lease to renew (single-pod
/// posture): effectively never fires meaningfully, and the arm no-ops.
const NO_LEASE_RENEW_INTERVAL: Duration = Duration::from_secs(86_400);

/// Shared internals behind the clonable service handle.
struct Inner {
    repo: SessionRepo,
    packages: PackageRepository,
    runner: SessionRunner,
    /// Per-session stop signal; entries are inserted by the entry-guarded
    /// spawn and removed by the owning driver task on every exit path. The
    /// lock is sync and never held across an await.
    registry: Mutex<HashMap<bson::Uuid, watch::Sender<bool>>>,
    /// Pod identity stamped onto sessions this pod drives. With
    /// distribution this is the distributor's pod id; without, the advisory
    /// `HOSTNAME` (or `null`).
    pod_id: Option<String>,
    /// Bound for the shutdown drain (engine stop grace + headroom).
    shutdown_bound: Duration,
    /// Placement + lease layer; `None` selects the single-pod posture.
    distribution: Option<Distributor>,
}

/// Clonable orchestration service: create / get / stop sessions and drive
/// their engine processes on this pod.
#[derive(Clone)]
pub struct SessionService {
    inner: Arc<Inner>,
}

impl SessionService {
    /// Build the single-pod service (no lease coordination; v1 behavior).
    /// `pod_id` is taken from `HOSTNAME` when present (the Kubernetes pod
    /// name); absent locally, the advisory field stays `null` until the
    /// first driver write.
    pub fn new(repo: SessionRepo, packages: PackageRepository, engine: EngineConfig) -> Self {
        let pod_id = std::env::var("HOSTNAME").ok();
        Self::build(repo, packages, engine, pod_id, None)
    }

    /// Build the distributed service: create places sessions through the
    /// distributor, drivers are fenced by the package lease, and the pod
    /// identity is the distributor's.
    pub fn with_distribution(
        repo: SessionRepo,
        packages: PackageRepository,
        engine: EngineConfig,
        distributor: Distributor,
    ) -> Self {
        let pod_id = Some(distributor.pod_id().to_string());
        Self::build(repo, packages, engine, pod_id, Some(distributor))
    }

    fn build(
        repo: SessionRepo,
        packages: PackageRepository,
        engine: EngineConfig,
        pod_id: Option<String>,
        distribution: Option<Distributor>,
    ) -> Self {
        let shutdown_bound = Duration::from_secs(engine.stop_grace_secs + SHUTDOWN_HEADROOM_SECS);
        Self {
            inner: Arc::new(Inner {
                repo,
                packages,
                runner: SessionRunner::new(engine),
                registry: Mutex::new(HashMap::new()),
                pod_id,
                shutdown_bound,
                distribution,
            }),
        }
    }

    /// The repository handle (startup hooks: orphan sweep).
    pub fn repo(&self) -> &SessionRepo {
        &self.inner.repo
    }

    /// Create a session for `package_name`: validate, check the package
    /// exists, insert the `pending` document, then hand off — single-pod:
    /// spawn the driver inline; distributed: place the session (the chosen
    /// pod's driver picks it up; a live lease for another session is a
    /// `409`). Returns the created document immediately (the driver
    /// advances it asynchronously).
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

        let Some(distributor) = self.inner.distribution.clone() else {
            // Single-pod posture: spawn the detached driver inline, no lease.
            self.spawn_driver(&session);
            return Ok(session);
        };

        match distributor.place(package_name, session.id).await {
            Ok(placement) if placement.pod_id == distributor.pod_id() => {
                let mut owned = session.clone();
                owned.pod_id = Some(placement.pod_id);
                owned.fencing_token = Some(placement.fencing_token);
                self.spawn_driver(&owned);
                Ok(session)
            }
            Ok(placement) => {
                // Another pod was chosen; its reaper picks the session up.
                tracing::info!(
                    session_id = %session.id,
                    package_name = %package_name,
                    pod_id = %placement.pod_id,
                    "session placed on another pod"
                );
                Ok(session)
            }
            Err(PlacementError::AlreadyRunning(_)) => {
                // Conflict: converge the just-inserted pending doc to failed
                // so no zombie pending document lingers, then 409.
                let _ = self
                    .inner
                    .repo
                    .transition(
                        session.id,
                        &[SessionStatus::Pending],
                        doc! {
                            "status": status_bson(SessionStatus::Failed),
                            "error": "package already has a live session",
                            "stopped_at": bson::DateTime::now(),
                        },
                    )
                    .await;
                tracing::info!(
                    session_id = %session.id,
                    package_name = %package_name,
                    "session create conflicts with a live lease"
                );
                Err(AppError::Conflict(format!(
                    "package {package_name} already has a live session"
                )))
            }
            Err(PlacementError::NoCapacity) => {
                // Retriable: the session stays pending; the reaper retries
                // placement on its next tick.
                tracing::warn!(
                    session_id = %session.id,
                    package_name = %package_name,
                    "no capacity at create; session stays pending for the reaper"
                );
                Ok(session)
            }
            Err(error) => {
                // Transient infrastructure failure: the session stays
                // pending and unassigned; the reaper retries placement.
                tracing::error!(
                    session_id = %session.id,
                    package_name = %package_name,
                    error = %error,
                    "placement failed at create; session stays pending for the reaper"
                );
                Ok(session)
            }
        }
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

    /// Entry-guarded driver spawn: start a detached driver task for
    /// `session` unless one is already registered. The driver's claim CAS
    /// (and, when fenced, the lease) stays the authoritative dedupe; this
    /// guard only prevents a second in-process task from clobbering the
    /// first one's stop channel.
    fn spawn_driver(&self, session: &SessionDoc) {
        let (stop_tx, stop_rx) = watch::channel(false);
        {
            let mut registry = self
                .inner
                .registry
                .lock()
                .expect("session registry lock poisoned");
            match registry.entry(session.id) {
                Entry::Occupied(_) => {
                    tracing::debug!(
                        session_id = %session.id,
                        "driver already live; spawn skipped"
                    );
                    return;
                }
                Entry::Vacant(vacant) => {
                    vacant.insert(stop_tx);
                }
            }
        }

        let inner = Arc::clone(&self.inner);
        let id = session.id;
        let package_name = session.package_name.clone();
        let fencing_token = session.fencing_token;
        tokio::spawn(async move {
            drive(Arc::clone(&inner), id, package_name, stop_rx, fencing_token).await;
            inner
                .registry
                .lock()
                .expect("session registry lock poisoned")
                .remove(&id);
            tracing::debug!(session_id = %id, "driver task exited");
        });
        tracing::info!(
            session_id = %session.id,
            package_name = %session.package_name,
            fenced = session.fencing_token.is_some(),
            "session driver spawned"
        );
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

/// The reaper's seam: ensure a local driver runs for a session this pod
/// owns (placement scan or post-takeover redo). Idempotent via the registry
/// entry guard; the claim CAS dedupes across process restarts.
#[async_trait]
impl DriverHost for SessionService {
    async fn ensure_driver(&self, session: &SessionDoc) {
        self.spawn_driver(session);
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

/// Drive one session, then settle the package lease: every terminal exit
/// releases the lease this driver held (equality-pinned, so an already
/// taken-over lease is an untouched `NotHeld`); a lost lease releases
/// NOTHING (the takeover pod owns lease and document now).
async fn drive(
    inner: Arc<Inner>,
    id: bson::Uuid,
    package_name: String,
    stop_rx: watch::Receiver<bool>,
    fencing_token: Option<i64>,
) {
    let release = drive_inner(&inner, id, &package_name, stop_rx, fencing_token).await;
    if !release {
        return;
    }
    if let (Some(distributor), Some(token)) = (&inner.distribution, fencing_token) {
        if let Err(error) = distributor.leases().release(&package_name, token).await {
            tracing::error!(
                session_id = %id,
                package_name = %package_name,
                token,
                error = %error,
                "lease release failed on driver exit; the lease will lapse"
            );
        }
    }
}

/// The driver state machine for one session. Every status write is a CAS
/// (pinned to this pod + fencing token when fenced); every exit path leaves
/// the document terminal or owned by whoever won the race. Returns whether
/// the caller should release the lease (`false` only when the lease was
/// LOST — the document belongs to a takeover pod and must not be touched).
async fn drive_inner(
    inner: &Arc<Inner>,
    id: bson::Uuid,
    package_name: &str,
    mut stop_rx: watch::Receiver<bool>,
    fencing_token: Option<i64>,
) -> bool {
    let now = bson::DateTime::now;

    // Ownership pins for every CAS this driver performs: with a fencing
    // token the filter requires the document to still carry this pod's
    // identity AND this token, so a superseded driver's write can never
    // land after a takeover rebound the session.
    let fence: Document = match (&inner.pod_id, fencing_token) {
        (Some(pod_id), Some(token)) => doc! { "pod_id": pod_id, "fencing_token": token },
        _ => Document::new(),
    };

    // (A) pending -> validating, stamping this pod's identity.
    let claimed = inner
        .repo
        .transition_guarded(
            id,
            &[SessionStatus::Pending],
            fence.clone(),
            doc! {
                "status": status_bson(SessionStatus::Validating),
                "pod_id": inner.pod_id.as_deref().map(bson::Bson::from).unwrap_or(bson::Bson::Null),
            },
        )
        .await;
    match claimed {
        Ok(Some(_)) => {}
        Ok(None) => {
            // Raced: a stop arrived before we claimed, the doc vanished, or
            // (fenced) a takeover rebound it to another pod/token.
            match inner.repo.get(id).await {
                Ok(Some(session)) if session.status == SessionStatus::Stopping => {
                    let _ = inner
                        .repo
                        .transition_guarded(
                            id,
                            &[SessionStatus::Stopping],
                            fence.clone(),
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
            return true;
        }
        Err(error) => {
            tracing::error!(session_id = %id, error = %error, "driver failed to claim session");
            return true;
        }
    }

    // (B) Load the package (it may have been deleted since create).
    let package = match inner.packages.get(package_name).await {
        Ok(Some(package)) => package,
        Ok(None) => {
            fail_session(inner, id, &fence, "package disappeared before start").await;
            return true;
        }
        Err(error) => {
            // The driver error (a Mongo failure) can carry internal
            // topology/connection detail: log it, never persist it into the
            // client-served `error` field.
            tracing::error!(session_id = %id, error = %error, "driver failed to load package");
            fail_session(inner, id, &fence, "failed to load package").await;
            return true;
        }
    };
    let prepared = PreparedPackage::from(package);

    // (C) Start the engine. This await runs to completion (never
    // select-cancelled); a stop that raced in is honored right after.
    let mut session = match inner.runner.start(&prepared).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(session_id = %id, error = %error, "engine start failed");
            fail_session(inner, id, &fence, &describe_runner_error(&error)).await;
            return true;
        }
    };

    let stop_already_requested = *stop_rx.borrow();
    let promoted = if stop_already_requested {
        None
    } else {
        inner
            .repo
            .transition_guarded(
                id,
                &[SessionStatus::Validating],
                fence.clone(),
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
            .transition_guarded(
                id,
                &[SessionStatus::Stopping, SessionStatus::Validating],
                fence.clone(),
                doc! {
                    "status": status_bson(SessionStatus::Stopped),
                    "stopped_at": now(),
                },
            )
            .await;
        return true;
    }
    tracing::info!(session_id = %id, pid = session.pid, "session running");

    // (D) Supervise: react to stop requests, renew the lease, and watch
    // engine liveness.
    let mut tick = tokio::time::interval(SUPERVISE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let renew_interval = match (&inner.distribution, fencing_token) {
        (Some(distributor), Some(_)) => distributor.config().renew_interval,
        _ => NO_LEASE_RENEW_INTERVAL,
    };
    let mut renew_tick = tokio::time::interval(renew_interval);
    renew_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
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
                            .transition_guarded(
                                id,
                                &[SessionStatus::Stopping, SessionStatus::Running],
                                fence.clone(),
                                doc! {
                                    "status": status_bson(SessionStatus::Stopped),
                                    "stopped_at": now(),
                                },
                            )
                            .await;
                        tracing::info!(session_id = %id, "session stopped");
                    }
                    Err(error) => {
                        // Host-side detail (paths, signalling internals)
                        // stays in the logs; the served field is generic.
                        tracing::error!(session_id = %id, error = %error, "engine stop failed");
                        let _ = inner
                            .repo
                            .transition_guarded(
                                id,
                                &[SessionStatus::Stopping, SessionStatus::Running],
                                fence.clone(),
                                doc! {
                                    "status": status_bson(SessionStatus::Failed),
                                    "error": "engine stop failed",
                                    "stopped_at": now(),
                                },
                            )
                            .await;
                    }
                }
                return true;
            }
            _ = renew_tick.tick() => {
                let (Some(distributor), Some(token)) = (&inner.distribution, fencing_token) else {
                    continue;
                };
                match distributor.leases().renew(package_name, token).await {
                    Ok(RenewOutcome::Renewed(_)) => {}
                    Ok(RenewOutcome::Lost) => {
                        // Fenced out: a takeover pod owns the lease and the
                        // document now. Kill the local engine and exit
                        // WITHOUT writing status and WITHOUT releasing.
                        tracing::warn!(
                            session_id = %id,
                            package_name = %package_name,
                            token,
                            "package lease lost; stopping the local engine without document writes"
                        );
                        if let Err(error) = inner.runner.stop(&mut session).await {
                            tracing::error!(session_id = %id, error = %error, "engine stop failed after lease loss");
                        }
                        return false;
                    }
                    Err(error) => {
                        // Transient: keep supervising; the TTL gives at
                        // least one more renewal window.
                        tracing::error!(
                            session_id = %id,
                            package_name = %package_name,
                            error = %error,
                            "lease renew errored; retrying on the next interval"
                        );
                    }
                }
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
                        .transition_guarded(
                            id,
                            &[SessionStatus::Stopping, SessionStatus::Running],
                            fence.clone(),
                            doc! {
                                "status": status_bson(SessionStatus::Stopped),
                                "stopped_at": now(),
                            },
                        )
                        .await;
                    tracing::info!(session_id = %id, "session stopped (engine exit after stop request)");
                    return true;
                }
                let error = describe_exit(live, &session);
                tracing::warn!(session_id = %id, ?live, "engine exited uncommanded");
                let failed = inner
                    .repo
                    .transition_guarded(
                        id,
                        &[SessionStatus::Running],
                        fence.clone(),
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
                        .transition_guarded(
                            id,
                            &[SessionStatus::Stopping],
                            fence.clone(),
                            doc! {
                                "status": status_bson(SessionStatus::Stopped),
                                "stopped_at": now(),
                            },
                        )
                        .await;
                }
                // Reap/cleanup the dead engine's dirs.
                let _ = inner.runner.stop(&mut session).await;
                return true;
            }
        }
    }
}

/// Converge a pre-`running` failure: CAS `validating|stopping -> failed`
/// with the (truncated) error and a stop timestamp, pinned by the fence.
async fn fail_session(inner: &Inner, id: bson::Uuid, fence: &Document, error: &str) {
    let result = inner
        .repo
        .transition_guarded(
            id,
            &[SessionStatus::Validating, SessionStatus::Stopping],
            fence.clone(),
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
