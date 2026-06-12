//! MongoDB-backed repository for the `goals` collection.
//!
//! CAS-style mutations: `patch`, `delete`, `transition_status`, and
//! `set_active_session` use `find_one_and_update` / `find_one_and_delete` with
//! status filters so that `package_names` and `repo` can only be changed when
//! the goal is in an immutable state set `{not_started, stopped, failed}`.
//! Title and description are editable in any status.

use bson::doc;
use mongodb::options::{IndexOptions, ReturnDocument};
use mongodb::{Collection, IndexModel};

use super::model::{GoalDoc, GoalStatus, GOALS_COLLECTION};

/// Log a driver failure and wrap it as `AppError::Mongo`.
fn log_db_error(op: &'static str, error: mongodb::error::Error) -> crate::error::AppError {
    tracing::error!(op, error = %error, "mongodb operation failed on goals collection");
    crate::error::AppError::Mongo(error)
}

/// Repository over the `goals` collection. Cheap to clone (the driver
/// `Collection` shares the underlying connection pool).
#[derive(Clone)]
pub struct GoalRepo {
    coll: Collection<GoalDoc>,
}

impl GoalRepo {
    /// Bind the repository to the `goals` collection of `db`.
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            coll: db.collection::<GoalDoc>(GOALS_COLLECTION),
        }
    }

    /// Idempotent startup hook for `goals` indexes. Safe to call on every boot.
    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        let indexes = vec![
            IndexModel::builder()
                .keys(doc! { "owner_user_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("goals_owner_created".to_string())
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "org_id": 1, "created_at": -1 })
                .options(
                    IndexOptions::builder()
                        .name("goals_org_created".to_string())
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! { "status": 1 })
                .options(
                    IndexOptions::builder()
                        .name("goals_status".to_string())
                        .build(),
                )
                .build(),
        ];
        self.coll.create_indexes(indexes).await?;
        tracing::info!(collection = GOALS_COLLECTION, "goals indexes ensured");
        Ok(())
    }

    /// Insert a new goal document.
    pub async fn insert(&self, goal: &GoalDoc) -> Result<(), crate::error::AppError> {
        self.coll.insert_one(goal).await.map_err(|error| {
            tracing::error!(
                goal_id = %goal.id,
                error = %error,
                "goal insert failed"
            );
            log_db_error("insert", error)
        })?;
        tracing::info!(
            goal_id = %goal.id,
            packages = goal.package_names.len(),
            "goal created"
        );
        Ok(())
    }

    /// Fetch one goal by id. `Ok(None)` when absent.
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<GoalDoc>, crate::error::AppError> {
        let found = self
            .coll
            .find_one(doc! { "_id": id })
            .await
            .map_err(|error| log_db_error("get", error))?;
        tracing::debug!(goal_id = %id, hit = found.is_some(), "goal get");
        Ok(found)
    }

    /// List goals visible to `owner_user_id` plus the given `visible_org_ids`,
    /// optionally filtered by `status`, sorted by `created_at` descending.
    /// Pagination via `limit` (max 200) and `offset`.
    pub async fn list(
        &self,
        owner_user_id: &str,
        visible_org_ids: &[String],
        status: Option<GoalStatus>,
        limit: u64,
        offset: u64,
    ) -> Result<Vec<GoalDoc>, crate::error::AppError> {
        let mut or_branches = vec![doc! { "owner_user_id": owner_user_id }];
        if !visible_org_ids.is_empty() {
            or_branches.push(doc! { "org_id": { "$in": visible_org_ids } });
        }
        let mut filter = doc! { "$or": or_branches };
        if let Some(s) = status {
            filter.insert("status", bson::to_bson(&s).expect("GoalStatus serializes"));
        }

        let mut cursor = self
            .coll
            .find(filter)
            .sort(doc! { "created_at": -1 })
            .skip(offset)
            .limit(limit as i64)
            .await
            .map_err(|error| log_db_error("list", error))?;

        let mut results = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|error| log_db_error("list", error))?
        {
            results.push(
                cursor
                    .deserialize_current()
                    .map_err(|error| log_db_error("list", error))?,
            );
        }
        tracing::debug!(
            owner = owner_user_id,
            orgs = visible_org_ids.len(),
            count = results.len(),
            "goals listed"
        );
        Ok(results)
    }

    /// CAS-style patch: apply `$set` updates to a goal, enforcing that
    /// `package_names` and `repo` changes are only allowed when the goal is in
    /// `{not_started, stopped, failed}`. Returns the post-update document or
    /// `None` if the goal is absent or in an immutable status.
    pub async fn patch(
        &self,
        id: bson::Uuid,
        mutability_filter: Option<Vec<GoalStatus>>,
        set: bson::Document,
    ) -> Result<Option<GoalDoc>, crate::error::AppError> {
        let mut filter = doc! { "_id": id };

        // If the update touches mutable-only fields, require the goal to be in
        // an editable state.
        if let Some(allowed) = mutability_filter {
            let status_bson: Vec<bson::Bson> = allowed
                .iter()
                .map(|s| bson::to_bson(s).expect("GoalStatus serializes"))
                .collect();
            filter.insert("status", doc! { "$in": status_bson });
        }

        let updated = self
            .coll
            .find_one_and_update(filter, doc! { "$set": set })
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| log_db_error("patch", error))?;

        tracing::debug!(
            goal_id = %id,
            applied = updated.is_some(),
            "goal patch attempted"
        );
        Ok(updated)
    }

    /// CAS-style delete: only allowed when the goal is in `{not_started,
    /// stopped, failed}`. Returns `Some(doc)` on success, `None` if absent or
    /// in an immutable status.
    pub async fn delete(
        &self,
        id: bson::Uuid,
        allowed_statuses: Vec<GoalStatus>,
    ) -> Result<Option<GoalDoc>, crate::error::AppError> {
        let status_bson: Vec<bson::Bson> = allowed_statuses
            .iter()
            .map(|s| bson::to_bson(s).expect("GoalStatus serializes"))
            .collect();
        let filter = doc! {
            "_id": id,
            "status": { "$in": status_bson }
        };

        let deleted = self
            .coll
            .find_one_and_delete(filter)
            .await
            .map_err(|error| log_db_error("delete", error))?;

        if deleted.is_some() {
            tracing::info!(goal_id = %id, "goal deleted");
        } else {
            tracing::debug!(goal_id = %id, "goal not deleted (absent or wrong status)");
        }
        Ok(deleted)
    }

    /// CAS: transition goal status, used by trigger/stop/fail flows.
    ///
    /// Atomically sets `$set: set` on the goal iff its current status is one of
    /// `from_statuses`. Returns the post-update document, or `None` when the
    /// filter missed (document absent or status moved on).
    pub async fn transition_status(
        &self,
        id: bson::Uuid,
        from_statuses: &[GoalStatus],
        set: bson::Document,
    ) -> Result<Option<GoalDoc>, crate::error::AppError> {
        let status_bson: Vec<bson::Bson> = from_statuses
            .iter()
            .map(|s| bson::to_bson(s).expect("GoalStatus serializes"))
            .collect();
        let filter = doc! {
            "_id": id,
            "status": { "$in": status_bson }
        };

        let updated = self
            .coll
            .find_one_and_update(filter, doc! { "$set": set })
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| log_db_error("transition_status", error))?;

        tracing::debug!(
            goal_id = %id,
            from = ?from_statuses,
            applied = updated.is_some(),
            "goal status transition attempted"
        );
        Ok(updated)
    }

    /// Set `active_session_id` on a triggered goal (CAS guarded).
    ///
    /// Only succeeds when the goal's status is `triggered`. Returns `true` when
    /// the update matched, `false` otherwise. Used by the trigger flow after
    /// session creation + placement to link the session back to the goal.
    pub async fn set_active_session(
        &self,
        goal_id: bson::Uuid,
        session_id: bson::Uuid,
    ) -> Result<bool, crate::error::AppError> {
        let filter = doc! {
            "_id": goal_id,
            "status": bson::to_bson(&GoalStatus::Triggered).expect("GoalStatus serializes"),
        };
        let update = doc! {
            "$set": { "active_session_id": session_id }
        };

        let result = self
            .coll
            .update_one(filter, update)
            .await
            .map_err(|error| log_db_error("set_active_session", error))?;

        let matched = result.matched_count > 0;
        tracing::debug!(
            goal_id = %goal_id,
            session_id = %session_id,
            matched,
            "active_session_id set attempted"
        );
        Ok(matched)
    }
}
