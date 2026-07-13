//! Backend-neutral session-runtime abstraction (issues #412 + #413).
//!
//! The reconciler and its background loops drive session runtimes through the
//! [`SessionBackend`] trait and never touch a concrete Kubernetes type: the executor
//! holds only an `Arc<dyn SessionBackend>` (on [`crate::reconcile::execute::ReconcileCtx`]),
//! and the token-rotation / health-scrape / sweep loops take one too. The
//! direct-Kubernetes implementation lives in [`k8s::K8sBackend`]; keeping it behind
//! this contract makes the pod-driving machinery a plug-and-play unit that a future
//! runtime (e.g. an OpenSandbox backend) can replace without touching the planner,
//! the per-repo driver, or the loops. This module is deliberately KUBE-FREE — no
//! `kube` / `k8s_openapi` type appears in the trait surface.

use std::collections::BTreeMap;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, LivePod};

pub mod k8s;
pub mod opensandbox;
/// Shared, kube-free env-validation verdict parsing (issue #419). Both backends parse
/// the SAME verdict frame through here so a validation verdict is byte-for-byte
/// identical regardless of which runtime executed it.
pub(crate) mod verdict;

#[cfg(test)]
pub(crate) mod test_support;

/// What ensuring a session did: a freshly created runtime, or an idempotent no-op
/// because the deterministically-named runtime already existed (already live).
#[derive(Debug, PartialEq, Eq)]
pub enum EnsureOutcome {
    Created,
    AlreadyLive,
}

/// A session-backend failure. [`NotFound`](BackendError::NotFound) is the
/// 404-equivalent the executor SWALLOWS (a runtime deleted between the plan and the
/// effect is a benign no-op); every other failure is carried opaquely in
/// [`Other`](BackendError::Other) and logged with context at the executor boundary.
#[derive(Debug, thiserror::Error)]
pub enum BackendError {
    #[error("session backend resource not found")]
    NotFound,
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

/// A failure of the [`SessionBackend::engine_observe`] verb, kept apart from
/// [`BackendError`] because its cases map to DISTINCT client responses (404 /
/// 409 / 503) rather than the executor's swallow-or-log split.
#[derive(Debug, thiserror::Error)]
pub enum ObserveError {
    /// No runtime exists for the session (pod/sandbox absent) → 404.
    #[error("session runtime not found")]
    SessionNotFound,
    /// The session's packages declare no reliable subscriptions, so the engine
    /// has neither an observe socket nor a `delivery.redb` to open → 409.
    #[error("session has no durable delivery store to observe")]
    NoDurableStore,
    /// Any other exec/transport failure. The message is safe for logs but NOT
    /// guaranteed safe for clients — route handlers map it to a generic 503.
    #[error("engine observe failed: {0}")]
    Failed(String),
}

/// The stderr marker the engine emits when a session has no durable store
/// (verified against fkst-substrate `observe.rs` / `delivery_store.rs`): the
/// offline fallback fails with "open existing durable delivery database …".
/// Both backends classify a non-zero observe exit through this marker.
pub(crate) const ENGINE_OBSERVE_NO_STORE_MARKER: &str = "open existing durable delivery database";

/// A live session runtime projected into a kube-free handle: the identity + repo the
/// fleet-wide loops (token rotation, health scrape, sweep) address a session by. The
/// concrete backend derives it from a runtime's stamped metadata.
#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub session_id: String,
    pub installation_id: i64,
    pub repo: RepoRef,
    /// The trigger-issue number, if the runtime carries a parseable non-zero one.
    pub trigger_issue: Option<u64>,
}

/// What delivering a credential into a live session did: it landed, or the session's
/// runtime had already vanished (the 404-equivalent the rotation loop treats as a
/// benign no-op — a deleted pod/Secret needs no fresh token).
#[derive(Debug, PartialEq, Eq)]
pub enum DeliveryOutcome {
    Delivered,
    SessionGone,
}

/// The coarse runtime-status facts the health scrape reads, projected kube-free. A
/// gone/absent runtime is the [`Default`] (all `None`), which the scrape treats as
/// "nothing to see". `restart_count` is an `Option<u32>` here (the concrete backend
/// adapts the raw signed count outside the pure health evaluator).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeStatus {
    pub phase: Option<String>,
    pub restart_count: Option<u32>,
    pub stall_reason: Option<String>,
}

/// The inputs the backend needs to validate a named environment's install commands
/// in a throwaway isolated runtime (kube-free view of the env-validation request).
#[derive(Debug, Clone)]
pub struct ValidationRequest {
    pub github_user_id: i64,
    pub name: String,
    pub install: Vec<String>,
    pub variables: BTreeMap<String, String>,
}

/// The result of validating an environment's install commands (moved out of the
/// Kubernetes env-validator so the trait surface stays kube-free).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationOutcome {
    /// Every install command exited zero. `commands` is how many ran.
    Passed { commands: usize },
    /// A command failed, the sequence timed out, or the pod produced no trusted
    /// verdict. Carries the detail the REST layer renders in its 422.
    Failed {
        failed_command_index: u32,
        failed_command: String,
        exit_code: i32,
        timed_out: bool,
        stderr_tail: String,
    },
}

/// The contract the reconciler + its loops drive one session runtime (and the fleet)
/// through. The lifecycle verbs (#412) cover probe/ensure/observe/mark-pending/stop/GC;
/// the fleet verbs (#413) cover enumerating live sessions, delivering a rotated
/// credential, reading a session's status + recent output, and running/reaping an
/// env-validation runtime. Implementations are the only place a concrete runtime
/// (Kubernetes today) is touched.
#[async_trait]
pub trait SessionBackend: Send + Sync {
    /// Probe the backend is reachable, returning its reported status string (for the
    /// Kubernetes backend, the apiserver `major.minor` version).
    async fn check_reachable(&self) -> Result<String, BackendError>;

    /// Ensure a session runtime exists for `spec`, injecting the assembled `creds`.
    /// Idempotent: an already-live session is an [`EnsureOutcome::AlreadyLive`] no-op.
    async fn ensure_session(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError>;

    /// Observe the live (or terminal) session runtimes belonging to `repo`, projected
    /// into the planner's [`LivePod`] view.
    async fn observe_repo(&self, repo: &RepoRef) -> Result<Vec<LivePod>, BackendError>;

    /// Refresh a live session's last-pending marker to now. A runtime that vanished
    /// between the plan and this call yields [`BackendError::NotFound`].
    async fn mark_pending(&self, session_id: &str) -> Result<(), BackendError>;

    /// Stop a session's runtime for `reason` (the reason is carried for the executor's
    /// log/contract, not the delete itself). An already-gone runtime yields
    /// [`BackendError::NotFound`].
    async fn stop_session(&self, session_id: &str, reason: KillReason) -> Result<(), BackendError>;

    /// GC a terminal session's runtime (and its owned resources). An already-gone
    /// runtime yields [`BackendError::NotFound`].
    async fn remove_terminal(&self, session_id: &str) -> Result<(), BackendError>;

    /// Enumerate every live session runtime as a kube-free [`SessionHandle`]. The
    /// fleet-wide loops (sweep, token rotation, health scrape) iterate this.
    async fn list_fleet(&self) -> Result<Vec<SessionHandle>, BackendError>;

    /// Deliver `contents` into a live session's mounted credential file `file`
    /// (in-place). [`DeliveryOutcome::SessionGone`] is the benign 404-equivalent (the
    /// runtime vanished); any other failure is a [`BackendError`]. The value is never
    /// logged.
    async fn deliver_credential(
        &self,
        session_id: &str,
        file: &str,
        contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError>;

    /// Read a session runtime's coarse status. An absent/gone runtime yields the
    /// default (empty) [`RuntimeStatus`] — the "nothing to see" the scrape expects.
    async fn status_summary(&self, session_id: &str) -> Result<RuntimeStatus, BackendError>;

    /// Read a bounded tail of a session runtime's recent output. The 3-state taxonomy
    /// the health scrape depends on is preserved INSIDE the `Option`: `Some(text)` =
    /// read OK, `Some("")` = the runtime is gone (benign empty window), `None` = the
    /// output could not be read at all (a transport error) — so the caller can
    /// withhold a health CLEAR it cannot justify. Best-effort: never propagates.
    async fn recent_output(&self, session_id: &str) -> Option<String>;

    /// Run the engine's observe read-model (`fkst-framework observe
    /// --durable-root … --json --limit N`) INSIDE the session's runtime and
    /// return its raw JSON stdout. Read-only; the engine never emits payload
    /// bodies (only schema/digest/byte counts), so the output is safe to serve
    /// to an authorized viewer. `limit` is pre-clamped by the caller to the
    /// engine's accepted 1..=10000.
    async fn engine_observe(&self, session_id: &str, limit: u32) -> Result<String, ObserveError>;

    /// Validate a named environment's install commands in a throwaway isolated
    /// runtime, returning the parsed verdict (the admission/concurrency guards stay
    /// in the caller — this verb owns only the runtime lifecycle).
    async fn run_validation(
        &self,
        req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError>;

    /// Reap validation runtimes left behind by a crashed control plane, returning how
    /// many were removed.
    async fn reap_stale_validations(&self) -> Result<usize, BackendError>;
}
