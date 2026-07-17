//! The OpenSandbox [`SessionBackend`] implementation (issue #418): drive one sandbox
//! per substrate session over the #416 lifecycle client + #417 execd client.
//!
//! This is the plug-and-play sibling of [`crate::session_backend::k8s::K8sBackend`]:
//! the reconciler + its loops hold an `Arc<dyn SessionBackend>` and never see a
//! sandbox type, so swapping Kubernetes for OpenSandbox touches nothing in the planner
//! or the loops. It is NOT wired at runtime yet — `main.rs` stays on `K8sBackend`
//! until #420 — so everything here is fully unit/wiremock-tested but unreachable in
//! production.
//!
//! ## Grounded correction to the issue: metadata-only correlation
//! The upstream Sandbox RESPONSE carries `metadata` but NOT `extensions` (confirmed
//! against `sandbox-lifecycle.yml` at tag `server/v0.2.1`); `extensions` is a
//! create-REQUEST-only field, so it cannot round-trip. ALL correlation therefore lives
//! in `metadata`, and the create request's `extensions` is always empty. Because
//! `metadata` values must be K8s label values (`≤63` chars, alphanumeric-bounded), the
//! 64-hex config hash is split across two keys and the arbitrary-UTF-8 work label is
//! hex-encoded — see [`correlate`] for the full encoding + its length caveat.
//!
//! ## Single-writer invariant (why the list-guard + reaper are safe)
//! The control plane is single-writer per repo: the reconcile loop
//! ([`crate::reconcile::loops::run_reconcile_loop`]) is the SOLE queue consumer and
//! reconciles each repo serially — "the single consumer guarantees per-repo
//! serialization (never two concurrent reconciles of the same repo)". The Deployment
//! runs `replicas: 1` with `strategy.type: Recreate` (see the control-plane
//! Deployment in `opensandbox-developer-guide.md` §14.6), so
//! a rollout never overlaps two writers. The live loop's `ensure_session` list-guard is
//! thus race-free in steady state; the `list_fleet` duplicate-reaper is the
//! cross-restart / split-brain BACKSTOP that converges the fleet back to one sandbox
//! per session should that invariant ever be briefly violated.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;
use secrecy::SecretString;

use crate::config::PodConfig;
use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, LivePod};
use crate::session_backend::{
    BackendError, DeliveryOutcome, EnsureOutcome, ObserveError, RuntimeStatus, SessionBackend,
    SessionHandle, ValidationOutcome, ValidationRequest,
};

use super::dto::{ImageSpec, OsbError, ResourceLimits, SandboxView};
use super::{ExecdClient, OsbLifecycleClient};

pub mod correlate;
mod engine_observe;
mod fleet;
mod health;
mod logs;
mod observe;
mod rotation;
mod spawn;
mod validation;

/// Default env var execd reads its per-request access token from (`ServerAccessToken`
/// in the execd server; gates every `X-EXECD-ACCESS-TOKEN` header). Grounded from the
/// execd flag parser at tag `server/v0.2.1`.
pub const DEFAULT_EXECD_TOKEN_ENV_KEY: &str = "EXECD_ACCESS_TOKEN";

/// Builds a per-sandbox [`ExecdClient`] from `(sandbox_id, session_id)`. Injected so
/// tests point execd at a wiremock base while the #420 production factory (built in
/// `main.rs`) derives the token via [`super::derive_execd_token`] and targets the real
/// lifecycle proxy. `pub` so `main.rs` (a separate binary crate) can name the alias
/// for the factory it passes to [`OsbBackend::new`] instead of spelling the boxed-`Fn`
/// type inline (which trips clippy's `type_complexity`).
pub type ExecdFactory = Arc<dyn Fn(&str, &str) -> ExecdClient + Send + Sync>;

/// One shield entry: when the delete was observed, plus the repo + trigger issue the
/// synthetic `Terminating` pod is rebuilt with. Keyed by session id.
type ShieldEntry = (Instant, RepoRef, i64);

/// Static (non-per-session) inputs the OpenSandbox backend launches a sandbox with.
/// Env-value wiring is #420; tests construct this ad hoc.
pub struct OsbConfig {
    /// The image every session sandbox runs (the control-plane image, `run-substrate`).
    pub image: ImageSpec,
    /// The EXPLICIT full sandbox entrypoint command (OpenSandbox has no image-default
    /// fallback the way a pod does, so it is always spelled out).
    pub entrypoint: Vec<String>,
    /// Free-form sandbox resource limits (cpu / memory / gpu / …).
    pub resource_limits: ResourceLimits,
    /// The long-lived seed the per-session execd token is derived from.
    pub execd_seed: SecretString,
    /// The env var name the derived execd token is injected under (default
    /// [`DEFAULT_EXECD_TOKEN_ENV_KEY`]).
    pub execd_token_env_key: String,
    /// The respawn-shield window: how long a just-stopped session is reported as a
    /// synthetic `Terminating` pod so the planner does not thrash (see [`observe`]).
    pub reconcile_window: Duration,
    /// The validation holder's server-side GC deadline base + the reaper's sweep-age
    /// budget, in seconds. Mirrors `K8sBackend`'s validate knobs; #420 wires the real
    /// value, tests set it ad hoc (see [`validation`]).
    pub validate_deadline_secs: i64,
    /// The poll cadence while waiting for a validation holder's command to finish.
    pub validate_poll_interval_secs: u64,
}

/// The OpenSandbox session backend: one sandbox per substrate session, correlated
/// entirely through sandbox `metadata`. Held by the reconciler + loops as
/// `Arc<dyn SessionBackend>`.
pub struct OsbBackend {
    /// The lifecycle client, behind an `Arc` so the validation drop-guard can hold a
    /// `'static` handle to spawn a best-effort holder delete on Drop (the client cannot
    /// derive `Clone` — its `SecretString` API key is non-`Clone`).
    lifecycle: Arc<OsbLifecycleClient>,
    execd_factory: ExecdFactory,
    pod_config: PodConfig,
    config: OsbConfig,
    /// Respawn shield: sessions deleted THIS process that must be reported
    /// `Terminating` (not `Absent`) until the window lapses — see [`observe`] for why
    /// the OpenSandbox read-your-writes delete needs this pacing that K8s gets free.
    shield: Mutex<HashMap<String, ShieldEntry>>,
    /// Latest full credential bundle per live session, for the full re-push heal path
    /// (see [`rotation`]): when a container restart wipes the creds dir, the rotation
    /// verb re-pushes THIS whole bundle rather than only the rotated file. A
    /// control-plane restart empties this map — the next reconcile tick repopulates it
    /// via `ensure_session`. `SecretString` is non-`Clone`, so bundles are MOVED in and
    /// BORROWED out for upload, never cloned.
    creds: Mutex<HashMap<String, BTreeMap<String, SecretString>>>,
}

impl OsbBackend {
    /// Build the backend over an injected lifecycle client + execd factory, the shared
    /// pod-env config, and the static launch config. The shield + creds cache start
    /// empty; the lifecycle client is wrapped in an `Arc` once here.
    pub fn new(
        lifecycle: OsbLifecycleClient,
        execd_factory: ExecdFactory,
        pod_config: PodConfig,
        config: OsbConfig,
    ) -> Self {
        Self {
            lifecycle: Arc::new(lifecycle),
            execd_factory,
            pod_config,
            config,
            shield: Mutex::new(HashMap::new()),
            creds: Mutex::new(HashMap::new()),
        }
    }

    /// Resolve the ONE managed sandbox for `session_id`: filter on
    /// `fkst-managed` + `fkst-session-id`, then 0 → [`BackendError::NotFound`], 1 → it,
    /// and (the single-writer violation backstop) >1 → the OLDEST by `(created_at, id)`
    /// with a warning. `stop` / `remove` / `mark_pending` all route through here.
    async fn resolve_one(&self, session_id: &str) -> Result<SandboxView, BackendError> {
        let mut views = self
            .lifecycle
            .list_sandboxes(&managed_session_filter(session_id))
            .await?;
        match views.len() {
            0 => Err(BackendError::NotFound),
            1 => Ok(views.pop().expect("len==1")),
            n => {
                tracing::warn!(
                    session_id = %session_id,
                    count = n,
                    "opensandbox resolve_one: multiple sandboxes for one session; using oldest"
                );
                let idx = pick_oldest_index(&views);
                Ok(views.swap_remove(idx))
            }
        }
    }

    /// Record that `session_id` was just deleted, so [`OsbBackend::observe_repo`]
    /// reports it as a synthetic `Terminating` pod until the window lapses.
    fn record_shield(&self, session_id: &str, repo: RepoRef, trigger: i64) {
        let mut guard = self.shield.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(session_id.to_string(), (Instant::now(), repo, trigger));
    }

    /// Prune expired shield entries, then return the still-shielded `(session_id,
    /// trigger)` pairs belonging to `repo`. The lock is dropped before the caller does
    /// any async work (no await under the lock).
    fn drain_shield_for_repo(&self, repo: &RepoRef) -> Vec<(String, i64)> {
        let window = self.config.reconcile_window;
        let now = Instant::now();
        let mut guard = self.shield.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|_, (recorded, _, _)| now.duration_since(*recorded) < window);
        guard
            .iter()
            .filter(|(_, (_, entry_repo, _))| entry_repo == repo)
            .map(|(session_id, (_, _, trigger))| (session_id.clone(), *trigger))
            .collect()
    }
}

/// The metadata filter pinning the ONE managed sandbox for a session.
pub(super) fn managed_session_filter(session_id: &str) -> Vec<(String, String)> {
    vec![
        (correlate::KEY_MANAGED.to_string(), "true".to_string()),
        (
            correlate::KEY_SESSION_ID.to_string(),
            session_id.to_string(),
        ),
    ]
}

/// Index of the OLDEST view by `(created_at, id)` (a `None` created_at sorts first).
/// The stable `id` tie-break makes the choice deterministic for same-timestamp
/// duplicates. `views` is always non-empty at every call site.
pub(super) fn pick_oldest_index(views: &[SandboxView]) -> usize {
    (0..views.len())
        .min_by(|&a, &b| sort_key(&views[a]).cmp(&sort_key(&views[b])))
        .unwrap_or(0)
}

/// The `(created_at, id)` ordering key: RFC3339 timestamps sort lexicographically in
/// chronological order, and `id` breaks ties.
fn sort_key(view: &SandboxView) -> (&str, &str) {
    (view.created_at.as_deref().unwrap_or(""), view.id.as_str())
}

/// Map every lifecycle/execd failure into the backend taxonomy: a missing sandbox is
/// the [`BackendError::NotFound`] the executor swallows; every other failure is carried
/// opaquely. A blanket `?` conversion so no verb can forget to map an [`OsbError`].
impl From<OsbError> for BackendError {
    fn from(error: OsbError) -> Self {
        match error {
            OsbError::NotFound => BackendError::NotFound,
            other => BackendError::Other(anyhow::Error::new(other)),
        }
    }
}

#[async_trait]
impl SessionBackend for OsbBackend {
    async fn check_reachable(&self) -> Result<String, BackendError> {
        self.check_reachable_impl().await
    }

    async fn ensure_session(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        self.ensure_session_impl(spec, creds).await
    }

    async fn observe_repo(&self, repo: &RepoRef) -> Result<Vec<LivePod>, BackendError> {
        self.observe_repo_impl(repo).await
    }

    async fn mark_pending(&self, session_id: &str) -> Result<(), BackendError> {
        self.mark_pending_impl(session_id).await
    }

    async fn stop_session(&self, session_id: &str, reason: KillReason) -> Result<(), BackendError> {
        self.stop_session_impl(session_id, reason).await
    }

    async fn remove_terminal(&self, session_id: &str) -> Result<(), BackendError> {
        self.remove_terminal_impl(session_id).await
    }

    async fn list_fleet(&self) -> Result<Vec<SessionHandle>, BackendError> {
        self.list_fleet_impl().await
    }

    // --- The five fleet verbs #419 completes (credential heal, health reads, env
    // validation). Each delegates to its `*_impl` in a sibling submodule; the taxonomy
    // + verdict contracts are shared with the Kubernetes backend, never re-derived.

    async fn deliver_credential(
        &self,
        session_id: &str,
        file: &str,
        contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        self.deliver_credential_impl(session_id, file, contents)
            .await
    }

    async fn status_summary(&self, session_id: &str) -> Result<RuntimeStatus, BackendError> {
        self.status_summary_impl(session_id).await
    }

    async fn recent_output(&self, session_id: &str) -> Option<String> {
        self.recent_output_impl(session_id).await
    }

    async fn engine_observe(&self, session_id: &str, limit: u32) -> Result<String, ObserveError> {
        self.engine_observe_impl(session_id, limit).await
    }

    async fn run_validation(
        &self,
        req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        self.run_validation_impl(req).await
    }

    async fn reap_stale_validations(&self) -> Result<usize, BackendError> {
        self.reap_stale_validations_impl().await
    }
}

#[cfg(test)]
#[path = "backend_test_support.rs"]
mod backend_test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
