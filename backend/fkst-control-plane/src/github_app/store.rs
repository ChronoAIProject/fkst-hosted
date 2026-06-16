//! Mongo-backed GitHub App installation persistence (issue #108).
//!
//! [`MongoInstallationStore`] is the production implementation of two seams:
//!   - the narrow [`InstallationStore`] trait the token service depends on
//!     (resolve before the GitHub probe, evict on uninstall) — so the service
//!     never touches MongoDB directly; and
//!   - the richer write path the webhook handler drives (upsert / set_repos /
//!     set_suspended / delete from `installation` and
//!     `installation_repositories` events).
//!
//! The collection accessor and indexes live in `crate::db` (which owns Mongo
//! collection wiring); this module owns the installation-specific logic so it
//! stays cohesive and small.

use mongodb::Collection;

use super::api::{InstallationStore, StoredInstallation};
use super::{GithubAppError, InstallationId};
use crate::db::Db;
use crate::error::AppError;
use crate::models::{AccountType, GithubInstallationDoc, RepositorySelection, COVERS_ALL};

/// Mongo-backed persistence for GitHub App installations (issue #108). Cheap to
/// clone (the `Collection` shares the connection pool).
#[derive(Clone)]
pub struct MongoInstallationStore {
    collection: Collection<GithubInstallationDoc>,
}

/// Log an unexpected driver failure (server-side only — the driver text may
/// carry connection detail and never reaches a client) and wrap it.
fn log_install_db_error(op: &'static str, error: mongodb::error::Error) -> AppError {
    tracing::error!(op, error = %error, "github installation store mongodb operation failed");
    AppError::Mongo(error)
}

impl MongoInstallationStore {
    /// Bind the store to the `github_installations` collection of `db`.
    pub fn new(db: &Db) -> Self {
        Self {
            collection: db.github_installations(),
        }
    }

    /// Upsert (replace) the installation record keyed by `installation_id`. A
    /// full replace makes webhook redeliveries idempotent: the latest event
    /// wins, and the record always reflects the event's coverage/suspension.
    pub async fn upsert(&self, doc: &GithubInstallationDoc) -> Result<(), AppError> {
        self.collection
            .replace_one(bson::doc! { "_id": doc.installation_id }, doc)
            .upsert(true)
            .await
            .map_err(|error| log_install_db_error("upsert", error))?;
        tracing::info!(
            installation_id = doc.installation_id,
            account_login = %doc.account_login,
            selection = ?doc.repository_selection,
            repo_count = doc.repos.len(),
            suspended = doc.suspended,
            "github installation record upserted"
        );
        Ok(())
    }

    /// Set the suspended flag on an installation (the `suspend` / `unsuspend`
    /// events). `Ok(false)` when no such installation is persisted (a redelivery
    /// for an install we never recorded — benign).
    pub async fn set_suspended(
        &self,
        installation_id: i64,
        suspended: bool,
    ) -> Result<bool, AppError> {
        let result = self
            .collection
            .update_one(
                bson::doc! { "_id": installation_id },
                bson::doc! { "$set": { "suspended": suspended, "updated_at": bson::DateTime::now() } },
            )
            .await
            .map_err(|error| log_install_db_error("set_suspended", error))?;
        Ok(result.matched_count > 0)
    }

    /// Replace the covered `repos` of an installation (the
    /// `installation_repositories` added/removed events). `Ok(false)` when the
    /// installation is not persisted yet (the caller upserts a full record
    /// instead). Repos are stored canonical (lowercase `owner/name`).
    pub async fn set_repos(
        &self,
        installation_id: i64,
        repos: &[String],
    ) -> Result<bool, AppError> {
        let result = self
            .collection
            .update_one(
                bson::doc! { "_id": installation_id },
                bson::doc! { "$set": {
                    "repos": repos,
                    "updated_at": bson::DateTime::now(),
                } },
            )
            .await
            .map_err(|error| log_install_db_error("set_repos", error))?;
        Ok(result.matched_count > 0)
    }

    /// Remove the installation record entirely (the `installation` `deleted`
    /// event). Idempotent: `Ok(0)` when nothing was stored.
    pub async fn delete(&self, installation_id: i64) -> Result<u64, AppError> {
        let result = self
            .collection
            .delete_one(bson::doc! { "_id": installation_id })
            .await
            .map_err(|error| log_install_db_error("delete", error))?;
        if result.deleted_count > 0 {
            tracing::info!(installation_id, "github installation record removed");
        }
        Ok(result.deleted_count)
    }

    /// Fetch one installation record by id (used by the webhook handler to
    /// reconcile a coverage change against a known account).
    pub async fn get(
        &self,
        installation_id: i64,
    ) -> Result<Option<GithubInstallationDoc>, AppError> {
        self.collection
            .find_one(bson::doc! { "_id": installation_id })
            .await
            .map_err(|error| log_install_db_error("get", error))
    }

    /// Resolve the installation covering `owner/repo`: first a direct
    /// (`selected`) match on the canonical repo full-name, then an account-wide
    /// (`all`) match by owner login. Suspended installs are filtered out at the
    /// `covers` check so a suspended account never resolves. `Ok(None)` => not
    /// known (the token service falls back to the on-demand GitHub probe).
    pub async fn lookup_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Option<GithubInstallationDoc>, AppError> {
        let canonical = GithubInstallationDoc::canonical_repo(owner, repo);
        // Either a selected-install that lists this repo, or an account-wide
        // install of this owner. One query covers both via `$or`.
        let filter = bson::doc! {
            "$or": [
                { "repos": &canonical },
                { "account_login": owner.to_lowercase(), "repos": COVERS_ALL },
            ]
        };
        let mut cursor = self
            .collection
            .find(filter)
            .await
            .map_err(|error| log_install_db_error("lookup_repo", error))?;
        while cursor
            .advance()
            .await
            .map_err(|error| log_install_db_error("lookup_repo", error))?
        {
            let doc = cursor
                .deserialize_current()
                .map_err(|error| log_install_db_error("lookup_repo", error))?;
            // `covers` enforces account-scoping for `all` and excludes suspended
            // installs; the query is a coarse prefilter only.
            if doc.covers(owner, repo) {
                return Ok(Some(doc));
            }
        }
        Ok(None)
    }
}

/// Bridge the Mongo store to the token service's persistence seam (issue #108).
/// The trait surface is deliberately narrow (lookup + remember + forget) so the
/// token service depends only on the abstraction; the rich write path stays on
/// the inherent impl for the webhook handler. A store failure maps to
/// [`GithubAppError::Http`] — it is the store's own failure, surfaced (not
/// swallowed) so the caller can fall back deliberately.
#[async_trait::async_trait]
impl InstallationStore for MongoInstallationStore {
    async fn lookup_repo(
        &self,
        owner: &str,
        repo: &str,
    ) -> Result<Option<StoredInstallation>, GithubAppError> {
        let found = MongoInstallationStore::lookup_repo(self, owner, repo)
            .await
            .map_err(|e| GithubAppError::Http(format!("installation store: {e}")))?;
        Ok(found.map(|doc| StoredInstallation {
            id: InstallationId(doc.installation_id as u64),
            is_organization: matches!(doc.account_type, AccountType::Organization),
            suspended: doc.suspended,
        }))
    }

    async fn remember_repo(
        &self,
        owner: &str,
        repo: &str,
        installation_id: InstallationId,
    ) -> Result<(), GithubAppError> {
        let canonical = GithubInstallationDoc::canonical_repo(owner, repo);
        let id = installation_id.0 as i64;
        // `$addToSet` records the repo without clobbering an authoritative
        // webhook record; `$setOnInsert` only fills the descriptive fields when
        // we are creating the record from a probe (so a later webhook with the
        // correct account_type / `all` selection replaces them wholesale via
        // `upsert`/`set_repos`). account_type defaults to User here: an on-demand
        // probe cannot know the account kind, and this field is consumed only by
        // the token service's StoredInstallation, never by the owner-gated
        // install hint (which does its own richer lookup).
        self.collection
            .update_one(
                bson::doc! { "_id": id },
                bson::doc! {
                    "$addToSet": { "repos": &canonical },
                    "$set": { "updated_at": bson::DateTime::now() },
                    "$setOnInsert": {
                        "account_login": owner.to_lowercase(),
                        "account_type": bson::to_bson(&AccountType::User)
                            .expect("AccountType serializes"),
                        "repository_selection": bson::to_bson(&RepositorySelection::Selected)
                            .expect("RepositorySelection serializes"),
                        "suspended": false,
                    },
                },
            )
            .upsert(true)
            .await
            .map_err(|e| GithubAppError::Http(format!("installation store remember: {e}")))?;
        Ok(())
    }

    async fn forget_repo(&self, owner: &str, repo: &str) -> Result<(), GithubAppError> {
        // Pull the canonical repo from any installation's `repos` array. An
        // `all` install does not list concrete repos, so this only affects
        // `selected` records; an uninstall of an `all` install is handled by
        // the `delete` path from the webhook handler.
        let canonical = GithubInstallationDoc::canonical_repo(owner, repo);
        self.collection
            .update_many(
                bson::doc! { "repos": &canonical },
                bson::doc! {
                    "$pull": { "repos": &canonical },
                    "$set": { "updated_at": bson::DateTime::now() },
                },
            )
            .await
            .map_err(|e| GithubAppError::Http(format!("installation store forget: {e}")))?;
        Ok(())
    }
}
