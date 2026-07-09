//! The stop / GC / enumerate verbs (issue #418): `stop_session`, `remove_terminal`,
//! and `list_fleet` (with the duplicate reaper).
//!
//! `stop_session` and `remove_terminal` share one shape: resolve the sandbox, delete
//! it, and RECORD the respawn shield (see [`super::observe`]) so the next observe
//! reports the session `Terminating` rather than `Absent`. `list_fleet` enumerates the
//! whole managed fleet and, as the single-writer backstop, reaps any duplicate sandbox
//! for a session down to the oldest survivor before projecting it to a handle.

use std::collections::BTreeMap;

use crate::models::RepoRef;
use crate::reconcile::desired::KillReason;
use crate::session_backend::opensandbox::dto::{OsbError, SandboxView};
use crate::session_backend::{BackendError, SessionHandle};

use super::{correlate, pick_oldest_index, OsbBackend};

impl OsbBackend {
    pub(super) async fn stop_session_impl(
        &self,
        session_id: &str,
        _reason: KillReason,
    ) -> Result<(), BackendError> {
        // `_reason` is part of the contract (the executor logs it); the delete does not
        // need it. Already-gone → the `NotFound` `resolve_one` returns propagates as
        // the benign 404-equivalent the executor swallows.
        let view = self.resolve_one(session_id).await?;
        self.delete_and_shield(session_id, view).await
    }

    pub(super) async fn remove_terminal_impl(&self, session_id: &str) -> Result<(), BackendError> {
        let view = self.resolve_one(session_id).await?;
        self.delete_and_shield(session_id, view).await
    }

    /// Delete the resolved sandbox and record the respawn shield. The shield is
    /// recorded regardless of the delete outcome: `observe_repo` injects the synthetic
    /// Terminating pod only when the session is ABSENT from the live list, so a shield
    /// left behind by a failed delete (sandbox still present) is inert.
    async fn delete_and_shield(
        &self,
        session_id: &str,
        view: SandboxView,
    ) -> Result<(), BackendError> {
        let sandbox_id = view.id.clone();
        let (repo, trigger) = shield_key_from_view(&view);
        let outcome = self.lifecycle.delete_sandbox(&sandbox_id).await;
        self.record_shield(session_id, repo, trigger);
        match outcome {
            Ok(()) => Ok(()),
            // Already gone between resolve and delete → benign NotFound.
            Err(OsbError::NotFound) => Err(BackendError::NotFound),
            Err(other) => Err(other.into()),
        }
    }

    pub(super) async fn list_fleet_impl(&self) -> Result<Vec<SessionHandle>, BackendError> {
        let filter = vec![(correlate::KEY_MANAGED.to_string(), "true".to_string())];
        let views = self.lifecycle.list_sandboxes(&filter).await?;

        // Group by session id; a managed sandbox missing the correlation key is a
        // malformed stray — skipped, never grouped.
        let mut groups: BTreeMap<String, Vec<SandboxView>> = BTreeMap::new();
        for view in views {
            match view.metadata.get(correlate::KEY_SESSION_ID).cloned() {
                Some(session_id) => groups.entry(session_id).or_default().push(view),
                None => tracing::warn!(
                    sandbox_id = %view.id,
                    "opensandbox list_fleet: managed sandbox missing fkst-session-id; skipping"
                ),
            }
        }

        let mut handles = Vec::new();
        for (session_id, mut dupes) in groups {
            // REAPER: keep the OLDEST by (created_at, id); delete every duplicate.
            let keep = pick_oldest_index(&dupes);
            let survivor = dupes.remove(keep);
            for dupe in &dupes {
                tracing::warn!(
                    session_id = %session_id,
                    kept = %survivor.id,
                    duplicate = %dupe.id,
                    "opensandbox list_fleet: reaping duplicate sandbox for one session"
                );
                if let Err(error) = self.lifecycle.delete_sandbox(&dupe.id).await {
                    tracing::warn!(
                        session_id = %session_id,
                        sandbox_id = %dupe.id,
                        error = %error,
                        "opensandbox list_fleet: duplicate reap delete failed (retried next sweep)"
                    );
                }
            }
            // Project the survivor to a handle; an unrecoverable survivor is skipped,
            // never a panic.
            match correlate::recover(&survivor) {
                Some(handle) => handles.push(handle),
                None => tracing::warn!(
                    session_id = %session_id,
                    sandbox_id = %survivor.id,
                    "opensandbox list_fleet: survivor unrecoverable from metadata; skipping"
                ),
            }
        }
        Ok(handles)
    }
}

/// Recover the shield key `(repo, trigger_issue)` from a resolved sandbox's metadata.
/// Missing owner/repo default to empty (the shield entry then matches no real repo —
/// inert), and a missing/unparseable trigger defaults to `0`.
fn shield_key_from_view(view: &SandboxView) -> (RepoRef, i64) {
    let owner = view
        .metadata
        .get(correlate::KEY_OWNER)
        .cloned()
        .unwrap_or_default();
    let name = view
        .metadata
        .get(correlate::KEY_REPO)
        .cloned()
        .unwrap_or_default();
    let trigger = view
        .metadata
        .get(correlate::KEY_TRIGGER_ISSUE)
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or(0);
    (RepoRef { owner, name }, trigger)
}

#[cfg(test)]
#[path = "fleet_tests.rs"]
mod tests;
