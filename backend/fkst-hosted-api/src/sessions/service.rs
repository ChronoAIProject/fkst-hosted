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
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use bson::{doc, Document};
use secrecy::SecretString;
use tokio::sync::watch;

use crate::distribution::{Distributor, DriverHost, PlacementError};
use crate::engine::config::is_reserved_env_key;
use crate::engine::{
    EngineConfig, GoalContext, LiveStatus, PreparedPackage, RunnerError, RunningSession,
    SessionRunner, StartSpec,
};
use crate::error::AppError;
use crate::github_app::GithubAppTokens;
use crate::goals::{GoalRepo, GoalStatus, RepoRef};
use crate::journal::model::LogRef;
use crate::journal::parse::{parse_raised_line, ParsedLine};
use crate::journal::store::MongoProgressStore;
use crate::journal::{
    package_fingerprint, JournalConfig, Journaler, LifecycleEvent, ProgressSignal, SessionCtx,
    Transition,
};
use crate::leases::RenewOutcome;
use crate::models::{SessionDoc, SessionStatus};
use crate::nyxid::NyxIdClient;
use crate::packages::{is_valid_name, PackageRepository};
use crate::sessions::nyxid_token::{self, NyxidTokenHandle};
use crate::sessions::repo::{status_bson, SessionRepo};
use crate::vault::{EnvScopeRef, VaultService};

/// Maximum stored byte length of a `package_name` (matches the packages
/// domain bound).
const MAX_PACKAGE_NAME_BYTES: usize = 128;

/// Ownership information stamped onto a new session.
pub struct SessionOwner {
    pub owner_user_id: String,
    pub org_id: Option<String>,
}

/// Information needed to create a session from a goal trigger. The handler
/// resolves and validates this data before passing it here.
pub struct GoalTriggerInfo {
    pub goal_id: bson::Uuid,
    pub repo: RepoRef,
    pub package_names: Vec<String>,
    pub owner_user_id: String,
    pub org_id: Option<String>,
    /// The prior goal status before trigger (captured for compensating CAS).
    pub prior_status: GoalStatus,
}

/// Outcome of a successful `create_for_goal` call.
pub struct GoalTriggerResult {
    pub session_id: bson::Uuid,
    pub goal_status: GoalStatus,
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

/// Default token refresh interval: mint a fresh GitHub installation token
/// every 55 minutes (tokens expire after ~60 min, 5 min buffer).
const TOKEN_REFRESH_INTERVAL: Duration = Duration::from_secs(55 * 60);

/// Minimum cooldown between token refresh attempts (even on failure, do not
/// hammer the GitHub API more often than once per minute).
const TOKEN_REFRESH_COOLDOWN: Duration = Duration::from_secs(60);

/// Per-session goal drive state: tracks token lifecycle and goal association
/// for a goal-triggered session running inside the driver's supervise loop.
struct GoalDrive {
    goal_id: bson::Uuid,
    repo: RepoRef,
    token_path: PathBuf,
    minted_at: std::time::Instant,
    last_attempt: std::time::Instant,
}

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
    /// Goal support layer (issue #63), enabled once at startup via
    /// [`SessionService::enable_goal_support`]. Set before any session is
    /// created; a second call is a logged no-op.
    goal_support: OnceLock<GoalSupport>,
    /// Per-session env vault (issue #102), enabled once at startup via
    /// [`SessionService::enable_vault`]. Unset => the driver resolves an EMPTY
    /// env profile (legacy tests / minimal runs behave exactly as pre-#102);
    /// production always enables it. A second call is a logged no-op.
    vault: OnceLock<VaultService>,
    /// Per-session NyxID token provisioning (issue #111), enabled once at
    /// startup via [`SessionService::enable_nyxid_token`]. Unset => the driver
    /// skips token provisioning entirely (legacy tests / minimal runs behave
    /// exactly as pre-#111). When set it carries the NyxID client and the
    /// origin (the NyxID issuer base URL) injected as `NYXID_URL`.
    nyxid: OnceLock<NyxidSetup>,
}

/// NyxID token provisioning wiring shared by every driver this service spawns.
struct NyxidSetup {
    client: NyxIdClient,
    /// The NyxID origin the engine talks to, injected as `NYXID_URL`.
    origin: String,
}

/// Journaling wiring shared by every driver this service spawns.
struct JournalSetup {
    config: JournalConfig,
    store: MongoProgressStore,
}

/// Goal support wiring shared by every driver this service spawns.
struct GoalSupport {
    goals: GoalRepo,
    github_app: GithubAppTokens,
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
                goal_support: OnceLock::new(),
                vault: OnceLock::new(),
                nyxid: OnceLock::new(),
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

    /// Enable goal-support features for this service: best-effort goal-status
    /// sync writes and token refresh inside the driver loop. Call once at
    /// startup, after construction but before any goal-triggered session is
    /// created.
    pub fn enable_goal_support(&self, goals: GoalRepo, github_app: GithubAppTokens) {
        if self
            .inner
            .goal_support
            .set(GoalSupport { goals, github_app })
            .is_err()
        {
            tracing::warn!("goal support already enabled; ignoring the second call");
            return;
        }
        tracing::info!("goal support enabled (goal-status sync + token refresh)");
    }

    /// Enable per-session env injection (issue #102): the driver resolves each
    /// session's vault scope into an `env_profile` and injects it into the
    /// engine run. Call once at startup, before any session is created; a
    /// second call is a logged no-op. When never called the driver resolves an
    /// empty profile (legacy behaviour).
    pub fn enable_vault(&self, vault: VaultService) {
        if self.inner.vault.set(vault).is_err() {
            tracing::warn!("vault already enabled; ignoring the second call");
            return;
        }
        tracing::info!("session env injection enabled (vault wired into the driver)");
    }

    /// Enable per-session NyxID token provisioning (issue #111): the driver
    /// mints a per-session agent key on the triggering user's behalf and
    /// injects it as `NYXID_ACCESS_TOKEN` (+ `NYXID_URL`) into the engine env,
    /// then revokes it at teardown. `origin` is the NyxID issuer base URL.
    /// Call once at startup, before any session is created; a second call is a
    /// logged no-op. When never called the driver skips provisioning entirely
    /// (legacy behaviour for tests / minimal runs).
    pub fn enable_nyxid_token(&self, client: NyxIdClient, origin: String) {
        if self.inner.nyxid.set(NyxidSetup { client, origin }).is_err() {
            tracing::warn!("nyxid token provisioning already enabled; ignoring the second call");
            return;
        }
        tracing::info!("per-session nyxid token provisioning enabled (wired into the driver)");
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
        raw_token: SecretString,
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
            // Package sessions have no repo, so their env resolves from the
            // owner-wide (global) vault scope (#102). The non-secret pointer is
            // persisted so a failover rebuild re-resolves the same scope.
            env_scope: Some(EnvScopeRef::global()),
            triggered_by: None,
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            created_at: bson::DateTime::now(),
            started_at: None,
            stopped_at: None,
        };
        self.inner.repo.insert(&session).await?;

        let Some(distributor) = self.inner.distribution.clone() else {
            // Single-pod posture: spawn the detached driver inline, no lease.
            // The user's raw token rides into the detached task transiently so
            // the driver can mint the per-session NyxID key (#111).
            self.spawn_driver(&session, Some(raw_token));
            return Ok(session);
        };

        match distributor.place(package_name, session.id).await {
            Ok(placement) if placement.pod_id == distributor.pod_id() => {
                let mut owned = session.clone();
                owned.pod_id = Some(placement.pod_id);
                owned.fencing_token = Some(placement.fencing_token);
                self.spawn_driver(&owned, Some(raw_token));
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

    /// Create a session from a goal trigger. Handles steps 4-8 of the trigger
    /// flow:
    /// 4. Goal CAS: not_started/stopped/failed -> triggered
    /// 5. Insert SessionDoc (pending)
    /// 6. Place via distributor
    /// 7. Set active_session_id on goal
    /// 8. Return result
    ///
    /// On failure after step 4, a compensating CAS returns the goal to its
    /// prior status.
    pub async fn create_for_goal(
        &self,
        goals: &GoalRepo,
        trigger: GoalTriggerInfo,
        raw_token: SecretString,
    ) -> Result<GoalTriggerResult, AppError> {
        let now = bson::DateTime::now();

        // Step 4: Goal CAS — not_started/stopped/failed -> triggered.
        let triggerable = [
            GoalStatus::NotStarted,
            GoalStatus::Stopped,
            GoalStatus::Failed,
        ];
        let repo_bson = bson::to_bson(&Some(trigger.repo.clone())).expect("RepoRef serializes");
        let cas_set = doc! {
            "status": bson::to_bson(&GoalStatus::Triggered).expect("GoalStatus serializes"),
            "repo": repo_bson,
            "updated_at": now,
        };
        let _goal_after_cas = goals
            .transition_status(trigger.goal_id, &triggerable, cas_set)
            .await?
            .ok_or_else(|| AppError::Conflict("goal already triggered or running".to_string()))?;

        // Step 5: Insert SessionDoc (pending).
        let first_package = trigger.package_names.first().cloned().unwrap_or_default();
        let session = SessionDoc {
            id: bson::Uuid::new(),
            package_name: first_package,
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            run_key: None,
            owner_user_id: Some(trigger.owner_user_id.clone()),
            org_id: trigger.org_id.clone(),
            package_names: trigger.package_names.clone(),
            goal_id: Some(trigger.goal_id),
            repo: Some(trigger.repo.clone()),
            // Goal sessions resolve their env from the target repo's vault
            // scope (which overlays the owner-wide global scope, repo winning
            // on a key collision — see VaultService::list_for_scope). #102.
            env_scope: Some(EnvScopeRef::repo(&trigger.repo.owner, &trigger.repo.name)),
            triggered_by: Some("goal-trigger".to_string()),
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            created_at: now,
            started_at: None,
            stopped_at: None,
        };
        if let Err(insert_err) = self.inner.repo.insert(&session).await {
            // Compensating CAS: return goal to prior status.
            let _ = goals
                .transition_status(
                    trigger.goal_id,
                    &[GoalStatus::Triggered],
                    doc! {
                        "status": bson::to_bson(&trigger.prior_status).expect("GoalStatus serializes"),
                        "updated_at": bson::DateTime::now(),
                    },
                )
                .await;
            return Err(insert_err);
        }

        // Step 6: Place via distributor (if configured).
        if let Some(ref distributor) = self.inner.distribution {
            let lease_key = session.lease_key();
            match distributor.place(&lease_key, session.id).await {
                Ok(placement) if placement.pod_id == distributor.pod_id() => {
                    // This pod was chosen; spawn the driver. The user's raw
                    // token rides into the detached task transiently so the
                    // driver can mint the per-session NyxID key (#111).
                    let mut owned = session.clone();
                    owned.pod_id = Some(placement.pod_id);
                    owned.fencing_token = Some(placement.fencing_token);
                    self.spawn_driver(&owned, Some(raw_token.clone()));
                }
                Ok(_placement) => {
                    // Another pod was chosen; its reaper picks it up.
                    tracing::info!(
                        session_id = %session.id,
                        goal_id = %trigger.goal_id,
                        "goal session placed on another pod"
                    );
                }
                Err(PlacementError::AlreadyRunning(_)) => {
                    // Conflict: converge the just-inserted pending doc to
                    // failed and compensate the goal CAS.
                    let _ = self
                        .inner
                        .repo
                        .transition(
                            session.id,
                            &[SessionStatus::Pending],
                            doc! {
                                "status": status_bson(SessionStatus::Failed),
                                "error": "goal already has a live session",
                                "stopped_at": bson::DateTime::now(),
                            },
                        )
                        .await;
                    let _ = goals
                        .transition_status(
                            trigger.goal_id,
                            &[GoalStatus::Triggered],
                            doc! {
                                "status": bson::to_bson(&trigger.prior_status).expect("GoalStatus serializes"),
                                "updated_at": bson::DateTime::now(),
                            },
                        )
                        .await;
                    return Err(AppError::Conflict(
                        "goal already has a live session".to_string(),
                    ));
                }
                Err(PlacementError::NoCapacity) => {
                    // Retriable: session stays pending; the reaper retries.
                    tracing::warn!(
                        session_id = %session.id,
                        goal_id = %trigger.goal_id,
                        "no capacity at goal trigger; session stays pending for the reaper"
                    );
                }
                Err(error) => {
                    // Transient failure: session stays pending; the reaper
                    // retries placement.
                    tracing::error!(
                        session_id = %session.id,
                        goal_id = %trigger.goal_id,
                        error = %error,
                        "placement failed at goal trigger; session stays pending for the reaper"
                    );
                }
            }
        } else {
            // Single-pod posture: spawn the driver inline.
            self.spawn_driver(&session, Some(raw_token.clone()));
        }

        // Step 7: Set active_session_id on goal (CAS guarded to triggered).
        let active_set = goals.set_active_session(trigger.goal_id, session.id).await;
        if !active_set.unwrap_or(false) {
            // The goal may have been concurrently modified; the session is
            // already created and will be picked up by the driver/reaper. Log
            // but do not fail the trigger.
            tracing::warn!(
                goal_id = %trigger.goal_id,
                session_id = %session.id,
                "active_session_id CAS missed; session is still created"
            );
        }

        // Step 8: Return result.
        tracing::info!(
            goal_id = %trigger.goal_id,
            session_id = %session.id,
            "goal triggered successfully"
        );
        Ok(GoalTriggerResult {
            session_id: session.id,
            goal_status: GoalStatus::Triggered,
        })
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
    ///
    /// `raw_token` is the triggering user's first-party access token, threaded
    /// transiently into the detached task so the driver can mint the
    /// per-session NyxID key (#111). It is `Some` only on the create/trigger
    /// path (where a live HTTP request supplied it) and `None` on the
    /// reaper/failover seam (which rebuilds purely from `SessionDoc`, with no
    /// user token in hand). It is NEVER persisted.
    fn spawn_driver(&self, session: &SessionDoc, raw_token: Option<SecretString>) {
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
        let goal_info = session.goal_id.map(|goal_id| {
            let repo = session.repo.clone().expect("goal session must have repo");
            GoalDrive {
                goal_id,
                repo,
                // Token path: <runtime_dir>/github-token (set once the
                // engine starts and the runtime dir is known; until then
                // this placeholder is never written).
                token_path: PathBuf::new(),
                minted_at: std::time::Instant::now(),
                last_attempt: std::time::Instant::now(),
            }
        });
        tokio::spawn(async move {
            drive(
                Arc::clone(&inner),
                id,
                package_name,
                stop_rx,
                fencing_token,
                goal_info,
                raw_token,
            )
            .await;
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
        // The reaper rebuilds from SessionDoc alone — no user token in hand.
        // `None` makes the driver take the failover branch (#111): re-mint is
        // impossible without a live token, so a session that previously had a
        // NyxID key ESCALATES rather than running with broken auth.
        self.spawn_driver(session, None);
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
    mut goal_info: Option<GoalDrive>,
    raw_token: Option<SecretString>,
) {
    // Holds the per-session NyxID key handle once the driver provisions it, so
    // `drive` can revoke it at teardown regardless of which exit path
    // `drive_inner` took. `drive_inner` writes it via this out-parameter
    // (mirroring how `goal_info` is threaded) to avoid rewriting its many
    // early returns.
    let mut nyxid_handle: Option<NyxidTokenHandle> = None;
    let release = drive_inner(
        &inner,
        id,
        &package_name,
        stop_rx,
        fencing_token,
        &mut goal_info,
        raw_token,
        &mut nyxid_handle,
    )
    .await;

    // Teardown revoke (#111): best-effort revoke of the per-session NyxID key
    // on EVERY terminal driver exit, including a lease loss / self-fence — the
    // key was minted by THIS driver and is revoked by its id, so revoking it
    // here cannot touch a takeover pod's freshly-minted key. A revoke failure
    // is logged and swallowed (the key is non-expiring; a sweep is the
    // backstop) so it never blocks lease release below.
    if let (Some(setup), Some(handle)) = (inner.nyxid.get(), &nyxid_handle) {
        nyxid_token::revoke(&setup.client, handle).await;
    }

    if !release {
        return;
    }
    if let (Some(distributor), Some(token)) = (&inner.distribution, fencing_token) {
        // Use lease_key for the release: for goal sessions the lease key is
        // "goal-<uuid>", for classic sessions it's the package_name.
        let lease_key = match &goal_info {
            Some(gi) => format!("goal-{}", gi.goal_id),
            None => package_name.clone(),
        };
        if let Err(error) = distributor.leases().release(&lease_key, token).await {
            tracing::error!(
                session_id = %id,
                lease_key = %lease_key,
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
#[allow(clippy::too_many_arguments)]
async fn drive_inner(
    inner: &Arc<Inner>,
    id: bson::Uuid,
    package_name: &str,
    mut stop_rx: watch::Receiver<bool>,
    fencing_token: Option<i64>,
    goal_info: &mut Option<GoalDrive>,
    raw_token: Option<SecretString>,
    nyxid_handle: &mut Option<NyxidTokenHandle>,
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
            fail_session(
                inner,
                id,
                &fence,
                "package disappeared before start",
                goal_info,
            )
            .await;
            return true;
        }
        Err(error) => {
            // The driver error (a Mongo failure) can carry internal
            // topology/connection detail: log it, never persist it into the
            // client-served `error` field.
            tracing::error!(session_id = %id, error = %error, "driver failed to load package");
            fail_session(inner, id, &fence, "failed to load package", goal_info).await;
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

    // (B3) Resolve the per-session env profile from the vault BEFORE the engine
    // starts (#102). The full SessionDoc carries the owner/org/scope needed to
    // resolve; a resolve error (e.g. a decrypt failure) FAILS the start — a run
    // must never proceed missing its secrets. Re-fetched (not threaded from
    // spawn) so a failover rebuild re-resolves with any rotated secret.
    let session_doc = match inner.repo.get(id).await {
        Ok(Some(session_doc)) => session_doc,
        Ok(None) => {
            // The document vanished between the claim and here (a concurrent
            // delete). Nothing to fail; just exit (the lease is still released).
            tracing::warn!(session_id = %id, "session document vanished before env resolution");
            return true;
        }
        Err(error) => {
            tracing::error!(session_id = %id, error = %error, "driver failed to load session for env resolution");
            fail_session(inner, id, &fence, "failed to load session", goal_info).await;
            return true;
        }
    };
    let mut env_profile = match resolve_env_profile(inner, &session_doc).await {
        Ok(profile) => profile,
        Err(error) => {
            // Host-side detail (decrypt internals) stays in the logs; the
            // served `error` field is generic. The secret value is never
            // logged — only the failure is.
            tracing::error!(session_id = %id, error = %error, "failed to resolve session secrets");
            fail_session(
                inner,
                id,
                &fence,
                "failed to resolve session secrets",
                goal_info,
            )
            .await;
            return true;
        }
    };
    if !env_profile.is_empty() {
        // Keys are non-secret env-var NAMES, safe to log; VALUES are never
        // logged (they are `SecretString`). Counts + names aid debugging which
        // env a run received without ever exposing the secret material.
        tracing::info!(
            session_id = %id,
            count = env_profile.len(),
            keys = %env_profile.keys().cloned().collect::<Vec<_>>().join(","),
            "injected session env"
        );
    }

    // (B4) Per-session NyxID token (#111): mint ONE non-expiring agent key on
    // the triggering user's behalf and merge NYXID_ACCESS_TOKEN + NYXID_URL into
    // the env_profile so the engine's `nyxid` CLI and codex provider act as that
    // user. Only runs when provisioning is wired (production); skipped for
    // legacy tests / minimal runs. A mint failure FAILS the start — a run must
    // never proceed without the credential its consumers expect. On a failover
    // rebuild (raw_token is None) for a session that previously had a key, the
    // token cannot be re-minted (no live user token on the rebuild pod), so per
    // the ESCALATE-ONLY v1 policy the session is failed rather than run with
    // broken auth.
    if let Some(setup) = inner.nyxid.get() {
        match &raw_token {
            Some(token) => {
                match nyxid_token::provision(&setup.client, id, &setup.origin, token).await {
                    Ok((handle, entries)) => {
                        // The two entries are SECRETS (the key) and a non-secret
                        // origin; neither is reserved, so both survive the engine
                        // env filter. Merge into the profile the run starts with.
                        for (key, value) in entries {
                            env_profile.insert(key, value);
                        }
                        // Persist ONLY the non-secret refs so a teardown/sweep
                        // can find the key; the full key is NEVER written. CAS is
                        // pinned by the fence so a superseded driver cannot stamp.
                        if let Err(error) = inner
                            .repo
                            .transition_guarded(
                                id,
                                &[SessionStatus::Validating],
                                fence.clone(),
                                doc! {
                                    "nyxid_key_id": &handle.key_id,
                                    "nyxid_key_prefix": &handle.key_prefix,
                                },
                            )
                            .await
                        {
                            // The key was minted but the ref could not be
                            // persisted: fail the start AND revoke immediately so
                            // we never leak an unreferenced key.
                            tracing::error!(session_id = %id, error = %error, "failed to persist nyxid key ref");
                            nyxid_token::revoke(&setup.client, &handle).await;
                            fail_session(
                                inner,
                                id,
                                &fence,
                                "failed to persist nyxid session credential reference",
                                goal_info,
                            )
                            .await;
                            return true;
                        }
                        // Hand the handle to `drive` for teardown revoke.
                        *nyxid_handle = Some(handle);
                    }
                    Err(error) => {
                        // `provision` already logged the precise cause (and never
                        // the token); the served `error` field stays generic.
                        tracing::error!(session_id = %id, error = %error, "failed to provision nyxid session token");
                        fail_session(
                            inner,
                            id,
                            &fence,
                            "failed to provision nyxid session credential",
                            goal_info,
                        )
                        .await;
                        return true;
                    }
                }
            }
            None => {
                if session_doc.nyxid_key_id.is_some() {
                    // ESCALATE: a failover rebuild with no user token cannot
                    // re-establish the NyxID session identity. Never run with
                    // broken auth.
                    tracing::error!(
                        session_id = %id,
                        "cannot re-establish nyxid session token on failover; no user access token available"
                    );
                    fail_session(
                        inner,
                        id,
                        &fence,
                        "cannot re-establish NyxID session token on failover; no user access token available",
                        goal_info,
                    )
                    .await;
                    return true;
                }
                // No prior key (this session never had one) and no token to mint
                // with: proceed without a NyxID token, unchanged behaviour.
                tracing::debug!(
                    session_id = %id,
                    "no user token on this driver and no prior nyxid key; skipping token provisioning"
                );
            }
        }
    }

    // (C0) For a goal session, mint the GitHub App installation token and build
    // the GoalContext so the engine receives GITHUB_TOKEN + the 0600 token file
    // + goal.json at t=0 (#106 — previously the token only arrived ~55 min in,
    // if ever). Minting here (rather than threading the trigger-time preflight
    // token across the detached driver boundary) unifies the initial start and
    // the failover rebuild: both rebuild the GoalContext purely from the
    // SessionDoc + a fresh mint, and `token_for_repo` caches per (repo, perms),
    // so this is a cache hit after the trigger preflight — not a second network
    // mint. A mint/goal-load failure FAILS the start: a goal run must never
    // proceed without its credential.
    let goal_ctx = match goal_info.as_mut() {
        Some(gi) => match build_goal_context(inner, gi).await {
            Ok(ctx) => Some(ctx),
            Err(reason) => {
                tracing::error!(session_id = %id, reason = %reason, "failed to prepare goal context");
                journal_finish(
                    &mut journaler,
                    Transition::Failed {
                        exit_code: None,
                        error: reason.clone(),
                    },
                )
                .await;
                fail_session(inner, id, &fence, &reason, goal_info).await;
                return true;
            }
        },
        None => None,
    };

    // (C) Start the engine. This await runs to completion (never
    // select-cancelled); a stop that raced in is honored right after. For a
    // goal session `goal_ctx` is `Some(..)`, so `start_with_spec` writes the
    // token file + goal.json and `spawn_supervise` sets the GitHub env vars
    // before the engine runs.
    let mut session = match inner
        .runner
        .start_with_spec(&StartSpec {
            packages: vec![prepared],
            goal: goal_ctx,
            env_profile,
        })
        .await
    {
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
            fail_session(inner, id, &fence, &describe_runner_error(&error), goal_info).await;
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

    // Goal-status sync: triggered -> running (best-effort CAS).
    if let Some(ref gi) = goal_info {
        goal_status_sync(
            inner,
            gi.goal_id,
            id,
            &[GoalStatus::Triggered],
            GoalStatus::Running,
        )
        .await;
    }

    // Update goal drive token_path now that the runtime dir is known.
    if let Some(ref mut gi) = goal_info {
        gi.token_path = session.runtime_dir.join("github-token");
    }

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
                        // Goal-status sync: {triggered,running} -> stopped.
                        if let Some(ref gi) = goal_info {
                            goal_status_sync(
                                inner,
                                gi.goal_id,
                                id,
                                &[GoalStatus::Triggered, GoalStatus::Running],
                                GoalStatus::Stopped,
                            )
                            .await;
                        }
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
                // Use the lease key (goal-<uuid> for goal sessions, package_name
                // for classic) for renewal.
                let lease_key = match goal_info {
                    Some(gi) => format!("goal-{}", gi.goal_id),
                    None => package_name.to_string(),
                };
                match distributor.leases().renew(&lease_key, token).await {
                    Ok(RenewOutcome::Renewed(_)) => {
                        last_renew_success = tokio::time::Instant::now();
                    }
                    Ok(RenewOutcome::Lost) => {
                        // Fenced out: a takeover pod owns the lease and the
                        // document now. Kill the local engine and exit
                        // WITHOUT writing status and WITHOUT releasing.
                        tracing::warn!(
                            session_id = %id,
                            lease_key = %lease_key,
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
                                lease_key = %lease_key,
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
                            lease_key = %lease_key,
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

                // Token refresh for goal sessions: mint a fresh GitHub
                // installation token when the current one is nearing expiry
                // and the cooldown since the last attempt has passed.
                if let Some(ref mut gi) = goal_info {
                    refresh_goal_token(inner, gi).await;
                }

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
                    // Goal-status sync: {triggered,running} -> stopped.
                    if let Some(ref gi) = goal_info {
                        goal_status_sync(
                            inner,
                            gi.goal_id,
                            id,
                            &[GoalStatus::Triggered, GoalStatus::Running],
                            GoalStatus::Stopped,
                        )
                        .await;
                    }
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
                // Goal-status sync: {triggered,running} -> failed.
                if let Some(ref gi) = goal_info {
                    goal_status_sync(
                        inner,
                        gi.goal_id,
                        id,
                        &[GoalStatus::Triggered, GoalStatus::Running],
                        GoalStatus::Failed,
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

/// Resolve the per-session env profile from the vault (#102). Returns an empty
/// map when no vault is wired (legacy tests / minimal runs behave exactly as
/// pre-#102). Otherwise it derives the scope — the persisted `env_scope` when
/// present, else a fallback derived from `repo` so legacy documents without
/// `env_scope` still resolve — and asks the vault for that scope's resolved
/// entries (secrets already decrypted into `SecretString`). Any key the
/// platform reserves (`is_reserved_env_key`) is dropped as defense in depth
/// (the vault write-validator already rejects them at write time). A decrypt
/// error propagates as `Err` so the caller fails the start rather than running
/// a session missing its secrets.
async fn resolve_env_profile(
    inner: &Arc<Inner>,
    session: &SessionDoc,
) -> Result<BTreeMap<String, SecretString>, AppError> {
    let Some(vault) = inner.vault.get() else {
        return Ok(BTreeMap::new());
    };

    let scope = scope_for_session(session);
    // The owner anchors the lookup; a pre-auth document with no owner resolves
    // to no entries (the empty owner can hold none), which is the safe answer.
    let owner_user_id = session.owner_user_id.as_deref().unwrap_or_default();
    let resolved = vault
        .list_for_scope(owner_user_id, session.org_id.as_deref(), &scope)
        .await?;
    Ok(profile_from_resolved(session.id, resolved))
}

/// Derive the vault scope to resolve for a session: the persisted non-secret
/// `env_scope` pointer wins; otherwise fall back to deriving it from `repo`
/// (repo-scope) or `global`, so a pre-#102 document without `env_scope` still
/// resolves the correct scope on a redrive.
fn scope_for_session(session: &SessionDoc) -> EnvScopeRef {
    session
        .env_scope
        .clone()
        .unwrap_or_else(|| match &session.repo {
            Some(repo) => EnvScopeRef::repo(&repo.owner, &repo.name),
            None => EnvScopeRef::global(),
        })
}

/// Build the engine `env_profile` from the vault's resolved entries, dropping
/// any platform-reserved key (`is_reserved_env_key`) as defense in depth — the
/// vault write-validator already rejects them, so a present reserved key is an
/// anomaly worth a warn. Keys (non-secret names) are logged; values never are.
fn profile_from_resolved(
    session_id: bson::Uuid,
    resolved: Vec<crate::vault::ResolvedEntry>,
) -> BTreeMap<String, SecretString> {
    let mut profile = BTreeMap::new();
    let mut dropped_reserved = 0usize;
    for entry in resolved {
        if is_reserved_env_key(&entry.key) {
            dropped_reserved += 1;
            continue;
        }
        profile.insert(entry.key, entry.value);
    }
    if dropped_reserved > 0 {
        tracing::warn!(
            session_id = %session_id,
            dropped_reserved,
            "dropped reserved env keys from the resolved session profile"
        );
    }
    profile
}

/// Converge a pre-`running` failure: CAS `validating|stopping -> failed`
/// with the (truncated) error and a stop timestamp, pinned by the fence.
/// Also performs best-effort goal-status sync ({triggered,running} -> failed)
/// when the session is a goal session.
async fn fail_session(
    inner: &Inner,
    id: bson::Uuid,
    fence: &Document,
    error: &str,
    goal_info: &Option<GoalDrive>,
) {
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
    // Goal-status sync: {triggered,running} -> failed (best-effort).
    if let Some(ref gi) = goal_info {
        goal_status_sync(
            inner,
            gi.goal_id,
            id,
            &[GoalStatus::Triggered, GoalStatus::Running],
            GoalStatus::Failed,
        )
        .await;
    }
}

// ---------------------------------------------------------------------------
// Goal-status sync + token refresh (issue #63). Best-effort CAS writes to the
// goals collection, logged and swallowed — never load-bearing. Token refresh
// writes <token_path>.tmp then renames atomically.
// ---------------------------------------------------------------------------

/// Best-effort CAS transition of a goal's status. The CAS is guarded with
/// `active_session_id == session_id` so a newer trigger is never clobbered.
/// All errors are logged and swallowed.
async fn goal_status_sync(
    inner: &Inner,
    goal_id: bson::Uuid,
    session_id: bson::Uuid,
    from_statuses: &[GoalStatus],
    target: GoalStatus,
) {
    let Some(gs) = inner.goal_support.get() else {
        return;
    };
    let goals = &gs.goals;
    let from_bson: Vec<bson::Bson> = from_statuses
        .iter()
        .map(|s| bson::to_bson(s).expect("GoalStatus serializes"))
        .collect();
    let filter = doc! {
        "_id": goal_id,
        "status": { "$in": from_bson },
        "active_session_id": session_id,
    };
    let update = doc! {
        "$set": {
            "status": bson::to_bson(&target).expect("GoalStatus serializes"),
            "updated_at": bson::DateTime::now(),
        }
    };
    match goals.transition_raw(filter, update).await {
        Ok(true) => tracing::info!(
            goal_id = %goal_id,
            session_id = %session_id,
            target = ?target,
            "goal-status sync applied"
        ),
        Ok(false) => tracing::debug!(
            goal_id = %goal_id,
            session_id = %session_id,
            "goal-status sync CAS missed (concurrent change)"
        ),
        Err(error) => tracing::warn!(
            goal_id = %goal_id,
            session_id = %session_id,
            error = %error,
            "goal-status sync write failed (swallowed)"
        ),
    }
}

/// Refresh the GitHub installation token for a goal session. Minted fresh
/// when the refresh interval has elapsed and the cooldown since the last
/// attempt has passed. Writes `<token_path>.tmp` then `fs::rename` for
/// atomicity on the same filesystem. Failures are WARN'd; the engine keeps
/// using the previous token.
/// Build the [`GoalContext`] for a goal session: load the goal (for its
/// title/description, written into `goal.json`) and mint a fresh GitHub App
/// installation token for its repo. Used IDENTICALLY by the initial start and
/// the failover rebuild — the token is never persisted, always (re-)minted
/// here. `token_for_repo` caches per `(repo, perms)`, so right after the
/// trigger-time install-check preflight this is a cache hit, not a second
/// network mint.
///
/// Records the actual mint instant on the `GoalDrive` so the periodic refresh
/// fires ~55 min later (5 min before the 60-min TTL) rather than being
/// suppressed for 55 min from the pre-seeded driver-creation `minted_at`
/// (the #106 refresh-seed bug).
async fn build_goal_context(inner: &Arc<Inner>, gi: &mut GoalDrive) -> Result<GoalContext, String> {
    let Some(gs) = inner.goal_support.get() else {
        return Err("goal support not enabled; cannot mint github token".to_string());
    };
    let goal = match gs.goals.get(gi.goal_id).await {
        Ok(Some(goal)) => goal,
        Ok(None) => return Err("goal not found for session".to_string()),
        Err(error) => {
            tracing::error!(goal_id = %gi.goal_id, error = %error, "failed to load goal for context");
            return Err("failed to load goal".to_string());
        }
    };
    let repo_ref = format!("{}/{}", gi.repo.owner, gi.repo.name);
    let token = match gs.github_app.token_for_repo(&repo_ref, None).await {
        Ok(token) => token,
        Err(error) => {
            // The error may carry the install URL hint (NotInstalled); log it,
            // never the token. The trigger preflight already validated install,
            // so a failure here is transient/edge — fail the start; #107 adds
            // the retry/pause hardening on top.
            tracing::error!(goal_id = %gi.goal_id, repo = %repo_ref, error = %error, "failed to mint github token at startup");
            return Err("failed to mint github token for the goal repo".to_string());
        }
    };
    // The token is in use from t=0; reset the refresh clock to this instant.
    gi.minted_at = std::time::Instant::now();
    Ok(GoalContext {
        goal_id: gi.goal_id,
        title: goal.title,
        description: goal.description,
        repo: gi.repo.clone(),
        github_token: token,
    })
}

async fn refresh_goal_token(inner: &Inner, drive: &mut GoalDrive) {
    let now = std::time::Instant::now();
    if drive.minted_at.elapsed() < TOKEN_REFRESH_INTERVAL {
        return;
    }
    if drive.last_attempt.elapsed() < TOKEN_REFRESH_COOLDOWN {
        return;
    }
    drive.last_attempt = now;

    let Some(gs) = inner.goal_support.get() else {
        return;
    };
    let github_app = &gs.github_app;

    if drive.token_path.as_os_str().is_empty() {
        // Runtime dir not yet established (engine not started).
        return;
    }

    let repo_ref = format!("{}/{}", drive.repo.owner, drive.repo.name);
    match github_app.token_for_repo(&repo_ref, None).await {
        Ok(token) => {
            let tmp_path = drive.token_path.with_extension("tmp");
            use secrecy::ExposeSecret;
            let token_str = token.expose_secret();
            match tokio::fs::write(&tmp_path, token_str.as_bytes()).await {
                Ok(()) => match tokio::fs::rename(&tmp_path, &drive.token_path).await {
                    Ok(()) => {
                        drive.minted_at = std::time::Instant::now();
                        tracing::info!(
                            goal_id = %drive.goal_id,
                            "github token refreshed"
                        );
                    }
                    Err(error) => {
                        tracing::warn!(
                            goal_id = %drive.goal_id,
                            path = %drive.token_path.display(),
                            error = %error,
                            "token rename failed; retrying next tick"
                        );
                        let _ = tokio::fs::remove_file(&tmp_path).await;
                    }
                },
                Err(error) => {
                    tracing::warn!(
                        goal_id = %drive.goal_id,
                        path = %tmp_path.display(),
                        error = %error,
                        "token tmp write failed; retrying next tick"
                    );
                }
            }
        }
        Err(error) => {
            tracing::warn!(
                goal_id = %drive.goal_id,
                repo = %repo_ref,
                error = %error,
                "token mint failed; engine keeps previous token"
            );
        }
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

    // ---- per-session env injection (issue #102) -------------------------------

    use crate::models::RepoRef as ModelRepoRef;
    use crate::vault::ResolvedEntry;
    use secrecy::ExposeSecret;

    /// A minimal `SessionDoc` for the env-resolution helper tests. Only the
    /// fields the helpers read (`id`, `owner_user_id`, `org_id`, `repo`,
    /// `env_scope`) matter; the rest are inert defaults.
    fn env_test_session() -> SessionDoc {
        SessionDoc {
            id: bson::Uuid::new(),
            package_name: "demo".to_string(),
            status: SessionStatus::Pending,
            pod_id: None,
            fencing_token: None,
            pid: None,
            runtime_dir: None,
            error: None,
            run_key: None,
            owner_user_id: Some("user-1".to_string()),
            org_id: None,
            package_names: vec![],
            goal_id: None,
            repo: None,
            env_scope: None,
            triggered_by: None,
            nyxid_key_id: None,
            nyxid_key_prefix: None,
            created_at: bson::DateTime::now(),
            started_at: None,
            stopped_at: None,
        }
    }

    #[test]
    fn scope_for_session_prefers_the_persisted_env_scope() {
        let mut session = env_test_session();
        // A repo is set but the persisted env_scope must win regardless.
        session.repo = Some(ModelRepoRef {
            owner: "acme".to_string(),
            name: "other".to_string(),
        });
        session.env_scope = Some(EnvScopeRef::repo("acme", "site"));
        assert_eq!(scope_for_session(&session).scope_key(), "repo:acme/site");
    }

    #[test]
    fn scope_for_session_falls_back_to_repo_when_env_scope_absent() {
        // Legacy doc (no env_scope) with a repo resolves to that repo's scope.
        let mut session = env_test_session();
        session.repo = Some(ModelRepoRef {
            owner: "acme".to_string(),
            name: "billing".to_string(),
        });
        assert_eq!(scope_for_session(&session).scope_key(), "repo:acme/billing");
    }

    #[test]
    fn scope_for_session_falls_back_to_global_when_neither_present() {
        // Legacy doc with neither env_scope nor repo resolves owner-wide.
        let session = env_test_session();
        assert_eq!(scope_for_session(&session).scope_key(), "global");
    }

    #[test]
    fn profile_from_resolved_keeps_ordinary_keys_and_drops_reserved() {
        let id = bson::Uuid::new();
        let resolved = vec![
            ResolvedEntry {
                key: "OPENAI_API_KEY".to_string(),
                value: SecretString::from("sk-secret".to_string()),
            },
            ResolvedEntry {
                key: "FOO".to_string(),
                value: SecretString::from("bar".to_string()),
            },
            // Reserved keys must be dropped (defense in depth): a platform
            // prefix, an explicit reserved name, and an allow-listed host var.
            ResolvedEntry {
                key: "FKST_DURABLE_ROOT".to_string(),
                value: SecretString::from("x".to_string()),
            },
            ResolvedEntry {
                key: "GITHUB_TOKEN".to_string(),
                value: SecretString::from("y".to_string()),
            },
            ResolvedEntry {
                key: "PATH".to_string(),
                value: SecretString::from("z".to_string()),
            },
        ];
        let profile = profile_from_resolved(id, resolved);
        assert_eq!(
            profile.keys().cloned().collect::<Vec<_>>(),
            vec!["FOO".to_string(), "OPENAI_API_KEY".to_string()],
            "only the non-reserved keys survive, key-sorted"
        );
        assert_eq!(
            profile.get("OPENAI_API_KEY").map(|v| v.expose_secret()),
            Some("sk-secret")
        );
        assert!(!profile.contains_key("FKST_DURABLE_ROOT"));
        assert!(!profile.contains_key("GITHUB_TOKEN"));
        assert!(!profile.contains_key("PATH"));
    }

    #[test]
    fn profile_from_resolved_is_empty_for_no_entries() {
        let profile = profile_from_resolved(bson::Uuid::new(), Vec::new());
        assert!(profile.is_empty());
    }

    #[tokio::test]
    async fn resolve_env_profile_is_empty_when_no_vault_wired() {
        // A service that never had `enable_vault` called resolves an EMPTY
        // profile — the pre-#102 behaviour for tests / minimal runs. The Db
        // handle is built lazily and never touched (no vault => early return),
        // so this needs no live Mongo.
        let db = crate::db::Db {
            database: mongodb::Client::with_uri_str("mongodb://localhost:27017")
                .await
                .expect("client")
                .database("sessions_unit_test"),
        };
        let service = SessionService::new(
            SessionRepo::new(&db),
            PackageRepository::new(&db.database),
            EngineConfig::default(),
        );
        let session = env_test_session();
        let profile = resolve_env_profile(&service.inner, &session)
            .await
            .expect("empty profile, no error");
        assert!(
            profile.is_empty(),
            "no vault wired => empty env profile (legacy behaviour)"
        );
    }
}
