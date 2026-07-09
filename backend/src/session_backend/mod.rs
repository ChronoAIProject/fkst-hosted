//! Backend-neutral session-runtime abstraction (issue #412).
//!
//! The reconciler drives session runtimes through the [`SessionBackend`] trait and
//! never touches a concrete Kubernetes type: the executor holds only an
//! `Arc<dyn SessionBackend>` (on [`crate::reconcile::execute::ReconcileCtx`]). The
//! direct-Kubernetes implementation lives in [`k8s::K8sBackend`]; keeping it behind
//! this contract makes the pod-driving machinery a plug-and-play unit that a future
//! runtime (e.g. an OpenSandbox backend) can replace without touching the planner or
//! the per-repo driver. This module is deliberately KUBE-FREE — no `kube` /
//! `k8s_openapi` type appears in the trait surface.

use std::collections::BTreeMap;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, LivePod};

pub mod k8s;

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

/// The contract the reconciler drives one session runtime through. Six verbs cover
/// the whole lifecycle the planner emits actions for: probe reachability, ensure a
/// session exists, observe a repo's live sessions, refresh a session's pending
/// marker, stop a session, and GC a terminal one. Implementations are the only place
/// a concrete runtime (Kubernetes today) is touched.
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
}
