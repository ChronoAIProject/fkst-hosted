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
        work_label: None,
    }
}

#[cfg(test)]
#[path = "observe_tests.rs"]
mod tests;
