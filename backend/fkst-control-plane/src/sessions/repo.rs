//! Session persistence over the `sessions` collection.
//!
//! THE single CAS choke-point for session state: every status change in the
//! system goes through [`SessionRepo::transition`], a conditional
//! `find_one_and_update` whose filter pins both the `_id` and the set of
//! statuses the caller believes the document is in. A miss means somebody
//! else (a concurrent stop request, the driver task) won the race — callers
//! re-read and converge instead of overwriting.

use bson::{doc, Bson, Document};
use mongodb::options::ReturnDocument;
use mongodb::Collection;

use crate::db::Db;
use crate::error::AppError;
use crate::models::{SessionDoc, SessionStatus};

/// Error text stamped onto sessions orphaned by a pod restart.
pub const ORPHANED_ERROR: &str = "orphaned by pod restart";

/// Repository over the `sessions` collection. Cheap to clone (the driver's
/// `Collection` is `Arc`-backed).
#[derive(Debug, Clone)]
pub struct SessionRepo {
    coll: Collection<SessionDoc>,
}

/// Serialize a status to its BSON string form (infallible for a unit enum
/// with `rename_all = "lowercase"`).
pub(crate) fn status_bson(status: SessionStatus) -> Bson {
    bson::to_bson(&status).expect("SessionStatus serializes to a string")
}

impl SessionRepo {
    /// Build the repository over the shared [`Db`] handle.
    pub fn new(db: &Db) -> Self {
        Self {
            coll: db.sessions(),
        }
    }

    /// Insert a freshly-created session document (status `pending`).
    pub async fn insert(&self, doc: &SessionDoc) -> Result<(), AppError> {
        self.coll.insert_one(doc).await.map_err(|error| {
            tracing::error!(session_id = %doc.id, error = %error, "session insert failed");
            AppError::Mongo(error)
        })?;
        tracing::info!(
            session_id = %doc.id,
            package_name = %doc.package_name,
            "session created"
        );
        Ok(())
    }

    /// Fetch one session by id.
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<SessionDoc>, AppError> {
        let found = self
            .coll
            .find_one(doc! { "_id": id })
            .await
            .map_err(|error| {
                tracing::error!(session_id = %id, error = %error, "session get failed");
                AppError::Mongo(error)
            })?;
        tracing::debug!(session_id = %id, found = found.is_some(), "session fetched");
        Ok(found)
    }

    /// Atomic compare-and-swap: apply `$set: set` to the session iff its
    /// current status is one of `from`. Returns the post-update document, or
    /// `None` when the filter missed (the document is absent or its status
    /// moved on). Callers MUST treat a miss as "re-read and converge", never
    /// as an error.
    pub async fn transition(
        &self,
        id: bson::Uuid,
        from: &[SessionStatus],
        set: Document,
    ) -> Result<Option<SessionDoc>, AppError> {
        self.transition_guarded(id, from, Document::new(), set)
            .await
    }

    /// [`Self::transition`] with extra equality guards merged into the CAS
    /// filter (e.g. `pod_id` + `fencing_token` ownership pins, so a write
    /// from a superseded pod can never land after a takeover). An empty
    /// `extra` is exactly `transition`.
    pub async fn transition_guarded(
        &self,
        id: bson::Uuid,
        from: &[SessionStatus],
        extra: Document,
        set: Document,
    ) -> Result<Option<SessionDoc>, AppError> {
        let from_bson: Vec<Bson> = from.iter().map(|status| status_bson(*status)).collect();
        let mut filter = doc! { "_id": id, "status": { "$in": from_bson } };
        filter.extend(extra);
        let updated = self
            .coll
            .find_one_and_update(filter, doc! { "$set": set })
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| {
                tracing::error!(session_id = %id, error = %error, "session transition failed");
                AppError::Mongo(error)
            })?;
        tracing::debug!(
            session_id = %id,
            from = ?from,
            applied = updated.is_some(),
            "session transition attempted"
        );
        Ok(updated)
    }

    /// Stamp the journaling layer's logical-run key onto a session. A
    /// DELIBERATELY narrow write: it sets ONLY `run_key` and never touches
    /// `status` (every status change stays inside [`Self::transition`]).
    /// Best-effort by contract — an absent document is a no-op, not an
    /// error (the driver may race a concurrent terminal sweep).
    pub async fn set_run_key(&self, id: bson::Uuid, run_key: &str) -> Result<(), AppError> {
        let result = self
            .coll
            .update_one(doc! { "_id": id }, doc! { "$set": { "run_key": run_key } })
            .await
            .map_err(|error| {
                tracing::error!(session_id = %id, error = %error, "run_key stamp failed");
                AppError::Mongo(error)
            })?;
        tracing::debug!(
            session_id = %id,
            run_key = %run_key,
            matched = result.matched_count,
            "run_key stamped"
        );
        Ok(())
    }

    /// Mark every pre-terminal session `failed` with [`ORPHANED_ERROR`].
    ///
    /// v1 single-pod stopgap: this binary supervises engine processes only
    /// in-memory (a detached task per session), so after a pod restart any
    /// non-terminal document refers to a process that no longer exists (the
    /// engine children die with the pod). Run once at startup BEFORE the
    /// listener binds. Superseded by the multi-pod lease takeover work
    /// (#24/#26), which will reconcile instead of failing.
    pub async fn fail_orphans(&self) -> Result<u64, AppError> {
        let pre_terminal: Vec<Bson> = [
            SessionStatus::Pending,
            SessionStatus::Validating,
            SessionStatus::Running,
            SessionStatus::Stopping,
        ]
        .iter()
        .map(|status| status_bson(*status))
        .collect();
        let result = self
            .coll
            .update_many(
                doc! { "status": { "$in": pre_terminal } },
                doc! { "$set": {
                    "status": status_bson(SessionStatus::Failed),
                    "error": ORPHANED_ERROR,
                    "stopped_at": bson::DateTime::now(),
                } },
            )
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "orphan sweep failed");
                AppError::Mongo(error)
            })?;
        if result.modified_count > 0 {
            tracing::warn!(
                count = result.modified_count,
                "orphaned sessions failed by startup sweep"
            );
        } else {
            tracing::info!("orphan sweep found no pre-terminal sessions");
        }
        Ok(result.modified_count)
    }

    /// Return the ids of every pre-terminal session targeting `owner/name`
    /// (issue #108). Used to wake the matching local drivers after a bulk
    /// uninstall fail; the bulk CAS in [`Self::fail_active_for_repo`] stays the
    /// authoritative state change.
    pub async fn active_ids_for_repo(
        &self,
        owner: &str,
        name: &str,
    ) -> Result<Vec<bson::Uuid>, AppError> {
        let pre_terminal: Vec<Bson> = [
            SessionStatus::Pending,
            SessionStatus::Validating,
            SessionStatus::Running,
            SessionStatus::Stopping,
        ]
        .iter()
        .map(|status| status_bson(*status))
        .collect();
        let mut cursor = self
            .coll
            .find(doc! {
                "repo.owner": owner,
                "repo.name": name,
                "status": { "$in": pre_terminal },
            })
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "active_ids_for_repo find failed");
                AppError::Mongo(error)
            })?;
        let mut ids = Vec::new();
        while cursor.advance().await.map_err(|error| {
            tracing::error!(error = %error, "active_ids_for_repo cursor failed");
            AppError::Mongo(error)
        })? {
            let doc = cursor.deserialize_current().map_err(|error| {
                tracing::error!(error = %error, "active_ids_for_repo deserialize failed");
                AppError::Mongo(error)
            })?;
            ids.push(doc.id);
        }
        Ok(ids)
    }

    /// Fail every pre-terminal session targeting `owner/name` with `reason`
    /// (issue #108: the GitHub App was uninstalled from, or had the repo removed
    /// from, the repo a live session depends on). A bulk CAS over the
    /// pre-terminal statuses, mirroring [`Self::fail_orphans`]: the matched
    /// driver observes the terminal status on its next supervise tick and
    /// converges (its own CAS misses, so it stops without overwriting). Returns
    /// the count of sessions transitioned.
    ///
    /// The `reason` is operator-supplied, fixed text (never a secret or a
    /// payload value) — it is what the user sees on the failed session.
    pub async fn fail_active_for_repo(
        &self,
        owner: &str,
        name: &str,
        reason: &str,
    ) -> Result<u64, AppError> {
        let pre_terminal: Vec<Bson> = [
            SessionStatus::Pending,
            SessionStatus::Validating,
            SessionStatus::Running,
            SessionStatus::Stopping,
        ]
        .iter()
        .map(|status| status_bson(*status))
        .collect();
        let result = self
            .coll
            .update_many(
                doc! {
                    "repo.owner": owner,
                    "repo.name": name,
                    "status": { "$in": pre_terminal },
                },
                doc! { "$set": {
                    "status": status_bson(SessionStatus::Failed),
                    "error": reason,
                    "stopped_at": bson::DateTime::now(),
                } },
            )
            .await
            .map_err(|error| {
                tracing::error!(
                    repo_owner = %owner,
                    repo_name = %name,
                    error = %error,
                    "failing sessions for uninstalled repo failed"
                );
                AppError::Mongo(error)
            })?;
        if result.modified_count > 0 {
            tracing::warn!(
                repo_owner = %owner,
                repo_name = %name,
                count = result.modified_count,
                "failed active sessions after github app uninstall/repo-removal"
            );
        }
        Ok(result.modified_count)
    }

    /// Return the ids of every pre-terminal session whose repo owner is `owner`
    /// (#141). Used for the owner-wide uninstall path (an `installation deleted`
    /// / `suspend` that enumerates no concrete repos) so the matching local
    /// drivers can be woken; the bulk CAS stays authoritative.
    pub async fn active_ids_for_owner(&self, owner: &str) -> Result<Vec<bson::Uuid>, AppError> {
        let pre_terminal: Vec<Bson> = [
            SessionStatus::Pending,
            SessionStatus::Validating,
            SessionStatus::Running,
            SessionStatus::Stopping,
        ]
        .iter()
        .map(|status| status_bson(*status))
        .collect();
        let mut cursor = self
            .coll
            .find(doc! {
                "repo.owner": owner,
                "status": { "$in": pre_terminal },
            })
            .await
            .map_err(|error| {
                tracing::error!(error = %error, "active_ids_for_owner find failed");
                AppError::Mongo(error)
            })?;
        let mut ids = Vec::new();
        while cursor.advance().await.map_err(|error| {
            tracing::error!(error = %error, "active_ids_for_owner cursor failed");
            AppError::Mongo(error)
        })? {
            let doc = cursor.deserialize_current().map_err(|error| {
                tracing::error!(error = %error, "active_ids_for_owner deserialize failed");
                AppError::Mongo(error)
            })?;
            ids.push(doc.id);
        }
        Ok(ids)
    }

    /// Fail every pre-terminal session whose repo owner is `owner` with `reason`
    /// (#141: the GitHub App was uninstalled from / suspended on an account that
    /// did not enumerate concrete repos). Same bulk-CAS discipline as
    /// [`Self::fail_active_for_repo`]; returns the count transitioned.
    pub async fn fail_active_for_owner(&self, owner: &str, reason: &str) -> Result<u64, AppError> {
        let pre_terminal: Vec<Bson> = [
            SessionStatus::Pending,
            SessionStatus::Validating,
            SessionStatus::Running,
            SessionStatus::Stopping,
        ]
        .iter()
        .map(|status| status_bson(*status))
        .collect();
        let result = self
            .coll
            .update_many(
                doc! {
                    "repo.owner": owner,
                    "status": { "$in": pre_terminal },
                },
                doc! { "$set": {
                    "status": status_bson(SessionStatus::Failed),
                    "error": reason,
                    "stopped_at": bson::DateTime::now(),
                } },
            )
            .await
            .map_err(|error| {
                tracing::error!(
                    repo_owner = %owner,
                    error = %error,
                    "failing sessions for uninstalled owner failed"
                );
                AppError::Mongo(error)
            })?;
        if result.modified_count > 0 {
            tracing::warn!(
                repo_owner = %owner,
                count = result.modified_count,
                "failed active sessions after github app account uninstall/suspend"
            );
        }
        Ok(result.modified_count)
    }
}
