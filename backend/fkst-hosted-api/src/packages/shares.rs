//! Package sharing: data models, validation, and MongoDB persistence for
//! the `package_shares` collection.
//!
//! A *share* grants read or use access to a package for an individual user
//! (GranteeKind::User) or an organization (GranteeKind::Org). Shares only
//! widen access -- they never grant write or manage. Duplicate grants are
//! arbitrated by the unique index on (package_name, grantee_kind, grantee_id).

use bson::doc;
use mongodb::options::IndexOptions;
use mongodb::{Collection, IndexModel};
use serde::{Deserialize, Serialize};

use super::error::is_duplicate_key;

/// MongoDB collection holding package share documents.
pub const PACKAGE_SHARES_COLLECTION: &str = "package_shares";

/// Who receives the share grant.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum GranteeKind {
    User,
    Org,
}

/// Access level for the share. `Use` supersedes `Read` (use implies read).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ShareLevel {
    Read,
    Use,
}

/// `package_shares` collection document.
///
/// Each document represents one grant of a specific package to a specific
/// grantee (user or org) at a given access level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ShareDoc {
    #[serde(rename = "_id")]
    pub id: bson::Uuid,
    pub package_name: String,
    pub grantee_kind: GranteeKind,
    /// Opaque NyxID user id or org id (not logins -- no rename drift).
    pub grantee_id: String,
    pub level: ShareLevel,
    /// NyxID user id of the grantor (audit only).
    pub granted_by: String,
    pub created_at: bson::DateTime,
}

/// Log an unexpected driver failure and wrap it.
fn log_db_error(op: &'static str, error: mongodb::error::Error) -> mongodb::error::Error {
    tracing::error!(op, error = %error, "mongodb operation failed");
    error
}

/// Repository over the `package_shares` collection. Cheap to clone (the driver
/// `Collection` shares the underlying connection pool).
#[derive(Clone)]
pub struct ShareRepo {
    collection: Collection<ShareDoc>,
}

impl ShareRepo {
    /// Bind the repository to the `package_shares` collection of `db`.
    pub fn new(db: &mongodb::Database) -> Self {
        Self {
            collection: db.collection::<ShareDoc>(PACKAGE_SHARES_COLLECTION),
        }
    }

    /// Idempotent startup hook for `package_shares` indexes; safe to call on
    /// every boot.
    ///
    /// Indexes:
    /// - `pkg_shares_unique_grantee`: unique on (package_name, grantee_kind,
    ///   grantee_id) -- duplicate grants arbitrated solely by this index.
    /// - `pkg_shares_grantee`: (grantee_kind, grantee_id) for shared-with-me
    ///   scans.
    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        let indexes = vec![
            IndexModel::builder()
                .keys(doc! {
                    "package_name": 1,
                    "grantee_kind": 1,
                    "grantee_id": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("pkg_shares_unique_grantee".to_string())
                        .unique(true)
                        .build(),
                )
                .build(),
            IndexModel::builder()
                .keys(doc! {
                    "grantee_kind": 1,
                    "grantee_id": 1,
                })
                .options(
                    IndexOptions::builder()
                        .name("pkg_shares_grantee".to_string())
                        .build(),
                )
                .build(),
        ];
        self.collection.create_indexes(indexes).await.map_err(|e| {
            tracing::error!(error = %e, "failed to create package_shares indexes");
            e
        })?;
        tracing::info!(
            collection = PACKAGE_SHARES_COLLECTION,
            "package_shares indexes ensured"
        );
        Ok(())
    }

    /// Insert a new share. Returns `Ok(ShareDoc)` on success,
    /// `Err(mongodb::error::Error)` on duplicate key (E11000) or other failures.
    /// The caller inspects the error to map duplicates to 409.
    pub async fn create(&self, share: ShareDoc) -> Result<ShareDoc, mongodb::error::Error> {
        if let Err(error) = self.collection.insert_one(&share).await {
            if is_duplicate_key(&error) {
                tracing::warn!(
                    package = %share.package_name,
                    grantee_kind = ?share.grantee_kind,
                    grantee_id = %share.grantee_id,
                    "share already exists"
                );
            } else {
                tracing::error!(
                    package = %share.package_name,
                    error = %error,
                    "share create failed"
                );
            }
            return Err(error);
        }
        tracing::info!(
            package = %share.package_name,
            grantee_kind = ?share.grantee_kind,
            grantee_id = %share.grantee_id,
            level = ?share.level,
            "share created"
        );
        Ok(share)
    }

    /// Check if a duplicate share exists (used for explicit 409 detection).
    /// Returns true when a document with the same (package_name, grantee_kind,
    /// grantee_id) already exists.
    pub async fn exists(
        &self,
        package_name: &str,
        grantee_kind: GranteeKind,
        grantee_id: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let filter = doc! {
            "package_name": package_name,
            "grantee_kind": bson::to_bson(&grantee_kind).expect("serialize grantee_kind"),
            "grantee_id": grantee_id,
        };
        let count = self
            .collection
            .count_documents(filter)
            .await
            .map_err(|e| log_db_error("share_exists", e))?;
        Ok(count > 0)
    }

    /// List all shares for a given package. Returns the documents sorted by
    /// `created_at` ascending.
    pub async fn list_for_package(
        &self,
        package_name: &str,
    ) -> Result<Vec<ShareDoc>, mongodb::error::Error> {
        let filter = doc! { "package_name": package_name };
        let mut cursor = self
            .collection
            .find(filter)
            .sort(doc! { "created_at": 1 })
            .await
            .map_err(|e| log_db_error("list_for_package", e))?;

        let mut shares = Vec::new();
        while cursor
            .advance()
            .await
            .map_err(|e| log_db_error("list_for_package", e))?
        {
            shares.push(
                cursor
                    .deserialize_current()
                    .map_err(|e| log_db_error("list_for_package", e))?,
            );
        }
        tracing::debug!(
            package = package_name,
            count = shares.len(),
            "shares listed for package"
        );
        Ok(shares)
    }

    /// Delete a single share by its `_id`. Returns `Ok(true)` when a document
    /// was deleted, `Ok(false)` when absent.
    pub async fn delete(
        &self,
        share_id: bson::Uuid,
        package_name: &str,
    ) -> Result<bool, mongodb::error::Error> {
        let filter = doc! {
            "_id": share_id,
            "package_name": package_name,
        };
        let result = self
            .collection
            .delete_one(filter)
            .await
            .map_err(|e| log_db_error("delete_share", e))?;
        let deleted = result.deleted_count > 0;
        if deleted {
            tracing::info!(share_id = %share_id, "share deleted");
        } else {
            tracing::debug!(share_id = %share_id, "share not found for delete");
        }
        Ok(deleted)
    }

    /// Delete all shares for a package (cascade on package delete). Best-effort;
    /// orphan rows are harmless because policy joins through the packages
    /// collection.
    pub async fn delete_for_package(
        &self,
        package_name: &str,
    ) -> Result<(), mongodb::error::Error> {
        let filter = doc! { "package_name": package_name };
        let result = self
            .collection
            .delete_many(filter)
            .await
            .map_err(|e| log_db_error("delete_for_package", e))?;
        tracing::info!(
            package = package_name,
            deleted = result.deleted_count,
            "shares cascade-deleted for package"
        );
        Ok(())
    }

    /// Return the distinct package names shared with a given user (direct user
    /// grant, or org grants for the user's org memberships). Deduped and sorted
    /// ascending.
    pub async fn shared_package_names(
        &self,
        user_id: &str,
        org_ids: &[String],
    ) -> Result<Vec<String>, mongodb::error::Error> {
        // Build $or: direct user grants + org grants.
        let mut or_branches = vec![doc! { "grantee_kind": "user", "grantee_id": user_id }];
        if !org_ids.is_empty() {
            or_branches.push(doc! {
                "grantee_kind": "org",
                "grantee_id": { "$in": org_ids }
            });
        }
        let filter = doc! { "$or": or_branches };

        let names = self
            .collection
            .distinct("package_name", filter)
            .await
            .map_err(|e| log_db_error("shared_package_names", e))?
            .into_iter()
            .filter_map(|bson| bson.as_str().map(|s| s.to_string()))
            .collect::<Vec<_>>();

        let mut names = names;
        names.sort();
        tracing::debug!(
            user = user_id,
            orgs = org_ids.len(),
            count = names.len(),
            "shared package names resolved"
        );
        Ok(names)
    }

    /// Check whether the given user has at least `Read` level access to the
    /// package via shares (either direct user grant at Read or Use level, or
    /// org grant at Read or Use level for any of the user's orgs).
    pub async fn has_read_share(
        &self,
        package_name: &str,
        user_id: &str,
        org_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        let mut or_branches = vec![doc! {
            "package_name": package_name,
            "grantee_kind": "user",
            "grantee_id": user_id,
        }];
        if !org_ids.is_empty() {
            or_branches.push(doc! {
                "package_name": package_name,
                "grantee_kind": "org",
                "grantee_id": { "$in": org_ids },
            });
        }
        let filter = doc! { "$or": or_branches };

        let count = self
            .collection
            .count_documents(filter)
            .await
            .map_err(|e| log_db_error("has_read_share", e))?;
        Ok(count > 0)
    }

    /// Check whether the given user has `Use` level access to the package via
    /// shares (either direct user grant at Use level, or org grant at Use
    /// level for any of the user's orgs). Use level implies read.
    pub async fn has_use_share(
        &self,
        package_name: &str,
        user_id: &str,
        org_ids: &[String],
    ) -> Result<bool, mongodb::error::Error> {
        let mut or_branches = vec![doc! {
            "package_name": package_name,
            "grantee_kind": "user",
            "grantee_id": user_id,
            "level": "use",
        }];
        if !org_ids.is_empty() {
            or_branches.push(doc! {
                "package_name": package_name,
                "grantee_kind": "org",
                "grantee_id": { "$in": org_ids },
                "level": "use",
            });
        }
        let filter = doc! { "$or": or_branches };

        let count = self
            .collection
            .count_documents(filter)
            .await
            .map_err(|e| log_db_error("has_use_share", e))?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod tests {
    use bson::{spec::BinarySubtype, Bson};

    use super::*;

    fn sample_share() -> ShareDoc {
        ShareDoc {
            id: bson::Uuid::new(),
            package_name: "demo-package".to_string(),
            grantee_kind: GranteeKind::User,
            grantee_id: "user-42".to_string(),
            level: ShareLevel::Read,
            granted_by: "owner-1".to_string(),
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
        }
    }

    #[test]
    fn share_doc_round_trips_and_id_is_binary_uuid() {
        let share = sample_share();
        let raw = bson::to_document(&share).expect("serialize");
        // _id must be Binary subtype 4 (UUID).
        match raw.get("_id").expect("_id present") {
            Bson::Binary(binary) => assert_eq!(binary.subtype, BinarySubtype::Uuid),
            other => panic!("expected Bson::Binary(subtype Uuid), got {other:?}"),
        }
        let back: ShareDoc = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, share);
    }

    #[test]
    fn grantee_kind_serializes_lowercase() {
        assert_eq!(
            bson::to_bson(&GranteeKind::User).expect("serialize"),
            Bson::String("user".to_string())
        );
        assert_eq!(
            bson::to_bson(&GranteeKind::Org).expect("serialize"),
            Bson::String("org".to_string())
        );
    }

    #[test]
    fn share_level_serializes_lowercase() {
        assert_eq!(
            bson::to_bson(&ShareLevel::Read).expect("serialize"),
            Bson::String("read".to_string())
        );
        assert_eq!(
            bson::to_bson(&ShareLevel::Use).expect("serialize"),
            Bson::String("use".to_string())
        );
    }

    #[test]
    fn share_doc_json_round_trips() {
        let share = sample_share();
        let json = serde_json::to_value(&share).expect("json serialize");
        assert_eq!(json["package_name"], "demo-package");
        assert_eq!(json["grantee_kind"], "user");
        assert_eq!(json["grantee_id"], "user-42");
        assert_eq!(json["level"], "read");
        assert_eq!(json["granted_by"], "owner-1");

        let back: ShareDoc = serde_json::from_value(json).expect("json deserialize");
        assert_eq!(back.package_name, share.package_name);
        assert_eq!(back.grantee_kind, share.grantee_kind);
        assert_eq!(back.grantee_id, share.grantee_id);
        assert_eq!(back.level, share.level);
        assert_eq!(back.granted_by, share.granted_by);
    }
}
