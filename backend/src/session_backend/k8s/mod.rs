//! The direct-Kubernetes [`SessionBackend`] implementation (issues #412 + #413).
//!
//! This is the concrete pod-driving machinery moved verbatim out of the reconciler
//! (`execute.rs` / `repo.rs` / `loops.rs`) and the standalone k8s loops
//! (`token_rotation.rs` / `health_scrape.rs` / `pod_logs.rs` / `env_validator.rs`),
//! placed behind the backend-neutral [`SessionBackend`] contract. Everything here —
//! the pod LIST + projections, the last-pending merge-patch, the grace-honouring
//! delete, the credential Secret patch, the status/log reads, and the env-validation
//! pod lifecycle — is the SAME code with the SAME logs and the SAME 404/409 tolerance
//! the callers had inline; only its HOME changed. The callers now reach it only
//! through `Arc<dyn SessionBackend>`.
//!
//! Split across submodules to keep every file under the 500-line limit: this module
//! owns the struct + the delegating trait impl + the helpers shared across
//! submodules; each verb family lives in its own file.

use std::collections::BTreeMap;

use async_trait::async_trait;
use k8s_openapi::api::core::v1::{Pod, Secret};
use kube::api::{Api, ListParams};
use secrecy::SecretString;

use crate::config::PodConfig;
use crate::k8s::session_launcher::{COMPONENT_LABEL_KEY, COMPONENT_LABEL_VALUE};
use crate::k8s::{KubeClient, SessionPodSpec};
use crate::models::RepoRef;
use crate::reconcile::desired::LivePod;

use crate::session_backend::ObserveError;

use super::{
    BackendError, DeliveryOutcome, EnsureOutcome, KillReason, RuntimeStatus, SessionBackend,
    SessionHandle, ValidationOutcome, ValidationRequest,
};

mod credential;
mod engine_observe;
mod fleet;
mod lifecycle;
mod status;
mod validation;

pub(crate) use engine_observe::classify_failure as classify_observe_failure;

/// The direct-Kubernetes session backend: drives one long-lived Pod (+ its
/// owner-referenced creds Secret) per substrate session, plus the throwaway
/// env-validation pods. Cheap to clone via its `Arc`-backed [`KubeClient`]; held by
/// the reconciler + loops as `Arc<dyn SessionBackend>`.
pub struct K8sBackend {
    kube: KubeClient,
    pod_config: PodConfig,
    termination_grace_secs: u64,
    /// The env-validation pod's `activeDeadlineSeconds` + the sweep-age budget.
    validate_deadline_secs: i64,
    /// The poll cadence while waiting for a validation pod to reach a terminal phase.
    validate_poll_interval_secs: u64,
}

impl K8sBackend {
    /// Build from the namespace-bound Kubernetes client, the pod-launch knobs, the
    /// configured termination grace (the delete drain window), and the env-validation
    /// deadline + poll cadence.
    pub fn new(
        kube: KubeClient,
        pod_config: PodConfig,
        termination_grace_secs: u64,
        validate_deadline_secs: i64,
        validate_poll_interval_secs: u64,
    ) -> Self {
        Self {
            kube,
            pod_config,
            termination_grace_secs,
            validate_deadline_secs,
            validate_poll_interval_secs,
        }
    }

    /// A namespaced Pod API bound to the reconciler's namespace.
    fn pods_api(&self) -> Api<Pod> {
        Api::namespaced(self.kube.client().clone(), self.kube.namespace())
    }

    /// A namespaced Secret API bound to the reconciler's namespace.
    fn secrets_api(&self) -> Api<Secret> {
        Api::namespaced(self.kube.client().clone(), self.kube.namespace())
    }

    /// LIST every substrate-session pod (the shared `COMPONENT_LABEL` selector every
    /// caller uses). Factored so `observe_repo` (repo-scoped) and `list_fleet`
    /// (fleet-wide) share one LIST + selector. A list error is a [`BackendError::Other`].
    async fn list_component_pods(&self) -> Result<Vec<Pod>, BackendError> {
        let selector = format!("{COMPONENT_LABEL_KEY}={COMPONENT_LABEL_VALUE}");
        let list = self
            .pods_api()
            .list(&ListParams::default().labels(&selector))
            .await
            .map_err(|e| BackendError::Other(anyhow::Error::new(e)))?;
        Ok(list.items)
    }
}

/// Read a pod annotation as `&str`, if present. Shared by the fleet + lifecycle
/// projections.
fn annotation<'a>(pod: &'a Pod, key: &str) -> Option<&'a str> {
    pod.metadata
        .annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .map(String::as_str)
}

#[async_trait]
impl SessionBackend for K8sBackend {
    async fn check_reachable(&self) -> Result<String, BackendError> {
        self.kube
            .check_reachable()
            .await
            .map_err(|e| BackendError::Other(anyhow::Error::new(e)))
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
