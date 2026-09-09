//! The observe-side verbs (issue #418): `observe_repo` (+ the respawn shield) and
//! `mark_pending`.
//!
//! ## Respawn shield (why it exists)
//! An OpenSandbox delete 404s instantly — there is no Kubernetes `deletionTimestamp`
//! window and the list is read-your-writes, so a session deleted THIS reconcile tick
//! is already ABSENT from the very next `observe_repo`. The planner would then see
//! Absent the same tick it issued the kill and immediately re-spawn (or, for an
//! orphan, re-issue the delete) — a thrash. The shield records each just-deleted
//! session for `reconcile_window` and injects a synthetic **Terminating** [`LivePod`]
//! for it while still absent: the planner leaves a `Terminating` pod strictly alone
//! (`plan_repo` — both the desired-session `Terminating` arm and the orphan
//! `Absent | Terminating` arm are no-ops), reproducing the "kill observed before
//! respawn" pacing K8s gets for free. Kubernetes needs no such shield (its create is
//! 409-idempotent and its delete leaves a Terminating pod behind).

use std::collections::{BTreeMap, HashSet};

use k8s_openapi::chrono::Utc;

use crate::models::RepoRef;
use crate::reconcile::desired::{LivePod, PodLiveness};
use crate::runtime_identity::{
    plan as plan_identity, IdentityPlan, RuntimeIdentityMetadata, RuntimeIdentityOutcome,
    OSB_IDENTITY_KEYS,
};
use crate::session_backend::BackendError;

use super::{correlate, OsbBackend};

impl OsbBackend {
    pub(super) async fn observe_repo_impl(
        &self,
        repo: &RepoRef,
    ) -> Result<Vec<LivePod>, BackendError> {
        // Server-side metadata filter: managed + owner + repo (owner/repo are stored
        // RAW, so the filter values match the stamp exactly).
        let filter = vec![
            (correlate::KEY_MANAGED.to_string(), "true".to_string()),
            (correlate::KEY_OWNER.to_string(), repo.owner.clone()),
            (correlate::KEY_REPO.to_string(), repo.name.clone()),
        ];
        let views = self.lifecycle.list_sandboxes(&filter).await?;
        let mut pods: Vec<LivePod> = views.iter().filter_map(correlate::to_live_pod).collect();

        // Merge the respawn shield (see the module doc): inject a synthetic
        // Terminating pod for each still-shielded session that is now absent.
        let present: HashSet<String> = pods.iter().map(|p| p.session_id.clone()).collect();
        for (session_id, trigger) in self.drain_shield_for_repo(repo) {
            if !present.contains(&session_id) {
                pods.push(synthetic_terminating(session_id, trigger));
            }
        }
        Ok(pods)
    }

    pub(super) async fn mark_pending_impl(&self, session_id: &str) -> Result<(), BackendError> {
        let view = self.resolve_one(session_id).await?;
        let mut patch = BTreeMap::new();
        patch.insert(
            correlate::KEY_LAST_PENDING.to_string(),
            Utc::now().timestamp().to_string(),
        );
        // A merge-patch touching ONLY the last-pending key leaves the rest of the
        // correlation metadata immutable. A 404 → `NotFound` (the runtime vanished).
        self.lifecycle.patch_metadata(&view.id, &patch).await?;
        Ok(())
    }

    /// Fill absent attribution metadata on the session's sandbox (issue #5673).
    ///
    /// Resolves the sandbox first so the decision is made against its CURRENT
    /// metadata, then merge-patches ONLY the absent keys through the existing
    /// metadata endpoint. Every patched value passes the same label-value
    /// validator the create-time stamp uses, so a legacy value the server would
    /// reject fails here as a bounded error instead of on the wire.
    pub(super) async fn ensure_runtime_identity_impl(
        &self,
        session_id: &str,
        identity: &RuntimeIdentityMetadata,
    ) -> Result<RuntimeIdentityOutcome, BackendError> {
        let view = match self.resolve_one(session_id).await {
            Ok(view) => view,
            Err(BackendError::NotFound) => return Ok(RuntimeIdentityOutcome::NotFound),
            Err(error) => return Err(error),
        };
        let missing = match plan_identity(&OSB_IDENTITY_KEYS, &view.metadata, identity) {
            IdentityPlan::Complete => return Ok(RuntimeIdentityOutcome::Unchanged),
            IdentityPlan::Conflict { key, marker } => {
                // The KEY is a bounded constant; the disagreeing VALUES are not
                // logged.
                tracing::warn!(
                    session_id = %session_id,
                    key = key,
                    "opensandbox runtime identity: stamped attribution disagrees with the current registration; leaving it untouched"
                );
                if let Some(marker) = marker {
                    self.record_identity_conflict(session_id, &view.id, marker)
                        .await;
                }
                return Ok(RuntimeIdentityOutcome::Conflict);
            }
            IdentityPlan::Backfill(missing) => missing,
        };

        let mut patch = BTreeMap::new();
        for (key, value) in &missing {
            correlate::put_metadata(&mut patch, key, value.clone())?;
        }
        match self.lifecycle.patch_metadata(&view.id, &patch).await {
            Ok(()) => {
                tracing::info!(
                    session_id = %session_id,
                    keys = missing.len(),
                    "opensandbox runtime identity: backfilled absent attribution metadata"
                );
                Ok(RuntimeIdentityOutcome::Backfilled)
            }
            Err(crate::session_backend::opensandbox::dto::OsbError::NotFound) => {
                Ok(RuntimeIdentityOutcome::NotFound)
            }
            Err(error) => Err(error.into()),
        }
    }

    /// Record the durable conflict marker on the sandbox.
    ///
    /// Best-effort for the same reason as the Kubernetes twin: the CONFLICT is
    /// the operation's true outcome and must be reported and audited even when
    /// this one additive metadata key cannot be written. The value still passes
    /// the shared label-value validator, so a marker the server would reject
    /// fails here rather than on the wire.
    async fn record_identity_conflict(
        &self,
        session_id: &str,
        sandbox_id: &str,
        marker: (&'static str, String),
    ) {
        let (key, value) = marker;
        let mut patch = BTreeMap::new();
        if let Err(error) = correlate::put_metadata(&mut patch, key, value.clone()) {
            tracing::warn!(
                session_id = %session_id,
                error = %error,
                "opensandbox runtime identity: attribution-conflict marker rejected by the metadata validator"
            );
            return;
        }
        match self.lifecycle.patch_metadata(sandbox_id, &patch).await {
            Ok(()) => tracing::info!(
                session_id = %session_id,
                field = %value,
                "opensandbox runtime identity: recorded a durable attribution-conflict marker"
            ),
            Err(error) => tracing::warn!(
                session_id = %session_id,
                error = %error,
                "opensandbox runtime identity: could not record the attribution-conflict marker; \
                 the conflict is still reported for this pass"
            ),
        }
    }
}

/// A synthetic `Terminating` [`LivePod`] the shield injects for a just-deleted,
/// now-absent session. Only `session_id` + `trigger_issue` carry meaning: the planner
/// does nothing with a `Terminating` pod, so the timestamps/hashes are placeholders.
fn synthetic_terminating(session_id: String, trigger: i64) -> LivePod {
    LivePod {
        session_id,
        trigger_issue: trigger,
        liveness: PodLiveness::Terminating,
        created_at: Utc::now(),
        last_pending_at: None,
        config_hash: None,
        work_labels: Vec::new(),
        // A shielded session has no runtime to read a stamp from; the empty
        // observation keeps the attribution backfill away from it, which is
        // exactly right for a runtime that is being deleted.
        identity: Default::default(),
    }
}

#[cfg(test)]
#[path = "observe_tests.rs"]
mod tests;
