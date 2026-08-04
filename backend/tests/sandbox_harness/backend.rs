//! A scripted [`SessionBackend`] that records every verb it is asked for.
//!
//! Two things make it worth its length. First, `list_runtime_inventory` is
//! counted, so "exactly one backend list per request" is a fact rather than a
//! hope. Second, EVERY other verb increments a forbidden-call counter — so
//! `list_fleet`, a per-runtime `status_summary`, a Pod-log read, or an exec would
//! be caught by a test that never mentions them (epic `SBOX-04`).

#![allow(dead_code)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use fkst_control_plane::k8s::SessionPodSpec;
use fkst_control_plane::models::RepoRef;
use fkst_control_plane::reconcile::desired::{KillReason, LivePod};
use fkst_control_plane::runtime_identity::{
    RuntimeBackendKind, RuntimeIdentityMetadata, RuntimeIdentityOutcome,
};
use fkst_control_plane::session_backend::inventory::{
    BoundedInventoryWarning, RuntimeInventoryItem, RuntimeInventorySnapshot, RuntimeLifetimePolicy,
};
use fkst_control_plane::session_backend::{
    BackendError, DeliveryOutcome, EnsureOutcome, ObserveError, RuntimeStatus, SessionBackend,
    SessionHandle, ValidationOutcome, ValidationRequest,
};
use k8s_openapi::chrono::{DateTime, Utc};
use secrecy::SecretString;

/// How the scripted backend answers one inventory read.
#[derive(Clone)]
pub enum InventoryScript {
    /// A healthy snapshot with these items and snapshot warnings.
    Snapshot {
        items: Vec<RuntimeInventoryItem>,
        warnings: Vec<BoundedInventoryWarning>,
        observed_at: DateTime<Utc>,
    },
    /// A generic backend failure.
    Failure,
    /// The backend's own oversize refusal.
    TooLarge { limit: usize },
    /// A read that outlives the route's bounded budget.
    Slow(Duration),
}

/// A recording, scriptable runtime backend.
pub struct ScriptedBackend {
    kind: RuntimeBackendKind,
    script: InventoryScript,
    inventory_calls: AtomicUsize,
    forbidden_calls: AtomicUsize,
}

impl ScriptedBackend {
    /// A Kubernetes backend answering with `script`.
    pub fn new(script: InventoryScript) -> Arc<Self> {
        Arc::new(Self {
            kind: RuntimeBackendKind::Kubernetes,
            script,
            inventory_calls: AtomicUsize::new(0),
            forbidden_calls: AtomicUsize::new(0),
        })
    }

    /// The same, presenting as the OpenSandbox backend.
    pub fn opensandbox(script: InventoryScript) -> Arc<Self> {
        Arc::new(Self {
            kind: RuntimeBackendKind::OpenSandbox,
            script,
            inventory_calls: AtomicUsize::new(0),
            forbidden_calls: AtomicUsize::new(0),
        })
    }

    /// How many inventory reads happened. `0` proves a refusal cost nothing.
    pub fn inventory_calls(&self) -> usize {
        self.inventory_calls.load(Ordering::SeqCst)
    }

    /// How many verbs the endpoint is forbidden to use were called.
    pub fn forbidden_calls(&self) -> usize {
        self.forbidden_calls.load(Ordering::SeqCst)
    }

    fn forbidden<T>(&self, verb: &str) -> Result<T, BackendError> {
        self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
        Err(BackendError::Other(anyhow::anyhow!(
            "the sandbox inventory endpoint must never call {verb}"
        )))
    }
}

#[async_trait]
impl SessionBackend for ScriptedBackend {
    fn backend_kind(&self) -> RuntimeBackendKind {
        self.kind
    }

    async fn list_runtime_inventory(
        &self,
        _policy: &RuntimeLifetimePolicy,
    ) -> Result<RuntimeInventorySnapshot, BackendError> {
        self.inventory_calls.fetch_add(1, Ordering::SeqCst);
        match &self.script {
            InventoryScript::Snapshot {
                items,
                warnings,
                observed_at,
            } => Ok(RuntimeInventorySnapshot {
                observed_at: *observed_at,
                backend: self.kind,
                items: items.clone(),
                warnings: warnings.clone(),
            }),
            InventoryScript::Failure => Err(BackendError::Other(anyhow::anyhow!(
                "scripted backend failure: apiserver at https://10.0.0.1:6443 said no"
            ))),
            InventoryScript::TooLarge { limit } => {
                Err(BackendError::InventoryTooLarge { limit: *limit })
            }
            InventoryScript::Slow(delay) => {
                tokio::time::sleep(*delay).await;
                Ok(RuntimeInventorySnapshot {
                    observed_at: Utc::now(),
                    backend: self.kind,
                    items: Vec::new(),
                    warnings: Vec::new(),
                })
            }
        }
    }

    async fn check_reachable(&self) -> Result<String, BackendError> {
        self.forbidden("check_reachable")
    }

    async fn ensure_runtime_identity(
        &self,
        _session_id: &str,
        _identity: &RuntimeIdentityMetadata,
    ) -> Result<RuntimeIdentityOutcome, BackendError> {
        self.forbidden("ensure_runtime_identity")
    }

    async fn ensure_session(
        &self,
        _spec: &SessionPodSpec,
        _creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        self.forbidden("ensure_session")
    }

    async fn credential_recovery_needed(&self, _session_id: &str) -> Result<bool, BackendError> {
        self.forbidden("credential_recovery_needed")
    }

    async fn observe_repo(&self, _repo: &RepoRef) -> Result<Vec<LivePod>, BackendError> {
        self.forbidden("observe_repo")
    }

    async fn mark_pending(&self, _session_id: &str) -> Result<(), BackendError> {
        self.forbidden("mark_pending")
    }

    async fn stop_session(
        &self,
        _session_id: &str,
        _reason: KillReason,
    ) -> Result<(), BackendError> {
        self.forbidden("stop_session")
    }

    async fn remove_terminal(&self, _session_id: &str) -> Result<(), BackendError> {
        self.forbidden("remove_terminal")
    }

    async fn list_fleet(&self) -> Result<Vec<SessionHandle>, BackendError> {
        self.forbidden("list_fleet")
    }

    async fn deliver_credential(
        &self,
        _session_id: &str,
        _file: &str,
        _contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        self.forbidden("deliver_credential")
    }

    async fn status_summary(&self, _session_id: &str) -> Result<RuntimeStatus, BackendError> {
        self.forbidden("status_summary")
    }

    async fn recent_output(&self, _session_id: &str) -> Option<String> {
        self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
        None
    }

    async fn engine_observe(&self, _session_id: &str, _limit: u32) -> Result<String, ObserveError> {
        self.forbidden_calls.fetch_add(1, Ordering::SeqCst);
        Err(ObserveError::Failed(
            "the sandbox inventory endpoint must never exec".to_string(),
        ))
    }

    async fn run_validation(
        &self,
        _req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        self.forbidden("run_validation")
    }

    async fn reap_stale_validations(&self) -> Result<usize, BackendError> {
        self.forbidden("reap_stale_validations")
    }
}
