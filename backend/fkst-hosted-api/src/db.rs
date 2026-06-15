//! MongoDB handle: typed collection accessors, startup ping, and idempotent
//! index creation.

use std::time::Duration;

use bson::doc;
use mongodb::options::{ClientOptions, IndexOptions};
use mongodb::{Client, Collection, IndexModel};

use crate::config::Config;
use crate::error::AppError;
use crate::models::{GithubInstallationDoc, LeaseDoc, SessionDoc};

/// Collection names (single source of truth). The `packages` collection name
/// is owned by `crate::packages::PACKAGES_COLLECTION`.
pub const SESSIONS: &str = "sessions";
pub const LEASES: &str = "leases";
/// The per-session secret/variable vault collection (issue #100).
pub const VAULT_ENTRIES: &str = "vault_entries";
/// GitHub App installation records (issue #108). One document per installation,
/// `_id` = the GitHub installation id.
pub const GITHUB_INSTALLATIONS: &str = "github_installations";

/// Stable index names (deterministic idempotency; asserted by integration
/// tests). No index is declared for `leases._id` — the implicit `_id` index
/// already enforces uniqueness. (`packages` indexes are owned by
/// `crate::packages`.)
pub const IDX_SESSIONS_PACKAGE_NAME: &str = "sessions_package_name";
pub const IDX_SESSIONS_STATUS: &str = "sessions_status";
pub const IDX_SESSIONS_POD_ID: &str = "sessions_pod_id";
pub const IDX_SESSIONS_OWNER_USER_ID: &str = "sessions_owner_user_id";
pub const IDX_SESSIONS_ORG_ID: &str = "sessions_org_id";
pub const IDX_SESSIONS_GOAL_ID: &str = "sessions_goal_id";
pub const IDX_LEASES_EXPIRES_AT: &str = "leases_expires_at";
/// Unique vault index over `(owner_user_id, scope_key, key)` — one entry per
/// key within an owner's scope (issue #100). Owned by `crate::vault::VaultRepo`.
pub const IDX_VAULT_OWNER_SCOPE_KEY: &str = "vault_owner_scope_key_unique";
/// Index over `github_installations.repos` (issue #108): resolve which
/// installation covers a `owner/name` repo by membership in the array.
pub const IDX_GH_INSTALL_REPOS: &str = "github_installations_repos";
/// Index over `github_installations.account_login` (issue #108): resolve an
/// account-wide (`all`) installation by owner login.
pub const IDX_GH_INSTALL_ACCOUNT: &str = "github_installations_account_login";

/// Cheap-to-clone handle to the Mongo database (`mongodb::Database` is
/// `Arc`-backed internally).
#[derive(Clone)]
pub struct Db {
    pub database: mongodb::Database,
}

/// Redact the userinfo (credentials) segment of a MongoDB URI for logging.
///
/// `mongodb://user:secret@host:27017` -> `mongodb://<redacted>@host:27017`;
/// a URI without userinfo is returned unchanged.
///
/// Splits at the LAST `@` so even a malformed URI with an unescaped `@`
/// inside the password (`mongodb://user:p@ss@host:27017`) cannot leak a
/// password tail into the redacted log line.
pub fn redact_mongodb_uri(uri: &str) -> String {
    match uri.rsplit_once('@') {
        Some((before_at, rest)) => match before_at.split_once("://") {
            Some((scheme, _userinfo)) => format!("{scheme}://<redacted>@{rest}"),
            None => format!("<redacted>@{rest}"),
        },
        None => uri.to_string(),
    }
}

/// Build a non-unique ascending index with a stable name.
fn index_model(keys: bson::Document, name: &str) -> IndexModel {
    IndexModel::builder()
        .keys(keys)
        .options(IndexOptions::builder().name(name.to_string()).build())
        .build()
}

/// Build a UNIQUE ascending index with a stable name. The existing
/// [`index_model`] builds non-unique indexes; the vault needs uniqueness on
/// `(owner_user_id, scope_key, key)` to enforce one entry per key per scope
/// (issue #100), so this is its own helper rather than a flag on the other.
pub fn unique_index_model(keys: bson::Document, name: &str) -> IndexModel {
    IndexModel::builder()
        .keys(keys)
        .options(
            IndexOptions::builder()
                .name(name.to_string())
                .unique(true)
                .build(),
        )
        .build()
}

impl Db {
    /// Build the handle lazily (no I/O beyond URI parsing; the driver
    /// connects on first operation). The server-selection timeout bounds
    /// every subsequent operation against an unreachable server.
    pub async fn from_config(cfg: &Config) -> Result<Db, AppError> {
        let mut options = ClientOptions::parse(&cfg.mongodb_uri)
            .await
            .map_err(AppError::Mongo)?;
        options.server_selection_timeout = Some(Duration::from_millis(
            cfg.mongodb_server_selection_timeout_ms,
        ));
        let client = Client::with_options(options).map_err(AppError::Mongo)?;
        Ok(Db {
            database: client.database(&cfg.mongodb_db),
        })
    }

    /// Build the handle and prove connectivity with a ping. Errors are
    /// fail-closed at startup (the caller exits non-zero).
    pub async fn connect(cfg: &Config) -> Result<Db, AppError> {
        let db = Self::from_config(cfg).await?;
        db.ping().await.map_err(AppError::Mongo)?;
        Ok(db)
    }

    /// Ping the server. Used by `/health` and startup; bounded by the
    /// server-selection timeout.
    pub async fn ping(&self) -> Result<(), mongodb::error::Error> {
        self.database.run_command(doc! { "ping": 1 }).await?;
        Ok(())
    }

    /// Typed collection accessor.
    pub fn collection<T: Send + Sync>(&self, name: &str) -> Collection<T> {
        self.database.collection::<T>(name)
    }

    /// The `sessions` collection.
    pub fn sessions(&self) -> Collection<SessionDoc> {
        self.collection(SESSIONS)
    }

    /// The `leases` collection.
    pub fn leases(&self) -> Collection<LeaseDoc> {
        self.collection(LEASES)
    }

    /// The `vault_entries` collection (issue #100). The typed
    /// `Collection<VaultEntry>` accessor lives on `crate::vault::VaultRepo`;
    /// this raw-document accessor exists so the collection name has a single
    /// home alongside the other collections and integration tests can inspect
    /// the stored BSON shape.
    pub fn vault_entries(&self) -> Collection<bson::Document> {
        self.collection(VAULT_ENTRIES)
    }

    /// The `github_installations` collection (issue #108), typed to
    /// [`GithubInstallationDoc`]. The lifecycle logic lives on
    /// [`crate::github_app::MongoInstallationStore`]; this accessor gives the
    /// collection a single home alongside the others and lets integration tests
    /// inspect the stored BSON shape.
    pub fn github_installations(&self) -> Collection<GithubInstallationDoc> {
        self.collection(GITHUB_INSTALLATIONS)
    }

    /// Idempotently create all secondary indexes (stable names). Safe across
    /// restarts and concurrent pod starts: MongoDB de-duplicates by index
    /// name + spec. Never drops or alters existing indexes.
    pub async fn ensure_indexes(&self) -> Result<(), mongodb::error::Error> {
        let session_indexes = [
            (doc! { "package_name": 1 }, IDX_SESSIONS_PACKAGE_NAME),
            (doc! { "status": 1 }, IDX_SESSIONS_STATUS),
            (doc! { "pod_id": 1 }, IDX_SESSIONS_POD_ID),
            (doc! { "owner_user_id": 1 }, IDX_SESSIONS_OWNER_USER_ID),
            (doc! { "org_id": 1 }, IDX_SESSIONS_ORG_ID),
            (doc! { "goal_id": 1 }, IDX_SESSIONS_GOAL_ID),
        ];
        self.sessions()
            .create_indexes(
                session_indexes
                    .iter()
                    .map(|(keys, name)| index_model(keys.clone(), name)),
            )
            .await?;
        for (_, name) in &session_indexes {
            tracing::debug!(collection = SESSIONS, index = name, "index ensured");
        }

        let lease_indexes = [(doc! { "expires_at": 1 }, IDX_LEASES_EXPIRES_AT)];
        self.leases()
            .create_indexes(
                lease_indexes
                    .iter()
                    .map(|(keys, name)| index_model(keys.clone(), name)),
            )
            .await?;
        for (_, name) in &lease_indexes {
            tracing::debug!(collection = LEASES, index = name, "index ensured");
        }

        // GitHub App installations (issue #108): `repos` membership answers
        // selected-install coverage; `account_login` answers account-wide
        // (`all`) coverage. `_id` (the installation id) is implicitly unique.
        let install_indexes = [
            (doc! { "repos": 1 }, IDX_GH_INSTALL_REPOS),
            (doc! { "account_login": 1 }, IDX_GH_INSTALL_ACCOUNT),
        ];
        self.github_installations()
            .create_indexes(
                install_indexes
                    .iter()
                    .map(|(keys, name)| index_model(keys.clone(), name)),
            )
            .await?;
        for (_, name) in &install_indexes {
            tracing::debug!(
                collection = GITHUB_INSTALLATIONS,
                index = name,
                "index ensured"
            );
        }

        // No INFO here: the single "indexes ensured" lifecycle line is
        // emitted by the caller (main.rs) so it appears exactly once.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redaction_strips_credentials() {
        let redacted = redact_mongodb_uri("mongodb://user:secret@host:27017");
        assert!(!redacted.contains("secret"), "password leaked: {redacted}");
        assert!(!redacted.contains("user"), "username leaked: {redacted}");
        assert_eq!(redacted, "mongodb://<redacted>@host:27017");
    }

    #[test]
    fn redaction_splits_at_the_last_at_sign() {
        // Malformed but real-world: unescaped '@' inside the password. A
        // first-'@' split would leak "ss@host:27017" as the redacted tail.
        let redacted = redact_mongodb_uri("mongodb://user:p@ss@host:27017");
        assert_eq!(redacted, "mongodb://<redacted>@host:27017");
        assert!(!redacted.contains("p@"), "password head leaked: {redacted}");
        assert!(!redacted.contains("ss"), "password tail leaked: {redacted}");
        assert!(!redacted.contains("user"), "username leaked: {redacted}");
    }

    #[test]
    fn redaction_leaves_credential_free_uris_unchanged() {
        assert_eq!(
            redact_mongodb_uri("mongodb://localhost:27017"),
            "mongodb://localhost:27017"
        );
    }

    #[test]
    fn redaction_handles_srv_uris() {
        assert_eq!(
            redact_mongodb_uri("mongodb+srv://app:hunter2@cluster.example.com"),
            "mongodb+srv://<redacted>@cluster.example.com"
        );
    }
}
