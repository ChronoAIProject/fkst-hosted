//! MongoDB-backed repository for the `packages` collection.

use bson::doc;
use mongodb::Collection;
use serde::Deserialize;

use super::error::{is_duplicate_key, PackageError};
use super::model::{NewPackage, Package, PACKAGES_COLLECTION};

/// Projection target for [`PackageRepository::list`]: only `_id` is pulled
/// over the wire, never file content.
#[derive(Debug, Deserialize)]
struct IdOnly {
    #[serde(rename = "_id")]
    name: String,
}

/// Repository over the `packages` collection. Cheap to clone (the driver
/// `Collection` shares the underlying connection pool), so it can live in
/// Axum state and be shared across handlers.
#[derive(Clone)]
pub struct PackageRepository {
    collection: Collection<Package>,
}

/// Log an unexpected driver failure (server-side only; the driver text may
/// carry connection detail and never reaches a client) and wrap it.
fn log_db_error(op: &'static str, error: mongodb::error::Error) -> PackageError {
    tracing::error!(op, error = %error, "mongodb operation failed");
    PackageError::Db(error)
}

impl PackageRepository {
    /// Bind the repository to the `packages` collection of `db`.
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection::<Package>(PACKAGES_COLLECTION),
        }
    }

    /// Idempotent startup hook for `packages` indexes; safe to call on
    /// every boot.
    ///
    /// All lookups are by `_id` (the package name), which MongoDB indexes
    /// implicitly and uniquely (`_id_`), so nothing is created today. This
    /// hook exists so future secondary indexes have a home and the startup
    /// wiring stays symmetric across collections. A future index MUST use a
    /// stable name/spec (`createIndexes` is idempotent for identical specs)
    /// and MUST NOT attempt to recreate `_id_` — Mongo forbids declaring an
    /// explicit `_id` index and would error.
    pub async fn ensure_indexes(&self) -> Result<(), PackageError> {
        tracing::info!(collection = PACKAGES_COLLECTION, "packages indexes ensured");
        Ok(())
    }

    /// Validate and insert a new package; returns the stored document with
    /// `created_at == updated_at` populated from a single clock read.
    ///
    /// NOT an upsert: an existing name yields `PackageError::Duplicate`
    /// (conceptually 409). Concurrency is arbitrated solely by the Mongo
    /// `_id` uniqueness constraint — no read-then-write pre-check (TOCTOU).
    pub async fn create(&self, new_package: NewPackage) -> Result<Package, PackageError> {
        if let Err(reason) = new_package.validate() {
            // Reasons carry paths, sizes, and counts only — never content.
            tracing::warn!(name = %new_package.name, reason = %reason, "package validation rejected");
            return Err(PackageError::Validation(reason));
        }

        let now = bson::DateTime::now();
        let package = Package {
            name: new_package.name,
            files: new_package.files,
            composed_deps: new_package.composed_deps,
            created_at: now,
            updated_at: now,
        };

        if let Err(error) = self.collection.insert_one(&package).await {
            if is_duplicate_key(&error) {
                tracing::warn!(name = %package.name, "package already exists");
                return Err(PackageError::Duplicate(package.name));
            }
            return Err(log_db_error("create", error));
        }

        let total_content_bytes: usize = package.files.iter().map(|file| file.content.len()).sum();
        tracing::info!(
            name = %package.name,
            files = package.files.len(),
            total_content_bytes,
            "package created"
        );
        Ok(package)
    }

    /// Fetch a package by name. `Ok(None)` when absent (conceptually 404).
    pub async fn get(&self, name: &str) -> Result<Option<Package>, PackageError> {
        let found = self
            .collection
            .find_one(doc! { "_id": name })
            .await
            .map_err(|error| log_db_error("get", error))?;
        tracing::debug!(name, hit = found.is_some(), "package get");
        Ok(found)
    }

    /// List all package names, sorted ascending. Only `_id` is projected and
    /// deserialized ([`IdOnly`]); file content never crosses the wire.
    pub async fn list(&self) -> Result<Vec<String>, PackageError> {
        let mut cursor = self
            .collection
            .clone_with_type::<IdOnly>()
            .find(doc! {})
            .projection(doc! { "_id": 1 })
            .sort(doc! { "_id": 1 })
            .await
            .map_err(|error| log_db_error("list", error))?;

        let mut names = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|error| log_db_error("list", error))?
        {
            names.push(
                cursor
                    .deserialize_current()
                    .map_err(|error| log_db_error("list", error))?
                    .name,
            );
        }
        tracing::debug!(count = names.len(), "package list");
        Ok(names)
    }

    /// Cheap presence check by name (no document fetch, no deserialization).
    /// For read-only callers (e.g. the session subsystem) — NOT a create
    /// guard; `create` relies on `_id` uniqueness instead.
    pub async fn exists(&self, name: &str) -> Result<bool, PackageError> {
        let count = self
            .collection
            .count_documents(doc! { "_id": name })
            .await
            .map_err(|error| log_db_error("exists", error))?;
        tracing::debug!(name, exists = count > 0, "package exists check");
        Ok(count > 0)
    }
}
