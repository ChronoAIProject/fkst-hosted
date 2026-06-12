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
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use bson::{doc, Document};
use tokio::sync::watch;

use crate::distribution::{Distributor, DriverHost, PlacementError};
use crate::engine::{
    EngineConfig, LiveStatus, PreparedPackage, RunnerError, RunningSession, SessionRunner,
};
use crate::error::AppError;
use crate::journal::model::LogRef;
use crate::journal::parse::{parse_raised_line, ParsedLine};
use crate::journal::store::MongoProgressStore;
use crate::journal::{
    package_fingerprint, JournalConfig, Journaler, LifecycleEvent, ProgressSignal, SessionCtx,
    Transition,
};
use crate::leases::RenewOutcome;
use crate::models::{SessionDoc, SessionStatus};
use crate::packages::{is_valid_name, PackageRepository};
use crate::sessions::repo::{status_bson, SessionRepo};

/// Maximum stored byte length of a `package_name` (matches the packages
/// domain bound).
const MAX_PACKAGE_NAME_BYTES: usize = 128;

/// Ownership information stamped onto a new session.
pub struct SessionOwner {
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

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
    /// Journaling layer (issue #25), enabled once at startup via
    /// [`SessionService::enable_journaling`]. Unset => journaling is off
    /// (legacy tests / minimal runs); the driver behaves identically either
    /// way — journaling NEVER changes session disposition.
    journal: OnceLock<JournalSetup>,
}

/// Journaling wiring shared by every driver this service spawns.
struct JournalSetup {
    config: JournalConfig,
    store: MongoProgressStore,
}

/// The concrete journaler type drivers hold.
type ServiceJournaler = Journaler<MongoProgressStore>;

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
                journal: OnceLock::new(),
            }),
        }
    }

    /// Enable session-progress journaling (issue #25) for every driver this
    /// service spawns. Call once at startup, before any session is created;
    /// a second call is a logged no-op.
    pub fn enable_journaling(&self, config: JournalConfig, store: MongoProgressStore) {
        let github = config.github_enabled && config.github_repo.is_some();
        if self
            .inner
            .journal
            .set(JournalSetup { config, store })
            .is_err()
        {
            tracing::warn!("journaling already enabled; ignoring the second call");
            return;
        }
        tracing::info!(github, "session progress journaling enabled");
    }

    /// The repository handle (startup hooks: orphan sweep).
    pub fn repo(&self) -> &SessionRepo {
        &self.inner.repo
    }

    /// Create a session for `package_name` with the given ownership:
    /// validate, check the package exists, insert the `pending` document,
    /// then hand off — single-pod: spawn the driver inline; distributed:
    /// place the session (the chosen pod's driver picks it up; a live lease
    /// for another session is a `409`). Returns the created document
    /// immediately (the driver advances it asynchronously).
    ///
    /// TOCTOU between route-level authorize and service-level create is
    /// benign: the package is read again inside `exists` and the owner is
    /// stamped from the caller's context, not re-read from the package.
    pub async fn create(
        &self,
        package_name: &str,
        owner: SessionOwner,
    ) -> Result<SessionDoc, AppError> {
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
            run_key: None,
            owner_user_id: Some(owner.owner_user_id),
            org_id: owner.org_id,
            package_names: vec![],
            goal_id: None,
            repo: None,
            triggered_by: None,
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

/// Pure self-fencing decision for the renew arm: with no successful renewal
/// for longer than the lease TTL, this driver can no longer prove it holds
/// the lease — the lease has certainly lapsed on the server (the last
/// successful renew set `expires_at = then + ttl`) and another pod may have
/// taken over, so a renew ERROR must now be treated exactly like an
/// observed `Lost`. At exactly the TTL boundary the lease is only just
/// dead and no takeover (which itself needs a write AFTER expiry plus the
/// grace window) can have spawned, so strictly-greater keeps one final
/// retry without risking a dual engine.
fn renew_overdue(since_last_success: Duration, lease_ttl: Duration) -> bool {
    since_last_success > lease_ttl
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

    // (B2) Journaling bootstrap (issue #25): resolve run_key, stamp it on
    // the document, and load the GitHub skip-set BEFORE any package work.
    // Journaling is never load-bearing — every failure below is logged and
    // swallowed; session disposition is decided exclusively by the CAS
    // choke-points.
    let mut journaler = start_journaler(inner, id, package_name, &package, fencing_token).await;
    journal_lifecycle(&mut journaler, Transition::Validating).await;

    let prepared = PreparedPackage::from(package);

    // (C) Start the engine. This await runs to completion (never
    // select-cancelled); a stop that raced in is honored right after.
    let mut session = match inner.runner.start(&prepared).await {
        Ok(session) => session,
        Err(error) => {
            tracing::warn!(session_id = %id, error = %error, "engine start failed");
            journal_finish(
                &mut journaler,
                Transition::Failed {
                    exit_code: None,
                    error: describe_runner_error(&error),
                },
            )
            .await;
            fail_session(inner, id, &fence, &describe_runner_error(&error)).await;
            return true;
        }
    };
    journal_lifecycle(&mut journaler, Transition::Spawned { pid: session.pid }).await;

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
        journal_finish(&mut journaler, Transition::Stopped { exit_code: None }).await;
        return true;
    }
    tracing::info!(session_id = %id, pid = session.pid, "session running");
    journal_lifecycle(&mut journaler, Transition::Running).await;
    journal_watermark(&mut journaler, &session.runtime_dir).await;

    // The journal's RAISED source: the engine's line-framed stdout, taken
    // exactly once. Leaving it untaken would be safe too (the drain task
    // keeps the pipe flowing); `None` after EOF parks the select arm.
    let mut stdout_rx = session.take_stdout();

    // (D) Supervise: react to stop requests, renew the lease, consume the
    // stdout journal stream, and watch engine liveness.
    let mut tick = tokio::time::interval(SUPERVISE_POLL);
    tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let renew_interval = match (&inner.distribution, fencing_token) {
        (Some(distributor), Some(_)) => distributor.config().renew_interval,
        _ => NO_LEASE_RENEW_INTERVAL,
    };
    let mut renew_tick = tokio::time::interval(renew_interval);
    renew_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Self-fencing clock: the lease was live when this driver claimed the
    // session, so the success window starts now. Renew ERRORS (Mongo
    // unreachable) leave us blind: past the TTL the lease may have lapsed
    // and been taken over, so the engine must die as if the lease were
    // observed Lost — never run past what we can prove we hold.
    let mut last_renew_success = tokio::time::Instant::now();
    loop {
        tokio::select! {
            // A closed channel (service dropped) also reads as a stop: this
            // pod is going away and the engine must not be orphaned silently.
            line = next_stdout_line(&mut stdout_rx) => {
                match line {
                    Some(raw) => journal_stdout_line(&mut journaler, &raw).await,
                    // EOF/closed: park this arm (recv on a closed channel
                    // answers instantly — polling it again would busy-loop).
                    None => stdout_rx = None,
                }
            }
            _ = stop_rx.changed() => {
                tracing::info!(session_id = %id, "stop signal received; stopping engine");
                journal_lifecycle(&mut journaler, Transition::Stopping).await;
                // Watermark BEFORE stop: stop() removes the runtime dirs.
                journal_watermark(&mut journaler, &session.runtime_dir).await;
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
                        journal_finish(&mut journaler, Transition::Stopped { exit_code: None })
                            .await;
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
                        journal_finish(
                            &mut journaler,
                            Transition::Failed {
                                exit_code: None,
                                error: "engine stop failed".to_string(),
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
                    Ok(RenewOutcome::Renewed(_)) => {
                        last_renew_success = tokio::time::Instant::now();
                    }
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
                        // Mongo-side terminal journal only: this writer's
                        // token is superseded, so the journaler's own fence
                        // keeps it off GitHub; local records aid forensics.
                        journal_finish(
                            &mut journaler,
                            Transition::Failed {
                                exit_code: None,
                                error: "package lease lost; superseded by takeover".to_string(),
                            },
                        )
                        .await;
                        return false;
                    }
                    Err(error) => {
                        let lease_ttl = distributor.config().pool.lease_ttl;
                        if renew_overdue(last_renew_success.elapsed(), lease_ttl) {
                            // Sustained failure past the TTL: the lease may
                            // already belong to a takeover pod. SELF-FENCE —
                            // treat it exactly like an observed Lost (kill
                            // the engine, zero document writes, no release).
                            tracing::warn!(
                                session_id = %id,
                                package_name = %package_name,
                                token,
                                error = %error,
                                lease_ttl_secs = lease_ttl.as_secs(),
                                "lease renew failing past the TTL; self-fencing \
                                 (stopping the local engine without document writes)"
                            );
                            if let Err(stop_error) = inner.runner.stop(&mut session).await {
                                tracing::error!(session_id = %id, error = %stop_error, "engine stop failed after renew self-fence");
                            }
                            journal_finish(
                                &mut journaler,
                                Transition::Failed {
                                    exit_code: None,
                                    error: "lease renew failing past the TTL; self-fenced"
                                        .to_string(),
                                },
                            )
                            .await;
                            return false;
                        }
                        // Transient: keep supervising; the TTL still gives
                        // at least one more renewal window.
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
                // Debounced GitHub sync of buffered completions (no-op when
                // empty or inside the debounce window).
                journal_flush(&mut journaler, false).await;
                let live = session.status();
                if live == LiveStatus::Running {
                    continue;
                }
                // Watermark BEFORE the cleanup below removes the dirs.
                journal_watermark(&mut journaler, &session.runtime_dir).await;
                let exit_code = match live {
                    LiveStatus::Stopped => Some(0),
                    LiveStatus::Failed { code, .. } => code,
                    LiveStatus::Running => None,
                };
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
                    journal_finish(&mut journaler, Transition::Stopped { exit_code }).await;
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
                journal_finish(
                    &mut journaler,
                    Transition::Failed {
                        exit_code,
                        error: error.clone(),
                    },
                )
                .await;
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

// ---------------------------------------------------------------------------
// Journaling glue (issue #25). EVERY journaling Result below is logged and
// swallowed: journaling never changes session disposition, and status writes
// stay exclusively inside the repository CAS choke-points above.
// ---------------------------------------------------------------------------

/// Build and bootstrap the per-session journaler: compute the package
/// fingerprint, start the journaler (run head upsert + GitHub client),
/// stamp `run_key` onto the sessions doc, and load the redo skip-set from
/// GitHub truth. `None` when journaling is not enabled or bootstrap failed.
async fn start_journaler(
    inner: &Arc<Inner>,
    id: bson::Uuid,
    package_name: &str,
    package: &crate::packages::Package,
    fencing_token: Option<i64>,
) -> Option<ServiceJournaler> {
    let setup = inner.journal.get()?;
    let ctx = SessionCtx {
        session_id: id.to_string(),
        package_name: package_name.to_string(),
        package_fingerprint: package_fingerprint(&package.files, &package.composed_deps),
        pod_id: inner.pod_id.clone(),
        fencing_token: fencing_token.unwrap_or(0),
    };
    let mut journaler = match Journaler::start(ctx, setup.config.clone(), setup.store.clone()).await
    {
        Ok(journaler) => journaler,
        Err(error) => {
            tracing::error!(
                session_id = %id,
                package_name = %package_name,
                error = %error,
                "journaler start failed; session proceeds unjournaled"
            );
            return None;
        }
    };
    if let Err(error) = inner.repo.set_run_key(id, journaler.run_key()).await {
        tracing::warn!(
            session_id = %id,
            error = %error,
            "run_key stamp failed; journaling continues"
        );
    }
    // Redo bootstrap: GitHub completed[] -> skip-set + local mirror,
    // fail-open to safe re-execution on any unreachability.
    match journaler.load_skip_set().await {
        Ok(skip) => {
            tracing::info!(
                session_id = %id,
                package_name = %package_name,
                run_key = %journaler.run_key(),
                skip_set_size = skip.len(),
                "redo skip-set loaded"
            );
        }
        Err(error) => {
            tracing::warn!(
                session_id = %id,
                error = %error,
                "skip-set bootstrap failed; proceeding with an empty set"
            );
        }
    }
    Some(journaler)
}

/// Record one signal; failures are swallowed (already logged with context by
/// the journaler / store layers).
async fn journal_record(journaler: &mut Option<ServiceJournaler>, signal: ProgressSignal) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.record(signal).await {
            tracing::warn!(error = %error, "journal record failed (swallowed; session unaffected)");
        }
    }
}

/// Debounced/forced GitHub flush; failures are swallowed (the buffer is
/// retained and retried on the next tick; Mongo already holds the records).
async fn journal_flush(journaler: &mut Option<ServiceJournaler>, force: bool) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.flush(force).await {
            tracing::warn!(error = %error, "journal flush failed (swallowed; retried next tick)");
        }
    }
}

/// Record a lifecycle transition and flush promptly (`force=true` — the
/// spec's lifecycle-flushes-immediately rule).
async fn journal_lifecycle(journaler: &mut Option<ServiceJournaler>, transition: Transition) {
    journal_record(
        journaler,
        ProgressSignal::Lifecycle(LifecycleEvent::now(transition)),
    )
    .await;
    journal_flush(journaler, true).await;
}

/// Terminal journal: record the terminal lifecycle + final forced flush
/// (+ the dormant issue-comment mirror); failures swallowed.
async fn journal_finish(journaler: &mut Option<ServiceJournaler>, transition: Transition) {
    if let Some(j) = journaler.as_mut() {
        if let Err(error) = j.finish(LifecycleEvent::now(transition)).await {
            tracing::warn!(error = %error, "journal finish failed (swallowed; session unaffected)");
        }
    }
}

/// Journal a `log_watermark` reference for the newest engine child log (a
/// pointer only — log bodies never enter Mongo). Observed at lifecycle
/// transitions only, never on a hot path.
async fn journal_watermark(
    journaler: &mut Option<ServiceJournaler>,
    runtime_dir: &std::path::Path,
) {
    if journaler.is_none() {
        return;
    }
    if let Some(log_ref) = newest_child_log(runtime_dir) {
        journal_record(
            journaler,
            ProgressSignal::Lifecycle(LifecycleEvent::now(Transition::LogWatermark(log_ref))),
        )
        .await;
    }
}

/// Newest file under `<runtime_dir>/logs/framework-child/`, as a [`LogRef`].
/// `None` is the normal answer for an idle session (child logs appear only
/// after the first dispatched event).
fn newest_child_log(runtime_dir: &std::path::Path) -> Option<LogRef> {
    let dir = runtime_dir.join("logs").join("framework-child");
    let mut newest: Option<(std::time::SystemTime, LogRef)> = None;
    for entry in std::fs::read_dir(&dir).ok()?.flatten() {
        let Ok(meta) = entry.metadata() else { continue };
        if !meta.is_file() {
            continue;
        }
        let Ok(modified) = meta.modified() else {
            continue;
        };
        if newest
            .as_ref()
            .map(|(time, _)| modified > *time)
            .unwrap_or(true)
        {
            newest = Some((
                modified,
                LogRef {
                    path: format!(
                        "logs/framework-child/{}",
                        entry.file_name().to_string_lossy()
                    ),
                    size: meta.len() as i64,
                    modified: bson::DateTime::from_system_time(modified),
                },
            ));
        }
    }
    newest.map(|(_, log_ref)| log_ref)
}

/// One engine stdout line: parse the `RAISED:` framing and journal the
/// outcome (raised event / malformed anomaly / debug-logged chatter).
async fn journal_stdout_line(journaler: &mut Option<ServiceJournaler>, raw: &[u8]) {
    let Some(max_line_bytes) = journaler.as_ref().map(|j| j.config().max_line_bytes) else {
        tracing::debug!(target: "engine.stdout", len = raw.len(), "stdout line (journaling off)");
        return;
    };
    match parse_raised_line(raw, max_line_bytes) {
        ParsedLine::Raised { event_json } => {
            journal_record(journaler, ProgressSignal::Raised { event_json }).await;
            // Debounced: the journaler batches by interval / batch size.
            journal_flush(journaler, false).await;
        }
        ParsedLine::Malformed { excerpt, oversize } => {
            if let Some(j) = journaler.as_mut() {
                j.malformed_raised_total += 1;
                if oversize {
                    j.oversize_raised_total += 1;
                }
                tracing::warn!(
                    oversize,
                    malformed_raised_total = j.malformed_raised_total,
                    oversize_raised_total = j.oversize_raised_total,
                    payload_excerpt = %excerpt,
                    "malformed RAISED line (journaled as anomaly; session continues)"
                );
            }
            journal_record(
                journaler,
                ProgressSignal::Lifecycle(LifecycleEvent::now(Transition::MalformedRaised {
                    detail: excerpt,
                })),
            )
            .await;
        }
        ParsedLine::Other { excerpt } => {
            tracing::debug!(target: "engine.stdout", line_excerpt = %excerpt, "engine stdout chatter");
        }
    }
}

/// Await the next stdout line; a parked (`None`) receiver pends forever so
/// the select arm goes quiet after EOF instead of busy-looping.
async fn next_stdout_line(
    rx: &mut Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
) -> Option<Vec<u8>> {
    match rx {
        Some(receiver) => receiver.recv().await,
        None => std::future::pending().await,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_error_keeps_short_text_verbatim() {
        assert_eq!(truncate_error("boom"), "boom");
    }

    /// The self-fence trips only STRICTLY past the TTL: while the time
    /// since the last successful renew is at or under the TTL the lease
    /// could still be live (keep retrying); one tick past it the engine
    /// must die rather than run unprovably.
    #[test]
    fn renew_overdue_trips_strictly_past_the_ttl() {
        let ttl = Duration::from_secs(30);
        assert!(!renew_overdue(Duration::ZERO, ttl));
        assert!(!renew_overdue(Duration::from_secs(10), ttl));
        assert!(
            !renew_overdue(ttl, ttl),
            "the exact TTL boundary still allows one final retry"
        );
        assert!(renew_overdue(ttl + Duration::from_millis(1), ttl));
        assert!(renew_overdue(Duration::from_secs(120), ttl));
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
