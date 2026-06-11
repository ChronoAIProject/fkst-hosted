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
        let from_bson: Vec<Bson> = from.iter().map(|status| status_bson(*status)).collect();
        let updated = self
            .coll
            .find_one_and_update(
                doc! { "_id": id, "status": { "$in": from_bson } },
                doc! { "$set": set },
            )
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
}
