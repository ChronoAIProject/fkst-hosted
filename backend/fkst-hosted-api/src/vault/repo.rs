//! MongoDB-backed repository for the `vault_entries` collection.
//!
//! A cheap-to-clone handle over a typed `Collection`, an idempotent
//! `ensure_indexes` startup hook, and CRUD that maps driver failures onto
//! [`AppError`] (driver text is logged server-side only, never echoed to a
//! client).

use bson::{doc, Document};
use mongodb::options::ReturnDocument;
use mongodb::Collection;

use super::model::{EnvScopeRef, VaultEntry};
use crate::db::{unique_index_model, IDX_VAULT_OWNER_SCOPE_KEY, VAULT_ENTRIES};
use crate::error::AppError;

/// Repository over the `vault_entries` collection. Cheap to clone (the driver
/// `Collection` shares the connection pool), so it lives in `VaultService`
/// inside Axum state.
#[derive(Clone)]
pub struct VaultRepo {
    collection: Collection<VaultEntry>,
}

/// Log an unexpected driver failure (server-side only — the driver text may
/// carry connection detail and never reaches a client) and wrap it.
fn log_db_error(op: &'static str, error: mongodb::error::Error) -> AppError {
    tracing::error!(op, error = %error, "vault mongodb operation failed");
    AppError::Mongo(error)
}

impl VaultRepo {
    /// Bind the repository to the `vault_entries` collection of `db`.
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection::<VaultEntry>(VAULT_ENTRIES),
        }
    }

    /// Idempotent startup hook: create the UNIQUE index on
    /// `(owner_user_id, scope_key, key)` so a given env-var key is unique within
    /// an owner's scope (global vs each repo index distinctly via `scope_key`).
    /// Safe across restarts and concurrent pod starts (Mongo de-duplicates by
    /// name + spec).
    pub async fn ensure_indexes(&self) -> Result<(), AppError> {
        let index = unique_index_model(
            doc! { "owner_user_id": 1, "scope_key": 1, "key": 1 },
            IDX_VAULT_OWNER_SCOPE_KEY,
        );
        self.collection
            .create_indexes([index])
            .await
            .map_err(|error| log_db_error("ensure_indexes", error))?;
        tracing::info!(collection = VAULT_ENTRIES, "vault indexes ensured");
        Ok(())
    }

    /// Upsert an entry by its identity `(owner_user_id, scope_key, key)`.
    ///
    /// Insert-on-absent, replace-the-mutable-fields-on-present: ownership
    /// identity and `created_at`/`created_by` are set only on insert
    /// (`$setOnInsert`); `kind`, the value fields, `masked_hint`, `scope`, and
    /// `updated_at` are always `$set`. Returns the stored document. The unique
    /// index arbitrates concurrent first-inserts (no read-then-write TOCTOU).
    pub async fn upsert(&self, entry: VaultEntry) -> Result<VaultEntry, AppError> {
        let filter = doc! {
            "owner_user_id": &entry.owner_user_id,
            "scope_key": &entry.scope_key,
            "key": &entry.key,
        };
        // Serialize the value/scope fields through BSON so the typed enum/blob
        // shapes match what `find_one` later deserializes.
        let scope = bson::to_bson(&entry.scope).map_err(AppError::Bson)?;
        let kind = bson::to_bson(&entry.kind).map_err(AppError::Bson)?;
        let value_plain = bson::to_bson(&entry.value_plain).map_err(AppError::Bson)?;
        let value_enc = bson::to_bson(&entry.value_enc).map_err(AppError::Bson)?;
        let masked_hint = bson::to_bson(&entry.masked_hint).map_err(AppError::Bson)?;

        let update = doc! {
            "$set": {
                "scope": scope,
                "kind": kind,
                "value_plain": value_plain,
                "value_enc": value_enc,
                "masked_hint": masked_hint,
                "updated_at": entry.updated_at,
            },
            "$setOnInsert": {
                "_id": entry.id,
                "owner_user_id": &entry.owner_user_id,
                "org_id": bson::to_bson(&entry.org_id).map_err(AppError::Bson)?,
                "scope_key": &entry.scope_key,
                "key": &entry.key,
                "created_at": entry.created_at,
                "created_by": &entry.created_by,
            },
        };

        let stored = self
            .collection
            .find_one_and_update(filter, update)
            .upsert(true)
            .return_document(ReturnDocument::After)
            .await
            .map_err(|error| log_db_error("upsert", error))?;

        stored.ok_or_else(|| {
            // `return_document: After` with `upsert: true` always yields a
            // document; a None here is an unexpected driver invariant break.
            AppError::Internal(anyhow::anyhow!("vault upsert returned no document"))
        })
    }

    /// Fetch one entry by id, scoped to its owner so a caller can never read
    /// another owner's entry by guessing the UUID. `Ok(None)` when absent.
    pub async fn get_owned(
        &self,
        id: bson::Uuid,
        owner_user_id: &str,
    ) -> Result<Option<VaultEntry>, AppError> {
        self.collection
            .find_one(doc! { "_id": id, "owner_user_id": owner_user_id })
            .await
            .map_err(|error| log_db_error("get_owned", error))
    }

    /// Fetch one entry by id WITHOUT an owner filter, for an authz check that
    /// needs the entry's ownership fields before deciding (org case). `Ok(None)`
    /// when absent.
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<VaultEntry>, AppError> {
        self.collection
            .find_one(doc! { "_id": id })
            .await
            .map_err(|error| log_db_error("get", error))
    }

    /// List every entry an owner has in a given scope (by canonical
    /// `scope_key`), sorted by `key` ascending for deterministic output.
    pub async fn list_by_scope(
        &self,
        owner_user_id: &str,
        scope: &EnvScopeRef,
    ) -> Result<Vec<VaultEntry>, AppError> {
        let filter = doc! {
            "owner_user_id": owner_user_id,
            "scope_key": scope.scope_key(),
        };
        let mut cursor = self
            .collection
            .find(filter)
            .sort(doc! { "key": 1 })
            .await
            .map_err(|error| log_db_error("list_by_scope", error))?;
        let mut entries = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|error| log_db_error("list_by_scope", error))?
        {
            entries.push(
                cursor
                    .deserialize_current()
                    .map_err(|error| log_db_error("list_by_scope", error))?,
            );
        }
        Ok(entries)
    }

    /// Delete an entry by id, scoped to its owner. `Ok(true)` when a document
    /// was deleted, `Ok(false)` when absent (or owned by someone else).
    pub async fn delete_owned(
        &self,
        id: bson::Uuid,
        owner_user_id: &str,
    ) -> Result<bool, AppError> {
        let result = self
            .collection
            .delete_one(doc! { "_id": id, "owner_user_id": owner_user_id })
            .await
            .map_err(|error| log_db_error("delete_owned", error))?;
        Ok(result.deleted_count > 0)
    }

    /// Count an owner's entries in a scope (cap enforcement before an insert).
    /// Raw-document count avoids deserializing every entry.
    pub async fn count_in_scope(
        &self,
        owner_user_id: &str,
        scope: &EnvScopeRef,
    ) -> Result<u64, AppError> {
        let filter = doc! {
            "owner_user_id": owner_user_id,
            "scope_key": scope.scope_key(),
        };
        self.collection
            .clone_with_type::<Document>()
            .count_documents(filter)
            .await
            .map_err(|error| log_db_error("count_in_scope", error))
    }
}
