//! In-memory session store (database-free pivot, #198/#143).
//!
//! THE single CAS choke-point for session state: every status change in the
//! system goes through [`SessionRepo::transition`], a conditional update whose
//! filter pins both the `_id` and the set of statuses the caller believes the
//! document is in. A miss means somebody else (a concurrent stop request, the
//! driver task) won the race — callers re-read and converge instead of
//! overwriting.
//!
//! This was a Mongo `sessions` collection until the db-free pivot. It is now a
//! single-process `Mutex<HashMap>` owned by the authoritative controller. The
//! atomicity contract is PRESERVED and in fact strengthened: a Mongo
//! `find_one_and_update` is atomic per document; here the entire read-check-write
//! of [`Self::transition_guarded`] runs under one lock, so the `pod_id` +
//! `fencing_token` ownership guards (a superseded writer can never land after a
//! takeover) hold exactly as before. The trade-off is the db-free pivot's
//! accepted one: a controller restart loses in-flight in-memory sessions, which
//! are recovered from the still-running workers' OS-truth re-adoption (#136) and
//! the GitHub journal skip-set (#139) — never from a datastore.
//!
//! The generic `extra` (CAS guards) and `set` (field updates) the driver passes
//! are applied through a serialize round-trip (`SessionDoc` ⇄ `bson::Document`)
//! so the field-level semantics are byte-for-byte the ones the Mongo `$set` /
//! equality filters produced — no caller changes were needed.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use bson::{Bson, Document};

use crate::error::AppError;
use crate::models::{SessionDoc, SessionStatus, TerminalCause};

/// Error text stamped onto sessions orphaned by a controller restart.
pub const ORPHANED_ERROR: &str = "orphaned by pod restart";

/// In-memory store over session documents, keyed by `_id`. Cheap to clone (the
/// map is `Arc`-backed and shared by every driver this controller spawns).
#[derive(Debug, Clone, Default)]
pub struct SessionRepo {
    inner: Arc<Mutex<HashMap<bson::Uuid, SessionDoc>>>,
}

/// Serialize a status to its BSON string form (infallible for a unit enum
/// with `rename_all = "lowercase"`). Retained because the driver builds the
/// `$set` update documents with it.
pub(crate) fn status_bson(status: SessionStatus) -> Bson {
    bson::to_bson(&status).expect("SessionStatus serializes to a string")
}

/// Serialize a terminal cause to its BSON string form (#180), for stamping
/// `terminal_cause` in a terminal `transition_guarded` update doc (infallible
/// for a unit enum with `rename_all = "snake_case"`).
pub(crate) fn terminal_cause_bson(cause: TerminalCause) -> Bson {
    bson::to_bson(&cause).expect("TerminalCause serializes to a string")
}

/// The pre-terminal statuses every bulk sweep targets.
const PRE_TERMINAL: [SessionStatus; 4] = [
    SessionStatus::Pending,
    SessionStatus::Validating,
    SessionStatus::Running,
    SessionStatus::Stopping,
];

fn is_pre_terminal(status: SessionStatus) -> bool {
    PRE_TERMINAL.contains(&status)
}

impl SessionRepo {
    /// Build an empty in-memory store. A fresh controller starts with no
    /// sessions (the db-free "controller loss" boundary).
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<bson::Uuid, SessionDoc>> {
        // A poisoned lock means a prior holder panicked mid-mutation; recover the
        // map rather than cascading the panic (the controller stays up).
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Insert a freshly-created session document (status `pending`). A duplicate
    /// `_id` is a conflict (mirrors the Mongo unique-`_id` insert), though
    /// callers always insert a fresh UUID.
    pub async fn insert(&self, doc: &SessionDoc) -> Result<(), AppError> {
        let mut map = self.lock();
        if map.contains_key(&doc.id) {
            tracing::error!(session_id = %doc.id, "session insert conflict: id already present");
            return Err(AppError::Conflict("session already exists".to_string()));
        }
        map.insert(doc.id, doc.clone());
        tracing::info!(
            session_id = %doc.id,
            package_name = %doc.package_name,
            "session created"
        );
        Ok(())
    }

    /// Fetch one session by id.
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<SessionDoc>, AppError> {
        let found = self.lock().get(&id).cloned();
        tracing::debug!(session_id = %id, found = found.is_some(), "session fetched");
        Ok(found)
    }

    /// Atomic compare-and-swap: apply `set` to the session iff its current
    /// status is one of `from`. Returns the post-update document, or `None` when
    /// the filter missed (the document is absent or its status moved on). Callers
    /// MUST treat a miss as "re-read and converge", never as an error.
    pub async fn transition(
        &self,
        id: bson::Uuid,
        from: &[SessionStatus],
        set: Document,
    ) -> Result<Option<SessionDoc>, AppError> {
        self.transition_guarded(id, from, Document::new(), set)
            .await
    }

    /// [`Self::transition`] with extra equality guards merged into the CAS filter
    /// (e.g. `pod_id` + `fencing_token` ownership pins, so a write from a
    /// superseded pod can never land after a takeover). An empty `extra` is
    /// exactly `transition`.
    ///
    /// The whole read-check-write runs under the map lock, so the CAS is atomic.
    /// `extra`/`set` are interpreted at the `bson::Document` level (equality on
    /// `extra` keys, `$set` semantics for `set` keys) against the document's
    /// serialized form — identical to the Mongo filter/update the driver built.
    pub async fn transition_guarded(
        &self,
        id: bson::Uuid,
        from: &[SessionStatus],
        extra: Document,
        set: Document,
    ) -> Result<Option<SessionDoc>, AppError> {
        let mut map = self.lock();
        let Some(current) = map.get(&id) else {
            tracing::debug!(session_id = %id, applied = false, "session transition: id miss");
            return Ok(None);
        };
        // Serialize once so the status `$in`, the `extra` equality guards, and the
        // `$set` all use the exact field shapes Mongo saw.
        let mut bson_doc = bson::to_document(current).map_err(serialize_err)?;

        // status ∈ from
        let from_bson: Vec<Bson> = from.iter().map(|s| status_bson(*s)).collect();
        let status_matches = bson_doc
            .get("status")
            .is_some_and(|cur| from_bson.iter().any(|f| f == cur));
        // every `extra` equality guard holds (an absent field never equals a
        // concrete guard value — exactly Mongo's equality-on-absent semantics).
        let guards_match = extra.iter().all(|(k, v)| bson_doc.get(k) == Some(v));

        if !(status_matches && guards_match) {
            tracing::debug!(session_id = %id, from = ?from, applied = false, "session transition: CAS miss");
            return Ok(None);
        }

        for (k, v) in &set {
            bson_doc.insert(k.clone(), v.clone());
        }
        let updated: SessionDoc = bson::from_document(bson_doc).map_err(deserialize_err)?;
        map.insert(id, updated.clone());
        tracing::debug!(session_id = %id, from = ?from, applied = true, "session transition applied");
        Ok(Some(updated))
    }

    /// Stamp the journaling layer's logical-run key onto a session. A
    /// DELIBERATELY narrow write: it sets ONLY `run_key` and never touches
    /// `status`. Best-effort by contract — an absent document is a no-op, not an
    /// error (the driver may race a concurrent terminal sweep).
    pub async fn set_run_key(&self, id: bson::Uuid, run_key: &str) -> Result<(), AppError> {
        let mut map = self.lock();
        let matched = match map.get_mut(&id) {
            Some(doc) => {
                doc.run_key = Some(run_key.to_string());
                true
            }
            None => false,
        };
        tracing::debug!(session_id = %id, run_key = %run_key, matched, "run_key stamped");
        Ok(())
    }

    /// Mark every pre-terminal session `failed` with [`ORPHANED_ERROR`]. v1
    /// startup stopgap; run once before the listener binds. Returns the count
    /// transitioned.
    pub async fn fail_orphans(&self) -> Result<u64, AppError> {
        let count = self.bulk_fail(|_| true, ORPHANED_ERROR);
        if count > 0 {
            tracing::warn!(count, "orphaned sessions failed by startup sweep");
        } else {
            tracing::info!("orphan sweep found no pre-terminal sessions");
        }
        Ok(count)
    }

    /// Ids of every pre-terminal session targeting `owner/name` (issue #108).
    pub async fn active_ids_for_repo(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Vec<bson::Uuid>, AppError> {
        Ok(self.active_ids(|doc| repo_matches(doc, owner, Some(name))))
    }

    /// Fail every pre-terminal session targeting `owner/name` with `reason`
    /// (issue #108). Returns the count transitioned. `reason` is fixed operator
    /// text, never a secret.
    pub async fn fail_active_for_repo(
        &self,
        owner: &str,
        name: &str,
        reason: &str,
    ) -> Result<u64, AppError> {
        let count = self.bulk_fail(|doc| repo_matches(doc, owner, Some(name)), reason);
        if count > 0 {
            tracing::warn!(
                repo_owner = %owner,
                repo_name = %name,
                count,
                "failed active sessions after github app uninstall/repo-removal"
            );
        }
        Ok(count)
    }

    /// Ids of every pre-terminal session whose repo owner is `owner` (#141).
    pub async fn active_ids_for_owner(&self, owner: &str) -> Result<Vec<bson::Uuid>, AppError> {
        Ok(self.active_ids(|doc| repo_matches(doc, owner, None)))
    }

    /// Fail every pre-terminal session whose repo owner is `owner` with `reason`
    /// (#141). Returns the count transitioned.
    pub async fn fail_active_for_owner(&self, owner: &str, reason: &str) -> Result<u64, AppError> {
        let count = self.bulk_fail(|doc| repo_matches(doc, owner, None), reason);
        if count > 0 {
            tracing::warn!(
                repo_owner = %owner,
                count,
                "failed active sessions after github app account uninstall/suspend"
            );
        }
        Ok(count)
    }

    /// Collect ids of pre-terminal sessions matching `pred` (a bulk find).
    fn active_ids(&self, pred: impl Fn(&SessionDoc) -> bool) -> Vec<bson::Uuid> {
        self.lock()
            .values()
            .filter(|doc| is_pre_terminal(doc.status) && pred(doc))
            .map(|doc| doc.id)
            .collect()
    }

    /// Atomically fail every pre-terminal session matching `pred` (a bulk CAS).
    fn bulk_fail(&self, pred: impl Fn(&SessionDoc) -> bool, reason: &str) -> u64 {
        let mut map = self.lock();
        let now = bson::DateTime::now();
        let mut count = 0u64;
        for doc in map.values_mut() {
            if is_pre_terminal(doc.status) && pred(doc) {
                doc.status = SessionStatus::Failed;
                doc.error = Some(reason.to_string());
                doc.stopped_at = Some(now);
                count += 1;
            }
        }
        count
    }
}

/// A document's repo matches `owner` and (optionally) `name`.
fn repo_matches(doc: &SessionDoc, owner: &str, name: Option<&str>) -> bool {
    doc.repo
        .as_ref()
        .is_some_and(|r| r.owner == owner && name.is_none_or(|n| r.name == n))
}

fn serialize_err(error: bson::ser::Error) -> AppError {
    tracing::error!(error = %error, "session doc serialize failed (in-memory CAS)");
    AppError::Internal(anyhow::anyhow!("session doc serialize failed"))
}

fn deserialize_err(error: bson::de::Error) -> AppError {
    tracing::error!(error = %error, "session doc deserialize failed (in-memory CAS)");
    AppError::Internal(anyhow::anyhow!("session doc deserialize failed"))
}

#[cfg(test)]
#[path = "repo_tests.rs"]
mod tests;
