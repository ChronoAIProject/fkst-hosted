//! A shared, recording + scriptable [`SessionBackend`] fake (issue #413), promoted
//! out of the reconciler's executor tests so the reconcile loops AND the k8s loops
//! (token rotation, health scrape) can drive the seam without a cluster. Every
//! lifecycle verb records its arguments; the fleet verbs return scripted values so a
//! test can assert what the loops did with them.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Mutex;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::k8s::SessionPodSpec;
use crate::models::RepoRef;
use crate::reconcile::desired::{KillReason, LivePod};

use super::{
    BackendError, DeliveryOutcome, EnsureOutcome, RuntimeStatus, SessionBackend, SessionHandle,
    ValidationOutcome, ValidationRequest,
};

/// A recording + scriptable [`SessionBackend`] fake. Lifecycle verbs push their
/// arguments into a `Mutex<Vec<..>>`; fleet verbs read the scripted `fleet` /
/// `gone_sessions` / `recent`. Returns `Ok` by default; `mark_pending` can be made to
/// return [`BackendError::NotFound`] to exercise the 404-swallow path.
#[derive(Default)]
pub(crate) struct FakeSessionBackend {
    pub(crate) ensured: Mutex<Vec<(String, Vec<String>)>>,
    pub(crate) marked_pending: Mutex<Vec<String>>,
    pub(crate) stopped: Mutex<Vec<(String, KillReason)>>,
    pub(crate) removed_terminal: Mutex<Vec<String>>,
    /// Each `deliver_credential` call as `(session_id, file)` (never the value).
    pub(crate) delivered: Mutex<Vec<(String, String)>>,
    mark_pending_not_found: bool,
    /// The fleet `list_fleet` returns.
    fleet: Vec<SessionHandle>,
    /// The pods `observe_repo` returns (regardless of repo).
    observed: Vec<LivePod>,
    /// Sessions whose `deliver_credential` reports [`DeliveryOutcome::SessionGone`].
    gone_sessions: HashSet<String>,
    /// Per-session scripted `recent_output` (absent → `None`).
    recent: HashMap<String, Option<String>>,
}

impl FakeSessionBackend {
    pub(crate) fn with_mark_pending_not_found() -> Self {
        Self {
            mark_pending_not_found: true,
            ..Default::default()
        }
    }

    /// Script the fleet `list_fleet` returns.
    pub(crate) fn with_fleet(mut self, fleet: Vec<SessionHandle>) -> Self {
        self.fleet = fleet;
        self
    }

    /// Script the pods `observe_repo` returns (any repo).
    pub(crate) fn with_observed(mut self, observed: Vec<LivePod>) -> Self {
        self.observed = observed;
        self
    }

    /// Mark a session's credential delivery as landing on an already-gone runtime.
    pub(crate) fn with_gone(mut self, session_id: &str) -> Self {
        self.gone_sessions.insert(session_id.to_string());
        self
    }

    /// Script a session's `recent_output` (the 3-state taxonomy: `Some(text)` /
    /// `Some("")` / `None`).
    pub(crate) fn with_recent(mut self, session_id: &str, output: Option<String>) -> Self {
        self.recent.insert(session_id.to_string(), output);
        self
    }
}

#[async_trait]
impl SessionBackend for FakeSessionBackend {
    async fn check_reachable(&self) -> Result<String, BackendError> {
        Ok("fake".to_string())
    }

    async fn ensure_session(
        &self,
        spec: &SessionPodSpec,
        creds: BTreeMap<String, SecretString>,
    ) -> Result<EnsureOutcome, BackendError> {
        // Record the session id + the assembled creds KEYS (never their values).
        let keys: Vec<String> = creds.keys().cloned().collect();
        self.ensured
            .lock()
            .unwrap()
            .push((spec.session_id.clone(), keys));
        Ok(EnsureOutcome::Created)
    }

    async fn observe_repo(&self, _repo: &RepoRef) -> Result<Vec<LivePod>, BackendError> {
        Ok(self.observed.clone())
    }

    async fn mark_pending(&self, session_id: &str) -> Result<(), BackendError> {
        self.marked_pending
            .lock()
            .unwrap()
            .push(session_id.to_string());
        if self.mark_pending_not_found {
            Err(BackendError::NotFound)
        } else {
            Ok(())
        }
    }

    async fn stop_session(&self, session_id: &str, reason: KillReason) -> Result<(), BackendError> {
        self.stopped
            .lock()
            .unwrap()
            .push((session_id.to_string(), reason));
        Ok(())
    }

    async fn remove_terminal(&self, session_id: &str) -> Result<(), BackendError> {
        self.removed_terminal
            .lock()
            .unwrap()
            .push(session_id.to_string());
        Ok(())
    }

    async fn list_fleet(&self) -> Result<Vec<SessionHandle>, BackendError> {
        Ok(self.fleet.clone())
    }

    async fn deliver_credential(
        &self,
        session_id: &str,
        file: &str,
        _contents: SecretString,
    ) -> Result<DeliveryOutcome, BackendError> {
        self.delivered
            .lock()
            .unwrap()
            .push((session_id.to_string(), file.to_string()));
        if self.gone_sessions.contains(session_id) {
            Ok(DeliveryOutcome::SessionGone)
        } else {
            Ok(DeliveryOutcome::Delivered)
        }
    }

    async fn status_summary(&self, _session_id: &str) -> Result<RuntimeStatus, BackendError> {
        Ok(RuntimeStatus::default())
    }

    async fn recent_output(&self, session_id: &str) -> Option<String> {
        self.recent.get(session_id).cloned().unwrap_or(None)
    }

    async fn engine_observe(
        &self,
        _session_id: &str,
        _limit: u32,
    ) -> Result<String, crate::session_backend::ObserveError> {
        // The fake serves a minimal valid snapshot; per-case behavior is not
        // needed by any current test (route mapping is unit-tested directly).
        Ok("{}".to_string())
    }

    async fn run_validation(
        &self,
        _req: &ValidationRequest,
    ) -> Result<ValidationOutcome, BackendError> {
        Ok(ValidationOutcome::Passed { commands: 0 })
    }

    async fn reap_stale_validations(&self) -> Result<usize, BackendError> {
        Ok(0)
    }
}
