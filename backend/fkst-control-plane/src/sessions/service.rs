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

use crate::controller::{ControllerHandle, PlacementError as ControllerPlacementError};
use crate::distribution::{Distributor, DriverHost, PlacementError};
use crate::engine::config::is_reserved_env_key;
use crate::engine::{
    clone_repo_packages, EngineConfig, GoalContext, LiveStatus, RunnerError, RunningSession,
    SessionRunner, StartSpec,
};
use crate::error::AppError;
use crate::github_app::GithubAppTokens;
use crate::goals::{GoalIssueStore, GoalStatus, RepoRef};
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
use crate::ornn::OrnnClient;
use crate::sessions::codex_provider::{self, AssumeConnected, ChronoLlmCheck};
use crate::sessions::nyxid_token::{self, NyxidTokenHandle};
use crate::sessions::repo::{status_bson, SessionRepo};
use crate::vault::{EnvScopeRef, VaultService};

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
    /// Resolved Ornn skill/skillset pins to inject into the session's codex
    /// (issue #114). Already boundary-validated by the trigger handler. `None`
    /// (or empty) means no skills are pinned (the common case).
    pub ornn_skills: Option<Vec<crate::ornn::OrnnSkillPin>>,
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

/// Tightened cooldown used once the on-disk token is within this margin of its
/// expiry (#107): as the deadline approaches a flaky mint gets retried more
/// aggressively (every 10s instead of every 60s) so a transient GitHub blip
/// does not let the credential lapse silently.
const TOKEN_REFRESH_COOLDOWN_URGENT: Duration = Duration::from_secs(10);

/// Margin before expiry at which the urgent cooldown engages (#107).
const TOKEN_EXPIRY_URGENT_MARGIN: Duration = Duration::from_secs(600);

/// Consecutive mint failures, WITH the token already past expiry, that escalate
/// the session to Failed instead of letting substrate hit a silent 401 (#107).
const MAX_CONSECUTIVE_MINT_FAILURES: u32 = 5;

/// Poll cadence of the JIT mint-request servicer (#107). Faster than the
/// supervise tick so a credential-helper's near-expiry re-mint request is
/// serviced within a git operation's patience window.
const MINT_REQUEST_POLL: Duration = Duration::from_millis(200);

/// Per-session goal drive state: tracks token lifecycle and goal association
/// for a goal-triggered session running inside the driver's supervise loop.
struct GoalDrive {
    goal_id: bson::Uuid,
    repo: RepoRef,
    /// The goal owner (#138): the inline-secret scope key is `(owner, repo)`, so
    /// the driver teardown clears exactly the scope the trigger handler set.
    owner_user_id: String,
    /// The goal's package names, resolved at spawn against the cloned repo's
    /// `<repo>/.fkst/packages/<name>/` (#115). Threaded from the SessionDoc so a
    /// failover rebuild re-resolves the identical set.
    package_names: Vec<String>,
    token_path: PathBuf,
    /// `<runtime_dir>` for this session, known once the engine starts. The JIT
    /// mint-request file (`<token_path>.request`) and the nonce file live under
    /// it; the poller services requests against this dir (#107).
    runtime_dir: PathBuf,
    minted_at: std::time::Instant,
    last_attempt: std::time::Instant,
    /// Expiry of the token currently on disk (#107). Drives the escalating
    /// retry: as expiry approaches the refresh cooldown is tightened so a flaky
    /// mint gets more attempts before the token actually dies.
    token_expires_at: std::time::SystemTime,
    /// Consecutive mint-failure count, reset on every success. A sustained run
    /// of failures with the token already expired transitions the session to a
    /// clear Failed state rather than letting substrate hit a silent 401 (#107).
    consecutive_failures: u32,
}

impl GoalDrive {
    /// The package names to resolve against the cloned repo, falling back to the
    /// single `package_name` for a (legacy) goal session whose doc carried no
    /// `package_names` array.
    fn package_names_or(&self, package_name: &str) -> Vec<String> {
        if self.package_names.is_empty() {
            vec![package_name.to_string()]
        } else {
            self.package_names.clone()
        }
    }
}

/// Shared internals behind the clonable service handle.
struct Inner {
    repo: SessionRepo,
    runner: SessionRunner,
    /// Per-session stop signal; entries are inserted by the entry-guarded
    /// spawn and removed by the owning driver task on every exit path. The
    /// lock is sync and never held across an await.
    registry: Mutex<HashMap<bson::Uuid, watch::Sender<bool>>>,
    /// Controller-held user access tokens, keyed by session id (#138). Inserted
    /// at `create_for_goal` when the trigger carried a token, re-supplied to the
    /// failover driver by [`SessionService::ensure_driver`] so a same-process
    /// worker loss can re-mint the per-session NyxID key, and removed on the
    /// session's terminal exit. Lost on a controller restart — which is exactly
    /// the documented "survive worker loss, NOT controller loss" boundary. The
    /// token is a zeroizing `SecretString`; the lock is sync, never held across
    /// an await.
    session_tokens: Mutex<HashMap<bson::Uuid, SecretString>>,
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
    /// Per-session codex LLM-provider config (issue #112), enabled once at
    /// startup via [`SessionService::enable_codex`]. Unset => the driver does
    /// NOT render a per-session CODEX_HOME (legacy tests / minimal runs behave
    /// exactly as pre-#112). When set it carries the operator-pinned chrono-llm
    /// DEFAULT model + base URL the renderer uses. Rendering also requires the
    /// vault (#100) to be wired so the user's override/connection can be read.
    codex: OnceLock<CodexSetup>,
    /// Per-session Ornn skill injection (issue #114), enabled once at startup
    /// via [`SessionService::enable_ornn`]. Unset => the driver skips injection
    /// entirely (legacy tests / minimal runs behave exactly as pre-#114). When
    /// set it carries the [`OrnnClient`]; injection ALSO requires a per-session
    /// CODEX_HOME (#112) and the session's NyxID token (#111) — without either,
    /// there is nowhere to install or no identity to fetch as, so it is skipped.
    ornn: OnceLock<OrnnClient>,
    /// Controller-backed placement authority (issue #135), enabled once via
    /// [`SessionService::enable_controller`]. Unset => placement goes through
    /// the Mongo `distribution` path (the live path until #143). When set,
    /// new sessions are placed through the in-memory `ClaimMap` and dispatched
    /// to the chosen worker (which pulls + runs the engine — #136). The driver-
    /// lifecycle status writes move onto `ClaimMap::set_status` in #136, when
    /// engine execution physically moves to the worker and this path activates.
    controller: OnceLock<ControllerHandle>,
}

/// Per-session codex provider wiring shared by every driver this service
/// spawns. Carries the operator-pinned chrono-llm DEFAULT values (#112).
struct CodexSetup {
    /// Model the chrono-llm DEFAULT serves (`FKST_HOSTED_CODEX_MODEL`).
    codex_model: String,
    /// NyxID proxy base URL for chrono-llm (`FKST_HOSTED_CHRONO_LLM_BASE_URL`).
    chrono_llm_base_url: String,
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
    goals: GoalIssueStore,
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
    pub fn new(repo: SessionRepo, engine: EngineConfig) -> Self {
        let pod_id = std::env::var("HOSTNAME").ok();
        Self::build(repo, engine, pod_id, None)
    }

    /// Build the distributed service: create places sessions through the
    /// distributor, drivers are fenced by the package lease, and the pod
    /// identity is the distributor's.
    pub fn with_distribution(
        repo: SessionRepo,
        engine: EngineConfig,
        distributor: Distributor,
    ) -> Self {
        let pod_id = Some(distributor.pod_id().to_string());
        Self::build(repo, engine, pod_id, Some(distributor))
    }

    fn build(
        repo: SessionRepo,
        engine: EngineConfig,
        pod_id: Option<String>,
        distribution: Option<Distributor>,
    ) -> Self {
        let shutdown_bound = Duration::from_secs(engine.stop_grace_secs + SHUTDOWN_HEADROOM_SECS);
        Self {
            inner: Arc::new(Inner {
                repo,
                runner: SessionRunner::new(engine),
                registry: Mutex::new(HashMap::new()),
                session_tokens: Mutex::new(HashMap::new()),
                pod_id,
                shutdown_bound,
                distribution,
                journal: OnceLock::new(),
                goal_support: OnceLock::new(),
                vault: OnceLock::new(),
                nyxid: OnceLock::new(),
                codex: OnceLock::new(),
                ornn: OnceLock::new(),
                controller: OnceLock::new(),
            }),
        }
    }

    /// Hold the triggering user's access token in controller memory for
    /// `session_id` (#138). A same-process failover re-supplies it (see
    /// [`Self::held_token`]) so the driver can re-mint the per-session NyxID key;
    /// it is forgotten on the session's terminal exit and lost on a controller
    /// restart (the "survive worker loss, NOT controller loss" boundary).
    pub fn hold_session_token(&self, session_id: bson::Uuid, token: SecretString) {
        self.inner
            .session_tokens
            .lock()
            .expect("session token store poisoned")
            .insert(session_id, token);
    }

    /// The controller-held token for `session_id`, if this process still holds it
    /// (#138). Used by the failover [`Self::ensure_driver`] to decide whether the
    /// per-session NyxID key can be re-minted (`Some`) or the session must
    /// escalate (`None`, after a controller restart).
    pub(crate) fn held_token(&self, session_id: bson::Uuid) -> Option<SecretString> {
        self.inner
            .session_tokens
            .lock()
            .expect("session token store poisoned")
            .get(&session_id)
            .cloned()
    }

    /// Enable controller-backed placement (issue #135): new sessions are placed
    /// through the in-memory claim authority instead of the Mongo distributor.
    /// Call once at startup; a second call is a logged no-op. When never called
    /// the service uses the Mongo `distribution` path (the live path until #143).
    pub fn enable_controller(&self, handle: ControllerHandle) {
        if self.inner.controller.set(handle).is_err() {
            tracing::warn!("controller placement already enabled; ignoring the second call");
            return;
        }
        tracing::info!("controller-backed placement enabled (in-memory claim authority)");
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
    pub fn enable_goal_support(&self, goals: GoalIssueStore, github_app: GithubAppTokens) {
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

    /// Enable per-session codex LLM-provider config (issue #112): every driver
    /// renders a per-session CODEX_HOME `config.toml` selecting the provider
    /// (default chrono-llm; RAW/STRUCTURED vault overrides). `codex_model` and
    /// `chrono_llm_base_url` are the operator-pinned chrono-llm DEFAULT values.
    /// Call once at startup, before any session is created; a second call is a
    /// logged no-op. When never called the driver skips CODEX_HOME rendering
    /// entirely (legacy behaviour for tests / minimal runs).
    pub fn enable_codex(&self, codex_model: String, chrono_llm_base_url: String) {
        if self
            .inner
            .codex
            .set(CodexSetup {
                codex_model,
                chrono_llm_base_url,
            })
            .is_err()
        {
            tracing::warn!("codex provider config already enabled; ignoring the second call");
            return;
        }
        tracing::info!("per-session codex provider config enabled (wired into the driver)");
    }

    /// Enable per-session Ornn skill injection (issue #114): when a session
    /// pins Ornn skills/skillsets, the driver fetches them as the session user
    /// (via the #111 NyxID token through the `ornn-api` proxy) and installs them
    /// into the per-session CODEX_HOME (#112) before the engine spawns. Call
    /// once at startup, before any session is created; a second call is a logged
    /// no-op. When never called the driver skips injection entirely (legacy
    /// behaviour for tests / minimal runs).
    pub fn enable_ornn(&self, client: OrnnClient) {
        if self.inner.ornn.set(client).is_err() {
            tracing::warn!("ornn skill injection already enabled; ignoring the second call");
            return;
        }
        tracing::info!("per-session ornn skill injection enabled (wired into the driver)");
    }

    /// The repository handle (startup hooks: orphan sweep).
    pub fn repo(&self) -> &SessionRepo {
        &self.inner.repo
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
        goals: &GoalIssueStore,
        trigger: GoalTriggerInfo,
        // The triggering user's forwarded access token, or `None` in "headers
        // mode" (no bearer forwarded by the proxy). When `None`, the driver
        // skips per-session NyxID provisioning (#111) and behaves as pre-#111.
        raw_token: Option<SecretString>,
    ) -> Result<GoalTriggerResult, AppError> {
        let now = bson::DateTime::now();

        // Step 4: Goal CAS — not_started/stopped/failed -> triggered.
        let triggerable = [
            GoalStatus::NotStarted,
            GoalStatus::Stopped,
            GoalStatus::Failed,
        ];
        // Step 4: trigger CAS — single-trigger atomicity is the controller
        // claim's job (#135); this only reflects the status label (#137).
        goals
            .transition_status(trigger.goal_id, &triggerable, GoalStatus::Triggered, false)
            .await?
            .ok_or_else(|| AppError::Conflict("goal already triggered or running".to_string()))?;
        // Set the (possibly newly-created) repo on the goal + materialize/refresh
        // its issue. Best-effort: the in-memory repo is set regardless; the issue
        // is the durable mirror (#137).
        let _ = goals.set_repo(trigger.goal_id, &trigger.repo).await;

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
            // Persisted (resolved, non-secret) so a failover rebuild re-injects
            // the identical pin set (#114). Empty is normalized to `None`.
            ornn_skills: trigger.ornn_skills.clone().filter(|p| !p.is_empty()),
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
                    trigger.prior_status,
                    true,
                )
                .await;
            return Err(insert_err);
        }

        // Hold the triggering user's token in controller memory (#138), keyed by
        // session id, BEFORE placement so a near-instant failover takeover finds
        // it. The failover driver re-mints the per-session NyxID key with it; it
        // is cleared on the session's terminal exit, and lost on a controller
        // restart (the documented "worker loss, not controller loss" boundary).
        if let Some(token) = &raw_token {
            self.hold_session_token(session.id, token.clone());
        }

        // Step 6: Place via the controller authority (#135) when enabled, else
        // via the Mongo distributor (the live path until #143).
        if let Some(handle) = self.inner.controller.get() {
            let lease_key = session.lease_key();
            match handle.place(&lease_key, session.id, session.goal_id).await {
                Ok(placement) => {
                    // The chosen worker pulls + runs the engine (#136). Stamp the
                    // owner worker (as pod_id) + the fence so the worker's driver
                    // writes are guarded; no local spawn (the worker runs it).
                    let _ = self
                        .inner
                        .repo
                        .transition(
                            session.id,
                            &[SessionStatus::Pending],
                            doc! {
                                "pod_id": &placement.worker_id,
                                "fencing_token": placement.fencing_id,
                            },
                        )
                        .await;
                    tracing::info!(
                        session_id = %session.id,
                        goal_id = %trigger.goal_id,
                        worker_id = %placement.worker_id,
                        fencing_id = placement.fencing_id,
                        "goal session placed on a worker via the controller (engine runs in #136)"
                    );
                }
                Err(ControllerPlacementError::AlreadyRunning(_)) => {
                    // Conflict: same convergence as the distributor path — fail
                    // the just-inserted pending doc and compensate the goal CAS.
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
                            trigger.prior_status,
                            true,
                        )
                        .await;
                    return Err(AppError::Conflict(
                        "goal already has a live session".to_string(),
                    ));
                }
                Err(error) => {
                    // NoCapacity / invalid / claim: stay pending; retried on the
                    // next tick (mirrors the distributor's stay-pending behaviour).
                    tracing::warn!(
                        session_id = %session.id,
                        goal_id = %trigger.goal_id,
                        error = %error,
                        "controller placement deferred; session stays pending"
                    );
                }
            }
        } else if let Some(ref distributor) = self.inner.distribution {
            let lease_key = session.lease_key();
            match distributor.place(&lease_key, session.id).await {
                Ok(placement) if placement.pod_id == distributor.pod_id() => {
                    // This pod was chosen; spawn the driver. The user's raw
                    // token rides into the detached task transiently so the
                    // driver can mint the per-session NyxID key (#111).
                    let mut owned = session.clone();
                    owned.pod_id = Some(placement.pod_id);
                    owned.fencing_token = Some(placement.fencing_token);
                    self.spawn_driver(&owned, raw_token.clone());
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
                            trigger.prior_status,
                            true,
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
            self.spawn_driver(&session, raw_token.clone());
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

    /// Fail every active session targeting `owner/name` because the GitHub App
    /// was uninstalled from (or had the repo removed from) that repo (issue
    /// #108). The document CAS runs FIRST so the intent is durable, then each
    /// affected driver living on this pod is signalled to stop (best-effort) —
    /// it will also observe the terminal status on its next supervise tick and
    /// converge. Returns the number of sessions transitioned to Failed.
    ///
    /// `reason` is fixed, operator-authored text (never a secret or a webhook
    /// payload value); it becomes the failed session's user-visible error.
    pub async fn fail_for_uninstalled_repo(
        &self,
        owner: &str,
        name: &str,
        reason: &str,
    ) -> Result<u64, AppError> {
        // Snapshot the affected session ids BEFORE the bulk CAS so we can wake
        // their local drivers; the CAS outcome stays authoritative regardless.
        let affected = self.inner.repo.active_ids_for_repo(owner, name).await?;
        let failed = self
            .inner
            .repo
            .fail_active_for_repo(owner, name, reason)
            .await?;
        if failed > 0 {
            let registry = self
                .inner
                .registry
                .lock()
                .expect("session registry lock poisoned");
            for id in &affected {
                if let Some(sender) = registry.get(id) {
                    let _ = sender.send(true);
                }
            }
        }
        Ok(failed)
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
                owner_user_id: session.owner_user_id.clone().unwrap_or_default(),
                // Resolved against the cloned repo at spawn (#115). Threaded from
                // the SessionDoc so a failover rebuild re-resolves the same set.
                package_names: session.effective_package_names(),
                // Token path: <runtime_dir>/github-token (set once the
                // engine starts and the runtime dir is known; until then
                // this placeholder is never written).
                token_path: PathBuf::new(),
                runtime_dir: PathBuf::new(),
                minted_at: std::time::Instant::now(),
                last_attempt: std::time::Instant::now(),
                // Seeded at the unix epoch so the first real mint always rewrites
                // the on-disk expiry; build_goal_context sets the true value.
                token_expires_at: std::time::SystemTime::UNIX_EPOCH,
                consecutive_failures: 0,
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
        // Failover (#138): re-supply the controller-held user token if this
        // process still has it (a same-process worker loss), so the driver can
        // re-mint the per-session NyxID key (B4). It is absent only after a
        // controller restart (or a takeover by a DIFFERENT pod that never held
        // it) — then `None` makes B4 ESCALATE a session that previously minted a
        // key rather than running with broken auth. This is the documented
        // "survive worker loss, NOT controller loss" guarantee.
        let token = self.held_token(session.id);
        self.spawn_driver(session, token);
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
        // Lease LOST: a takeover pod owns the document now. Do NOT clear this
        // pod's in-memory token/secret state here — clearing happens only on a
        // genuine terminal exit (below), so a same-process re-dispatch can still
        // find the held token.
        return;
    }

    // Genuine terminal exit (#138): drop the controller-held token and the inline
    // secrets for this session's scope so secret material does not linger in
    // memory beyond the run. The token map is keyed by session id; the inline
    // secrets are keyed by `(owner, repo)` (a goal session only).
    inner
        .session_tokens
        .lock()
        .expect("session token store poisoned")
        .remove(&id);
    if let (Some(vault), Some(gi)) = (inner.vault.get(), &goal_info) {
        vault.clear_inline(
            &gi.owner_user_id,
            &EnvScopeRef::repo(&gi.repo.owner, &gi.repo.name),
        );
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

    // (B) Resolve the repo-scoped package source (#115). Every session is now a
    // GOAL session: mint the GitHub App installation token, CLONE the goal repo
    // into a per-session project root, and resolve its
    // `<repo>/.fkst/packages/<name>` dirs — the repo-scoped source that replaced
    // the Mongo package store. A clone/resolve failure fails the start with a
    // clear error. The cloned repo's TempDir guards live in THIS frame
    // (`_clone_guard`) so the working tree + the transient clone credential are
    // removed on every exit path, mirroring how `_codex_home_guard` is held.
    let Some(gi) = goal_info.as_mut() else {
        // No goal repo => no package source since #115 (a session loads its
        // packages from its goal repo). This can only be a stale pre-#115
        // document a reaper picked up; fail it loudly rather than run with no
        // packages — sessions are created exclusively via a goal trigger now.
        tracing::error!(
            session_id = %id,
            "session has no goal repo; classic package sessions are unsupported since #115"
        );
        fail_session(
            inner,
            id,
            &fence,
            "session has no goal repo; sessions must be created via a goal trigger",
            goal_info,
        )
        .await;
        return true;
    };
    // Mint the token + build the GoalContext (the clone needs the freshly-minted
    // App installation token; `token_for_repo` caches per (repo, perms), so this
    // is the single mint the trigger preflight primed — not a second network
    // call). This is the failover-safe rebuild point: it works from the
    // SessionDoc + a fresh mint alone, needing no live user token.
    let goal_ctx = match build_goal_context(inner, gi).await {
        Ok(ctx) => ctx,
        Err(reason) => {
            tracing::error!(session_id = %id, reason = %reason, "failed to prepare goal context");
            fail_session(inner, id, &fence, &reason, goal_info).await;
            return true;
        }
    };
    // Clone the repo + resolve the named packages against `<repo>/.fkst/packages/`.
    // The token rides into the credential helper (0600 file), never the clone
    // argv — see engine::clone.
    let names = gi.package_names_or(package_name);
    let cloned = match clone_repo_packages(
        inner.runner.temp_root(),
        &goal_ctx.repo,
        &goal_ctx.github_token,
        &names,
        inner.runner.framework_bin(),
    )
    .await
    {
        Ok(cloned) => cloned,
        Err(error) => {
            // The error is client-safe (repo ref / package name only; never the
            // token); detail is logged inside clone.
            let reason = describe_runner_error(&error);
            tracing::error!(session_id = %id, error = %error, "failed to clone goal repo packages");
            fail_session(inner, id, &fence, &reason, goal_info).await;
            return true;
        }
    };
    // Journaling content fingerprint (#25 redo keys on package content): derived
    // from the FIRST resolved package dir (the run's primary package), with the
    // goal's package_name anchoring run_key uniqueness. Repo-scoped, so editing
    // a package in the repo yields a fresh run_key on the next run.
    let fingerprint_files: Vec<crate::engine::materialize::PackageFile> = cloned
        .package_roots
        .first()
        .map(|root| crate::engine::clone::read_package_files(root))
        .unwrap_or_default();
    // The engine argv inputs (consumed at step C). The TempDir guards stay in
    // `_clone_guard` for the session lifetime — dropping it removes the working
    // tree and the transient clone credential on every exit path.
    let cloned_project_root = cloned.project_root.clone();
    let cloned_package_roots = cloned.package_roots.clone();
    let goal_ctx_early = Some(goal_ctx);
    let _clone_guard = cloned;

    // (B2) Journaling bootstrap (issue #25): resolve run_key, stamp it on
    // the document, and load the GitHub skip-set BEFORE any package work.
    // Journaling is never load-bearing — every failure below is logged and
    // swallowed; session disposition is decided exclusively by the CAS
    // choke-points.
    let mut journaler = start_journaler(
        inner,
        id,
        package_name,
        &fingerprint_files,
        // Repo-scoped packages carry their composed deps as an in-repo
        // `composed.deps` file (part of the fingerprinted content above), not a
        // separate store field, so there is no extra dep list to fold in here.
        &[],
        fencing_token,
    )
    .await;
    journal_lifecycle(&mut journaler, Transition::Validating).await;

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
    // never proceed without the credential its consumers expect.
    //
    // The token source (#138): at trigger it is the forwarded user token; on a
    // failover rebuild it is the controller-held token re-supplied by
    // `ensure_driver` from the in-memory `session_tokens` map (a same-process
    // worker loss). `raw_token` is `None` only when the controller no longer
    // holds it — a controller RESTART, or a takeover by a different pod that
    // never held it — and there a session that previously minted a key ESCALATES
    // to Failed rather than running with broken auth (the documented "survive
    // worker loss, NOT controller loss" boundary).
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
                    // ESCALATE: the controller no longer holds this session's
                    // token (a controller restart, or a takeover by a pod that
                    // never held it), so the NyxID session identity cannot be
                    // re-established (#138). Never run with broken auth.
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

    // (B5) Per-session codex CODEX_HOME (#112): render the codex config.toml
    // selecting the LLM provider (default chrono-llm; RAW/STRUCTURED vault
    // overrides) into a fresh 0700 dir holding a 0600 config.toml. Only runs
    // when BOTH codex config and the vault are wired (production); skipped for
    // legacy tests / minimal runs, which keeps CODEX_HOME unset and the
    // behaviour byte-identical. A render/IO failure FAILS the start — a run must
    // never proceed without a working LLM backend config. The TempDir GUARD is
    // held in this `drive_inner` frame (`_codex_home_guard`) so the dir lives
    // for the whole session and is removed on every exit path (mirroring how the
    // runner holds its runtime/package guards).
    let codex_home: Option<std::path::PathBuf>;
    let _codex_home_guard: Option<tempfile::TempDir>;
    match prepare_codex_home(inner, &session_doc).await {
        Ok(Some((guard, dir))) => {
            tracing::info!(session_id = %id, "rendered per-session codex config");
            codex_home = Some(dir);
            _codex_home_guard = Some(guard);
        }
        Ok(None) => {
            codex_home = None;
            _codex_home_guard = None;
        }
        Err(error) => {
            // The error message is client-safe (no provider key — this module
            // never handles the key value); detail is logged, not persisted raw.
            tracing::error!(session_id = %id, error = %error, "failed to render codex config");
            let reason = "failed to render codex provider config";
            journal_finish(
                &mut journaler,
                Transition::Failed {
                    exit_code: None,
                    error: reason.to_string(),
                },
            )
            .await;
            fail_session(inner, id, &fence, reason, goal_info).await;
            return true;
        }
    }

    // (B6) Per-session Ornn skill injection (#114): when the session pinned
    // Ornn skills/skillsets, fetch them as the session user (the #111 NyxID
    // token already merged into `env_profile`) through the `ornn-api` proxy and
    // install them into the per-session CODEX_HOME (#112) BEFORE the engine
    // spawns, so codex discovers them. Only runs when Ornn is wired AND a
    // CODEX_HOME exists AND the session has the NyxID token AND pins are present
    // — any of those absent means there is nothing to do / nowhere to fetch as,
    // so it is skipped and behaviour is byte-identical to pre-#114. Any Ornn
    // 404/403 (missing or forbidden pin) or download/unzip failure FAILS the
    // start LOUDLY — a pinned capability must never be silently dropped. The pin
    // set is read from the persisted SessionDoc, so a failover rebuild
    // re-resolves and re-injects the identical set.
    if let (Some(ornn_client), Some(codex_dir), Some(pins)) = (
        inner.ornn.get(),
        codex_home.as_ref(),
        session_doc.ornn_skills.as_ref(),
    ) {
        if !pins.is_empty() {
            // The session NyxID token is the user identity the fetch acts as
            // (#111). It is a SecretString in the env_profile; clone it for the
            // proxy call and NEVER log it. Absent => the run has no NyxID
            // identity, so a pinned (private/visibility-gated) fetch cannot be
            // performed as the user — fail loudly rather than fetch anonymously.
            match env_profile.get(nyxid_token::NYXID_ACCESS_TOKEN_KEY) {
                Some(user_token) => {
                    if let Err(error) =
                        crate::ornn::inject_pins(ornn_client, user_token, codex_dir, pins).await
                    {
                        // `inject_pins` already logged the precise cause (and
                        // never the token/presigned URL); the served `error`
                        // stays client-safe.
                        tracing::error!(session_id = %id, error = %error, "failed to inject ornn skills");
                        let reason = "failed to inject pinned Ornn skills";
                        journal_finish(
                            &mut journaler,
                            Transition::Failed {
                                exit_code: None,
                                error: reason.to_string(),
                            },
                        )
                        .await;
                        fail_session(inner, id, &fence, reason, goal_info).await;
                        return true;
                    }
                    tracing::info!(session_id = %id, pin_count = pins.len(), "injected pinned ornn skills");
                }
                None => {
                    tracing::error!(
                        session_id = %id,
                        "cannot inject pinned ornn skills without a session NyxID token"
                    );
                    let reason = "cannot inject pinned Ornn skills without a NyxID session token";
                    journal_finish(
                        &mut journaler,
                        Transition::Failed {
                            exit_code: None,
                            error: reason.to_string(),
                        },
                    )
                    .await;
                    fail_session(inner, id, &fence, reason, goal_info).await;
                    return true;
                }
            }
        }
    }

    // (C0) The GoalContext (the GitHub App token + goal.json inputs) was built at
    // step (B), moved there so the repo clone could use the freshly-minted token
    // (#115). The runner points the engine at the cloned repo:
    // `--project-root <repo>` + `--package-root <repo>/.fkst/packages/<name>`.
    let project_root = Some(cloned_project_root);
    let package_roots = cloned_package_roots;

    // (C) Start the engine. This await runs to completion (never
    // select-cancelled); a stop that raced in is honored right after.
    // `goal_ctx` is `Some(..)`, so `start_with_spec` writes the token file +
    // goal.json and `spawn_supervise` sets the GitHub env vars before the engine
    // runs.
    let mut session = match inner
        .runner
        .start_with_spec(&StartSpec {
            // Repo-scoped: the runner uses `project_root` + `package_roots`
            // (below), not materialized packages, so this stays empty.
            packages: Vec::new(),
            goal: goal_ctx_early,
            env_profile,
            codex_home,
            project_root,
            package_roots,
            // Real dispatch (#136): write the owner breadcrumb so the OS-truth
            // reconcile sweep sees this engine as live (and a restarted worker
            // can re-adopt it). The session id makes the breadcrumb mandatory.
            session_id: id.to_string(),
            worker_id: inner.pod_id.clone().unwrap_or_default(),
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

    // Update goal drive token_path + runtime_dir now that the runtime dir is
    // known: the JIT mint-request poller services requests under it (#107).
    if let Some(ref mut gi) = goal_info {
        gi.runtime_dir = session.runtime_dir.clone();
        gi.token_path = session.runtime_dir.join(crate::engine::TOKEN_FILE_NAME);
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
    // Fast JIT mint-request servicer (#107): polls for the credential helper's
    // near-expiry re-mint request so it is serviced within a git operation's
    // patience window (the 500ms supervise tick alone would be too slow).
    let mut mint_tick = tokio::time::interval(MINT_REQUEST_POLL);
    mint_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    // Self-fencing clock: the lease was live when this driver claimed the
    // session, so the success window starts now. Renew ERRORS (Mongo
    // unreachable) leave us blind: past the TTL the lease may have lapsed
    // and been taken over, so the engine must die as if the lease were
    // observed Lost — never run past what we can prove we hold.
    let mut last_renew_success = tokio::time::Instant::now();
    loop {
        tokio::select! {
            // JIT mint-request servicer (#107): the credential helper drops a
            // nonce-bearing request file near token expiry; service it (mint +
            // atomic rewrite + delete the request) so git gets a fresh token
            // without waiting for the periodic interval. A FatalExpired outcome
            // (e.g. the App was uninstalled) fails the session loudly.
            _ = mint_tick.tick() => {
                let jit_outcome = match goal_info {
                    Some(ref mut gi) => Some(service_mint_request(inner, gi).await),
                    None => None,
                };
                if let Some(MintOutcome::FatalExpired { reason }) = jit_outcome {
                    fail_running_for_token(
                        inner, id, &fence, &mut session, &mut journaler, goal_info, &reason,
                    )
                    .await;
                    return true;
                }
            }
            // A closed channel (service dropped) also reads as a stop: this
            // pod is going away and the engine must not be orphaned silently.
            line = next_stdout_line(&mut stdout_rx) => {
                match line {
                    Some(raw) => {
                        // Reactive re-mint (#109): a GitHub auth failure on the
                        // engine's stdout re-mints the goal token immediately
                        // (cooldown-gated), instead of waiting for the 55-min
                        // interval loop to catch the expiry. Checked BEFORE
                        // journaling so a real 401 cannot be swallowed by it.
                        let reactive_outcome = match goal_info {
                            Some(ref mut gi) if is_github_auth_failure(&raw) => {
                                tracing::warn!(
                                    session_id = %id,
                                    goal_id = %gi.goal_id,
                                    "github auth failure detected on engine output; re-minting goal token"
                                );
                                Some(reactive_refresh_goal_token(inner, gi).await)
                            }
                            _ => None,
                        };
                        if let Some(MintOutcome::FatalExpired { reason }) = reactive_outcome {
                            fail_running_for_token(
                                inner, id, &fence, &mut session, &mut journaler, goal_info,
                                &reason,
                            )
                            .await;
                            return true;
                        }
                        journal_stdout_line(&mut journaler, &raw).await;
                    }
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
                // and the (escalating) cooldown since the last attempt has
                // passed. A FatalExpired outcome (persistent mint failure with
                // an already-expired token, or InstallationGone) fails the
                // session loudly instead of letting substrate hit a silent 401.
                let refresh_outcome = match goal_info {
                    Some(ref mut gi) => Some(refresh_goal_token(inner, gi).await),
                    None => None,
                };
                if let Some(MintOutcome::FatalExpired { reason }) = refresh_outcome {
                    fail_running_for_token(
                        inner, id, &fence, &mut session, &mut journaler, goal_info, &reason,
                    )
                    .await;
                    return true;
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

/// Prepare a per-session CODEX_HOME for the engine's codex provider (#112).
///
/// Returns `Ok(None)` when codex config OR the vault is not wired (legacy tests
/// / minimal runs) — the caller then leaves CODEX_HOME unset and the behaviour
/// is byte-identical to pre-#112. Otherwise it:
/// 1. resolves the provider LAYER from the vault (default chrono-llm, with
///    RAW/STRUCTURED overrides), surfacing the missing-chrono-llm 422;
/// 2. renders the codex `config.toml` for that layer (operator-pinned chrono-llm
///    DEFAULT model/base_url from config);
/// 3. creates a fresh `fkst-codex-*` dir (0700) under the engine temp root and
///    writes `config.toml` (0600).
///
/// Returns the held [`tempfile::TempDir`] guard (the caller keeps it for the
/// session's lifetime) plus the canonicalized dir path to set as CODEX_HOME.
/// The rendered toml never contains a provider key (the key rides `env_key`),
/// and no vault value is logged.
async fn prepare_codex_home(
    inner: &Arc<Inner>,
    session: &SessionDoc,
) -> Result<Option<(tempfile::TempDir, PathBuf)>, AppError> {
    let (Some(codex), Some(vault)) = (inner.codex.get(), inner.vault.get()) else {
        // Codex config or the vault is not wired: skip CODEX_HOME entirely.
        return Ok(None);
    };

    let scope = scope_for_session(session);
    let owner_user_id = session.owner_user_id.as_deref().unwrap_or_default();
    // v1 connection precondition: assume connected on the online path (the live
    // chrono-llm connection is verified by the documented manual/staging
    // preflight, not on every session start). The seam keeps the 422 mapping
    // exercised by unit tests and lets a future issue swap in a live preflight.
    let check: &dyn ChronoLlmCheck = &AssumeConnected;
    let choice = codex_provider::resolve_provider_choice(
        vault,
        owner_user_id,
        session.org_id.as_deref(),
        &scope,
        check,
    )
    .await?;

    let config_toml = codex_provider::render_codex_config(
        &choice,
        &codex.codex_model,
        &codex.chrono_llm_base_url,
    )?;

    // 0700 dir under the engine temp root (same filesystem as the runtime dirs).
    let guard = tempfile::Builder::new()
        .prefix("fkst-codex-")
        .tempdir_in(inner.runner.temp_root())
        .map_err(|error| {
            tracing::error!(error = %error, "failed to create codex home dir");
            AppError::Unavailable("failed to create codex home".to_string())
        })?;
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(guard.path(), std::fs::Permissions::from_mode(0o700)).map_err(
            |error| {
                tracing::error!(error = %error, "failed to chmod codex home dir");
                AppError::Unavailable("failed to secure codex home".to_string())
            },
        )?;
    }

    // 0600 config.toml inside it.
    let config_path = guard.path().join("config.toml");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(&config_path, config_toml.as_bytes()).map_err(|error| {
            tracing::error!(error = %error, "failed to write codex config.toml");
            AppError::Unavailable("failed to write codex config".to_string())
        })?;
        std::fs::set_permissions(&config_path, std::fs::Permissions::from_mode(0o600)).map_err(
            |error| {
                tracing::error!(error = %error, "failed to chmod codex config.toml");
                AppError::Unavailable("failed to secure codex config".to_string())
            },
        )?;
    }

    let dir = guard.path().canonicalize().map_err(|error| {
        tracing::error!(error = %error, "failed to canonicalize codex home dir");
        AppError::Unavailable("failed to resolve codex home".to_string())
    })?;
    Ok(Some((guard, dir)))
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

/// Converge a RUNNING goal session to Failed when its GitHub credential can no
/// longer be served (#107): a persistent mint failure with the token already
/// expired, or the App installation removed. This is the hardened backstop's
/// loud-failure path — rather than letting substrate keep hitting a silent 401
/// with a dead token, the engine is stopped, the session CAS'd
/// `running|stopping -> failed` with a clear reason, the failure journaled, and
/// the goal synced to Failed. The credential reason never contains the token.
async fn fail_running_for_token(
    inner: &Inner,
    id: bson::Uuid,
    fence: &Document,
    session: &mut RunningSession,
    journaler: &mut Option<ServiceJournaler>,
    goal_info: &Option<GoalDrive>,
    reason: &str,
) {
    tracing::error!(session_id = %id, reason, "goal token unrecoverable; failing the session");
    // Watermark before stop() removes the runtime dirs.
    journal_watermark(journaler, &session.runtime_dir).await;
    if let Err(error) = inner.runner.stop(session).await {
        tracing::error!(session_id = %id, error = %error, "engine stop failed during token-failure");
    }
    let _ = inner
        .repo
        .transition_guarded(
            id,
            &[SessionStatus::Running, SessionStatus::Stopping],
            fence.clone(),
            doc! {
                "status": status_bson(SessionStatus::Failed),
                "error": truncate_error(reason),
                "stopped_at": bson::DateTime::now(),
            },
        )
        .await;
    journal_finish(
        journaler,
        Transition::Failed {
            exit_code: None,
            error: reason.to_string(),
        },
    )
    .await;
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
// uses the shared atomic JSON token-file writer (engine::write_token_file, #107).
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
    // Guard (replacing the old `active_session_id == session_id` CAS pin): only
    // sync if this session is still the goal's active session, so a newer
    // trigger is never clobbered. The active link lives in controller memory.
    if goals.active_session(goal_id).await != Some(session_id) {
        tracing::debug!(
            goal_id = %goal_id,
            session_id = %session_id,
            "goal-status sync skipped (not the goal's active session)"
        );
        return;
    }
    match goals
        .transition_status(goal_id, from_statuses, target, false)
        .await
    {
        Ok(Some(_)) => tracing::info!(
            goal_id = %goal_id,
            session_id = %session_id,
            target = ?target,
            "goal-status sync applied"
        ),
        Ok(None) => tracing::debug!(
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
    let (token, expires_at) = match gs
        .github_app
        .token_with_expiry_for_repo(&repo_ref, None)
        .await
    {
        Ok(pair) => pair,
        Err(error) => {
            // The error may carry the install URL hint (NotInstalled); log it,
            // never the token. The trigger preflight already validated install,
            // so a failure here is transient/edge — fail the start.
            tracing::error!(goal_id = %gi.goal_id, repo = %repo_ref, error = %error, "failed to mint github token at startup");
            return Err("failed to mint github token for the goal repo".to_string());
        }
    };
    // The token is in use from t=0; reset the refresh clock and record its
    // expiry so the credential helper's JIT path and the escalating backstop
    // both know the true freshness (#107).
    gi.minted_at = std::time::Instant::now();
    gi.token_expires_at = expires_at;
    gi.consecutive_failures = 0;
    Ok(GoalContext {
        goal_id: gi.goal_id,
        title: goal.title,
        description: goal.description,
        repo: gi.repo.clone(),
        github_token: token,
        token_expires_at: expires_at,
    })
}

/// Outcome of a mint-and-rewrite attempt (#107). The driver uses
/// [`Self::FatalExpired`] to transition the session to a clear Failed state
/// instead of letting substrate keep hitting a silent 401 with a dead token.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MintOutcome {
    /// Token re-minted and the file atomically rewritten.
    Refreshed,
    /// Gated (cooldown not elapsed, no goal support, or runtime dir unknown);
    /// nothing attempted.
    Skipped,
    /// A mint/write attempt failed but the current token is still valid — keep
    /// supervising; the next (tightening) interval retries.
    TransientFailure,
    /// Persistent mint failure with the on-disk token already past expiry: the
    /// credential can no longer be served, so the session must fail loudly.
    FatalExpired { reason: String },
}

/// Time-based refresh of the GitHub installation token for a goal session.
/// Minted fresh ONLY when the refresh interval has elapsed (token nearing the
/// ~60-min TTL) AND the cooldown since the last attempt has passed. This is the
/// periodic-loop entry; reactive (auth-failure-driven) refreshes go through
/// [`reactive_refresh_goal_token`], which skips the interval gate.
async fn refresh_goal_token(inner: &Inner, drive: &mut GoalDrive) -> MintOutcome {
    if drive.minted_at.elapsed() < TOKEN_REFRESH_INTERVAL {
        return MintOutcome::Skipped;
    }
    mint_and_rewrite_token(inner, drive, "interval").await
}

/// Reactive re-mint (issue #109): a detected GitHub auth failure for the
/// session's work re-mints the token IMMEDIATELY — BYPASSING the 55-min
/// interval gate — rather than waiting for the periodic loop to catch the
/// expiry. The cooldown is still respected so a burst of auth-failure signals
/// cannot hammer the GitHub mint API.
async fn reactive_refresh_goal_token(inner: &Inner, drive: &mut GoalDrive) -> MintOutcome {
    mint_and_rewrite_token(inner, drive, "auth-failure").await
}

/// Service a pending just-in-time mint request from the credential helper
/// (#107). The helper drops `<token_path>.request` containing the per-session
/// nonce when the on-disk token is near expiry; this verifies the nonce, mints
/// a fresh token, atomically rewrites the token file, and ONLY THEN deletes the
/// request file (the helper waits on that deletion as the "fresh token ready"
/// signal). A non-matching/absent nonce or no request file is a no-op.
///
/// The mint here shares [`mint_and_rewrite_token`]'s cooldown + coalescing, so a
/// burst of git invocations cannot trigger redundant App-token calls.
async fn service_mint_request(inner: &Inner, drive: &mut GoalDrive) -> MintOutcome {
    if drive.token_path.as_os_str().is_empty() {
        return MintOutcome::Skipped;
    }
    let request_path = mint_request_path(&drive.token_path);
    let Ok(contents) = tokio::fs::read_to_string(&request_path).await else {
        return MintOutcome::Skipped; // no pending request
    };
    // Authenticate the request against the per-session nonce file (0600). Only
    // this session's own engine child knows the nonce (env + 0600 dir), so a
    // mismatch means a stray/forged file — drop it without minting.
    let nonce_path = drive.runtime_dir.join(crate::engine::NONCE_FILE_NAME);
    let expected = tokio::fs::read_to_string(&nonce_path).await.ok();
    let presented = contents.trim();
    if expected.as_deref().map(str::trim) != Some(presented) || presented.is_empty() {
        tracing::warn!(
            goal_id = %drive.goal_id,
            "mint request nonce mismatch; ignoring and clearing the request file"
        );
        let _ = tokio::fs::remove_file(&request_path).await;
        return MintOutcome::Skipped;
    }

    let outcome = mint_and_rewrite_token(inner, drive, "jit-helper").await;
    // Signal completion to the waiting helper by deleting the request file —
    // ALWAYS, even on a transient failure, so the helper stops waiting and falls
    // back to the current token rather than blocking the whole patience window.
    let _ = tokio::fs::remove_file(&request_path).await;
    outcome
}

/// Path of the credential-helper's JIT mint-request file for a token file.
fn mint_request_path(token_path: &std::path::Path) -> PathBuf {
    let mut p = token_path.as_os_str().to_owned();
    p.push(crate::engine::MINT_REQUEST_SUFFIX);
    PathBuf::from(p)
}

/// Effective cooldown for this attempt: tightened to [`TOKEN_REFRESH_COOLDOWN_URGENT`]
/// once the on-disk token is within [`TOKEN_EXPIRY_URGENT_MARGIN`] of expiry, so
/// a flaky mint near the deadline gets retried aggressively (#107).
fn effective_cooldown(token_expires_at: std::time::SystemTime) -> Duration {
    let remaining = token_expires_at
        .duration_since(std::time::SystemTime::now())
        .unwrap_or(Duration::ZERO);
    if remaining <= TOKEN_EXPIRY_URGENT_MARGIN {
        TOKEN_REFRESH_COOLDOWN_URGENT
    } else {
        TOKEN_REFRESH_COOLDOWN
    }
}

/// Shared mint-and-rewrite core for the interval, reactive, and JIT refresh
/// paths. Respects an escalating per-attempt cooldown, mints a fresh
/// installation token, and atomically rewrites the JSON token file
/// (`{token, expires_at}`, mode 0600, tmp + rename — #107). On success the
/// failure counter resets; on failure the engine keeps the previous token until
/// the token is past expiry AND failures persist, at which point a
/// [`MintOutcome::FatalExpired`] is returned for the caller to fail the session.
/// `reason` tags the trigger in logs. The token value is never logged.
async fn mint_and_rewrite_token(inner: &Inner, drive: &mut GoalDrive, reason: &str) -> MintOutcome {
    // Escalating cooldown gate (applies to ALL paths): never hammer the mint API
    // more than once per window, but tighten the window as expiry approaches.
    if drive.last_attempt.elapsed() < effective_cooldown(drive.token_expires_at) {
        return MintOutcome::Skipped;
    }
    drive.last_attempt = std::time::Instant::now();

    let Some(gs) = inner.goal_support.get() else {
        return MintOutcome::Skipped;
    };
    let github_app = &gs.github_app;

    if drive.token_path.as_os_str().is_empty() {
        // Runtime dir not yet established (engine not started).
        return MintOutcome::Skipped;
    }

    let repo_ref = format!("{}/{}", drive.repo.owner, drive.repo.name);
    match github_app.token_with_expiry_for_repo(&repo_ref, None).await {
        Ok((token, expires_at)) => {
            // Atomic JSON rewrite via the shared engine writer so the on-disk
            // format and atomicity match the startup write exactly (#107).
            match crate::engine::write_token_file(&drive.token_path, &token, expires_at) {
                Ok(()) => {
                    drive.minted_at = std::time::Instant::now();
                    drive.token_expires_at = expires_at;
                    drive.consecutive_failures = 0;
                    tracing::info!(
                        goal_id = %drive.goal_id,
                        reason,
                        "github token refreshed"
                    );
                    MintOutcome::Refreshed
                }
                Err(error) => {
                    tracing::warn!(
                        goal_id = %drive.goal_id,
                        path = %drive.token_path.display(),
                        error = %error,
                        "token file rewrite failed; retrying next tick"
                    );
                    classify_mint_failure(drive)
                }
            }
        }
        Err(error) => {
            // InstallationGone after a fresh re-resolve means the App was
            // uninstalled mid-session — the credential cannot be recovered, so
            // fail loudly rather than spin (#107).
            let installation_gone = matches!(
                error,
                crate::github_app::GithubAppError::InstallationGone { .. }
            );
            tracing::warn!(
                goal_id = %drive.goal_id,
                repo = %repo_ref,
                reason,
                error = %error,
                "token mint failed; engine keeps previous token"
            );
            if installation_gone {
                return MintOutcome::FatalExpired {
                    reason: "github app installation removed for the goal repo".to_string(),
                };
            }
            classify_mint_failure(drive)
        }
    }
}

/// Bump the consecutive-failure counter and decide whether the failure is fatal:
/// fatal only once the on-disk token is genuinely past expiry AND the failures
/// have persisted, so a transient blip while the token is still valid never
/// kills a healthy session (#107).
fn classify_mint_failure(drive: &mut GoalDrive) -> MintOutcome {
    drive.consecutive_failures = drive.consecutive_failures.saturating_add(1);
    let expired = drive.token_expires_at <= std::time::SystemTime::now();
    if expired && drive.consecutive_failures >= MAX_CONSECUTIVE_MINT_FAILURES {
        MintOutcome::FatalExpired {
            reason: format!(
                "github token mint failed {} times and the token has expired",
                drive.consecutive_failures
            ),
        }
    } else {
        MintOutcome::TransientFailure
    }
}

/// Detect a GitHub authentication/authorization failure in one engine stdout
/// line (issue #109). The engine surfaces its GitHub work on stdout; a 401/403
/// or an expired/bad-credentials message is the reactive-re-mint signal. Matched
/// case-insensitively against the well-known GitHub auth-failure phrases so a
/// real expiry triggers an immediate re-mint instead of waiting for the
/// time-based loop. Conservative by design: only unambiguous auth markers match,
/// so ordinary chatter never spuriously burns a mint attempt (the cooldown is
/// the additional backstop).
fn is_github_auth_failure(raw: &[u8]) -> bool {
    let line = String::from_utf8_lossy(raw).to_ascii_lowercase();
    line.contains("bad credentials")
        || line.contains("401 unauthorized")
        || line.contains("http 401")
        || line.contains("requires authentication")
        || (line.contains("github") && line.contains("token") && line.contains("expired"))
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
    fingerprint_files: &[crate::engine::materialize::PackageFile],
    fingerprint_deps: &[String],
    fencing_token: Option<i64>,
) -> Option<ServiceJournaler> {
    let setup = inner.journal.get()?;
    let ctx = SessionCtx {
        session_id: id.to_string(),
        package_name: package_name.to_string(),
        package_fingerprint: package_fingerprint(fingerprint_files, fingerprint_deps),
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
            ornn_skills: None,
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
        let service = SessionService::new(SessionRepo::new(&db), EngineConfig::default());
        let session = env_test_session();
        let profile = resolve_env_profile(&service.inner, &session)
            .await
            .expect("empty profile, no error");
        assert!(
            profile.is_empty(),
            "no vault wired => empty env profile (legacy behaviour)"
        );
    }

    // ---- reactive goal-token re-mint (issue #109) -----------------------------

    /// The auth-failure detector recognises the well-known GitHub 401/403
    /// markers (case-insensitively) and ignores ordinary engine chatter, so a
    /// real expiry triggers a re-mint while normal output never burns one.
    #[test]
    fn is_github_auth_failure_matches_only_real_auth_markers() {
        assert!(is_github_auth_failure(b"GitHub API error: Bad credentials"));
        assert!(is_github_auth_failure(b"response: 401 Unauthorized"));
        assert!(is_github_auth_failure(b"HTTP 401 from api.github.com"));
        assert!(is_github_auth_failure(b"Requires authentication"));
        assert!(is_github_auth_failure(
            b"the github installation token expired"
        ));
        // Ordinary chatter must NOT match (no spurious mint attempts).
        assert!(!is_github_auth_failure(b"RAISED: eyJkZXB0IjoiaGVsbG8ifQ=="));
        assert!(!is_github_auth_failure(b"consumer started dept=hello"));
        assert!(!is_github_auth_failure(b"token written successfully"));
    }

    /// A service with no goal-support wired: `mint_and_rewrite_token` still runs
    /// its cooldown gate + stamps `last_attempt` before the (absent) mint, so
    /// this exercises the gating without any GitHub/Mongo wiring.
    async fn no_goal_support_service() -> SessionService {
        let db = crate::db::Db {
            database: mongodb::Client::with_uri_str("mongodb://localhost:27017")
                .await
                .expect("client")
                .database("sessions_unit_test"),
        };
        SessionService::new(SessionRepo::new(&db), EngineConfig::default())
    }

    fn goal_drive_with(minted_ago: Duration, last_attempt_ago: Duration) -> GoalDrive {
        let now = std::time::Instant::now();
        GoalDrive {
            goal_id: bson::Uuid::new(),
            repo: RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
            owner_user_id: "user-1".to_string(),
            package_names: vec!["demo".to_string()],
            // Non-empty so the path is not short-circuited by the
            // "runtime dir not yet established" guard.
            token_path: PathBuf::from("/run/session/github-token"),
            runtime_dir: PathBuf::from("/run/session"),
            minted_at: now - minted_ago,
            last_attempt: now - last_attempt_ago,
            // Far in the future so the escalating-cooldown gate uses the normal
            // 60s window in the gating tests (not the urgent 10s one).
            token_expires_at: std::time::SystemTime::now() + Duration::from_secs(3600),
            consecutive_failures: 0,
        }
    }

    /// A simulated auth failure triggers an IMMEDIATE re-mint attempt even when
    /// the 55-min interval has NOT elapsed (the reactive path bypasses the
    /// interval gate). The observable is `last_attempt` advancing: the gate is
    /// passed and the attempt stamp is taken before the mint fires.
    #[tokio::test]
    async fn reactive_refresh_bypasses_the_interval_gate() {
        let service = no_goal_support_service().await;
        // minted just now (interval NOT elapsed); last attempt long ago
        // (cooldown clear).
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_secs(3600));
        let before = drive.last_attempt;

        // The periodic path would no-op (interval not elapsed)...
        refresh_goal_token(&service.inner, &mut drive).await;
        assert_eq!(
            drive.last_attempt, before,
            "the interval-gated path must NOT attempt while the interval is unmet"
        );

        // ...but the reactive path attempts immediately despite the fresh mint.
        reactive_refresh_goal_token(&service.inner, &mut drive).await;
        assert!(
            drive.last_attempt > before,
            "the reactive path must attempt a re-mint immediately, bypassing the interval gate"
        );
    }

    /// The reactive path still RESPECTS the 60s cooldown: a second auth-failure
    /// signal arriving inside the cooldown window does not take another attempt
    /// (no hammering the GitHub mint API on a burst of 401s).
    #[tokio::test]
    async fn reactive_refresh_respects_the_cooldown() {
        let service = no_goal_support_service().await;
        // A very recent attempt (cooldown NOT elapsed); interval irrelevant.
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_millis(10));
        let before = drive.last_attempt;

        reactive_refresh_goal_token(&service.inner, &mut drive).await;
        assert_eq!(
            drive.last_attempt, before,
            "a re-mint inside the cooldown window must be suppressed"
        );
    }

    /// #107: the refresh cooldown tightens to the urgent window once the token is
    /// within the urgent margin of expiry, and stays at the normal window while
    /// the token is comfortably fresh.
    #[test]
    fn effective_cooldown_tightens_near_expiry() {
        let fresh = std::time::SystemTime::now() + Duration::from_secs(3600);
        assert_eq!(effective_cooldown(fresh), TOKEN_REFRESH_COOLDOWN);
        let near = std::time::SystemTime::now() + Duration::from_secs(60);
        assert_eq!(effective_cooldown(near), TOKEN_REFRESH_COOLDOWN_URGENT);
        let expired = std::time::SystemTime::UNIX_EPOCH;
        assert_eq!(effective_cooldown(expired), TOKEN_REFRESH_COOLDOWN_URGENT);
    }

    /// #107: a mint failure is only fatal once the token is genuinely expired AND
    /// the failures have persisted; a blip while the token is still valid stays
    /// transient so a healthy session is never killed.
    #[test]
    fn classify_mint_failure_only_fatal_when_expired_and_persistent() {
        // Still valid: never fatal, however many failures.
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_secs(3600));
        for _ in 0..(MAX_CONSECUTIVE_MINT_FAILURES + 3) {
            assert_eq!(
                classify_mint_failure(&mut drive),
                MintOutcome::TransientFailure
            );
        }

        // Expired token: fatal only once the failure count crosses the threshold.
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_secs(3600));
        drive.token_expires_at = std::time::SystemTime::UNIX_EPOCH;
        for _ in 0..(MAX_CONSECUTIVE_MINT_FAILURES - 1) {
            assert_eq!(
                classify_mint_failure(&mut drive),
                MintOutcome::TransientFailure
            );
        }
        assert!(matches!(
            classify_mint_failure(&mut drive),
            MintOutcome::FatalExpired { .. }
        ));
    }

    /// #107: the JIT mint-request path is `<token_path>` + the request suffix.
    #[test]
    fn mint_request_path_appends_the_request_suffix() {
        let token = PathBuf::from("/run/session/github-token");
        let req = mint_request_path(&token);
        assert_eq!(req, PathBuf::from("/run/session/github-token.request"));
    }

    /// #107: a JIT request whose nonce does not match the session nonce file is
    /// ignored (no mint) and the stray request file is cleared.
    #[tokio::test]
    async fn service_mint_request_rejects_a_bad_nonce_and_clears_the_file() {
        let service = no_goal_support_service().await;
        let dir = tempfile::tempdir().expect("dir");
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_secs(3600));
        drive.runtime_dir = dir.path().to_path_buf();
        drive.token_path = dir.path().join(crate::engine::TOKEN_FILE_NAME);

        // Real session nonce, and a request file presenting a DIFFERENT one.
        crate::engine::goal_token::write_nonce_file(dir.path(), "realnonce").expect("nonce");
        let request = mint_request_path(&drive.token_path);
        tokio::fs::write(&request, b"forgednonce\n")
            .await
            .expect("write request");

        let outcome = service_mint_request(&service.inner, &mut drive).await;
        assert_eq!(outcome, MintOutcome::Skipped, "bad nonce must not mint");
        assert!(!request.exists(), "stray request file must be cleared");
    }

    /// #107: no pending request file is a no-op (the common idle case).
    #[tokio::test]
    async fn service_mint_request_no_request_is_a_noop() {
        let service = no_goal_support_service().await;
        let dir = tempfile::tempdir().expect("dir");
        let mut drive = goal_drive_with(Duration::ZERO, Duration::from_secs(3600));
        drive.runtime_dir = dir.path().to_path_buf();
        drive.token_path = dir.path().join(crate::engine::TOKEN_FILE_NAME);
        assert_eq!(
            service_mint_request(&service.inner, &mut drive).await,
            MintOutcome::Skipped
        );
    }

    // ---- failover: controller-held session token (#138) -----------------------
    //
    // These exercise the deterministic seam that drives the failover NyxID
    // re-mint decision: `ensure_driver` re-supplies `held_token(id)` to the
    // driver, and B4 re-mints when that is `Some` or ESCALATES when it is `None`.
    // (The full B4 mint/escalate fork itself is pre-existing #111 logic, covered
    // by `nyxid_token`'s provision tests; a full-drive test cannot reach B4 in a
    // fake-repo harness because `clone_repo_packages` runs — and fails — first.)

    #[tokio::test]
    async fn failover_reuses_controller_held_token_to_remint() {
        // A token held at trigger is re-supplied to the failover driver, so the
        // per-session NyxID key CAN be re-minted (B4's `Some(token)` arm).
        let service = no_goal_support_service().await;
        let id = bson::Uuid::new();
        let token = SecretString::from("user-access-token".to_string());
        service.hold_session_token(id, token.clone());
        let held = service.held_token(id).expect("token must be held");
        assert_eq!(held.expose_secret(), "user-access-token");
    }

    #[tokio::test]
    async fn failover_without_token_after_controller_restart_escalates() {
        // A fresh service models a controller restart: it never held this
        // session's token, so the failover driver gets `None` and B4 ESCALATES a
        // session that previously minted a key rather than running broken auth.
        let service = no_goal_support_service().await;
        let id = bson::Uuid::new();
        assert!(
            service.held_token(id).is_none(),
            "a restarted controller holds no token; the failover must escalate"
        );
    }

    #[tokio::test]
    async fn holding_then_forgetting_a_token_round_trips() {
        // The token is dropped on terminal exit; after that a failover sees None.
        let service = no_goal_support_service().await;
        let id = bson::Uuid::new();
        service.hold_session_token(id, SecretString::from("tok".to_string()));
        assert!(service.held_token(id).is_some());
        // The teardown path removes the entry from the same map.
        service
            .inner
            .session_tokens
            .lock()
            .expect("token store")
            .remove(&id);
        assert!(service.held_token(id).is_none());
    }
}
