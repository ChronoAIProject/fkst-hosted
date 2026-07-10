//! The coarse runtime-status read (issue #419): `status_summary`, feeding the
//! package-agnostic session-health scrape ([`crate::k8s::health_scrape`]).
//!
//! The scrape runs the SAME pure evaluator ([`crate::k8s::health_eval::evaluate_health`])
//! over both backends, so this verb must project a sandbox into a [`RuntimeStatus`]
//! whose `phase` speaks the evaluator's EXACT Kubernetes-pod phase vocabulary — a
//! byte-for-byte taxonomy match with the direct-Kubernetes backend's `status_summary`.
//! [`state_to_phase`] is the pure mapping that guarantees it (verified against
//! `health_eval::status_offender`): only `Failed` / `Unknown` degrade a session, and a
//! `Terminated` sandbox reads as a clean `Succeeded` (never degraded).

use crate::session_backend::opensandbox::dto::{SandboxState, SandboxView};
use crate::session_backend::{BackendError, RuntimeStatus};

use super::OsbBackend;

impl OsbBackend {
    pub(super) async fn status_summary_impl(
        &self,
        session_id: &str,
    ) -> Result<RuntimeStatus, BackendError> {
        match self.resolve_one(session_id).await {
            Ok(view) => Ok(runtime_status(&view)),
            // Gone/absent between the fleet LIST and this read → the default (empty)
            // status, the "nothing to see" the scrape expects (parity with k8s/status.rs).
            Err(BackendError::NotFound) => Ok(RuntimeStatus::default()),
            Err(error) => Err(error),
        }
    }
}

/// Project a resolved sandbox into the kube-free [`RuntimeStatus`] the scrape reads.
///
/// `restart_count` is `None` (a sandbox has no pod-style container restart signal) —
/// it round-trips to `0` in the evaluator, so it never triggers a false restart-degrade.
/// `stall_reason` carries the sandbox `reason` ONLY for a `Failed` sandbox (the sole
/// state where a reason is a genuine stall signal); for every other state it is `None`,
/// so a healthy sandbox's transient reason can never degrade it.
fn runtime_status(view: &SandboxView) -> RuntimeStatus {
    RuntimeStatus {
        phase: Some(state_to_phase(&view.state)),
        restart_count: None,
        stall_reason: view
            .reason
            .clone()
            .filter(|_| matches!(view.state, SandboxState::Failed)),
    }
}

/// Map a sandbox lifecycle state into the Kubernetes-pod phase vocabulary the pure
/// evaluator's `status_offender` branches on — byte-for-byte with the direct-Kubernetes
/// backend so a session's health verdict is identical on either runtime:
///
/// - `Failed` → `"Failed"` (a degrade trigger in `status_offender`);
/// - `Terminated` → `"Succeeded"` (a CLEAN exit — `status_offender` never degrades it);
/// - `Running` → `"Running"`;
/// - every transitional/pending state (`Pending` / `Pausing` / `Paused` / `Resuming` /
///   `Stopping`) → `"Pending"` (still starting — `status_offender` treats it as
///   not-yet-degraded);
/// - `Unknown` → `"Unknown"` (the other `status_offender` degrade trigger).
pub(super) fn state_to_phase(state: &SandboxState) -> String {
    match state {
        SandboxState::Failed => "Failed",
        SandboxState::Terminated => "Succeeded",
        SandboxState::Running => "Running",
        SandboxState::Pending
        | SandboxState::Pausing
        | SandboxState::Paused
        | SandboxState::Resuming
        | SandboxState::Stopping => "Pending",
        SandboxState::Unknown(_) => "Unknown",
    }
    .to_string()
}

#[cfg(test)]
#[path = "health_tests.rs"]
mod tests;
