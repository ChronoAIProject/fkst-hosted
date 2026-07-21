//! The **Environment Profiles** storage interface (issue #465).
//!
//! An *environment profile* is a user-owned, named bundle of ordered install
//! commands, non-secret variables, and write-only secrets that is validated once
//! and later materialized into a session's runtime. This module owns the
//! backend-agnostic seam — [`EnvironmentProfileStore`] — that the REST routes
//! ([`crate::routes::environments`]) and the reconciler
//! ([`crate::reconcile::execute`]) depend on. Neither ever names a concrete
//! storage type.
//!
//! The default remains the legacy Kubernetes ConfigMap/Secret pair
//! ([`crate::k8s::env_store::EnvStore`]). Setting
//! `FKST_ENV_STORE_NAMESPACE` selects the namespace-independent, encrypted
//! [`crate::k8s::durable_env_store::DurableEnvStore`] without changing routes or
//! session materialization.
//!
//! Secret-value discipline is part of the contract: every reader except
//! [`EnvironmentProfileStore::load_environment_for_session`] exposes secret KEY
//! NAMES only, so a value never crosses the API boundary. An implementation MUST
//! preserve that (the K8s impl does).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::env_config::EnvConfig;
use crate::error::AppError;
use crate::k8s::durable_env_store::DurableEnvStore;
use crate::k8s::env_store::{EnvRecord, EnvStore, EnvSummary};

/// The pluggable storage backend for environment profiles. All methods are keyed
/// by the owner's immutable numeric GitHub id (`id`) plus the profile `name`.
///
/// Object-safe on purpose: callers hold `Arc<dyn EnvironmentProfileStore>` so the
/// concrete backend is chosen once (see [`default_store`]) and never leaks into
/// the request/spawn paths.
#[async_trait]
pub trait EnvironmentProfileStore: Send + Sync {
    /// Create-or-replace one profile: its ordered install commands, non-secret
    /// variables, and secret VALUES, with the validation metadata. A backend MUST
    /// make the write atomic enough that a partial write is never observed as
    /// `ready`. `expected_version` is the opaque token returned by the preceding
    /// read; a stale replace must surface a retriable conflict.
    #[allow(clippy::too_many_arguments)]
    async fn put_environment(
        &self,
        id: i64,
        login: &str,
        name: &str,
        install: &[String],
        variables: &BTreeMap<String, String>,
        secrets: &BTreeMap<String, String>,
        validated_at: &str,
        content_hash: &str,
        validation_image: &str,
        expected_version: Option<&str>,
    ) -> Result<(), AppError>;

    /// One profile's public view (install + variables + status + secret KEY NAMES).
    /// `None` when absent. MUST NOT return secret values.
    async fn get_environment(&self, id: i64, name: &str) -> Result<Option<EnvRecord>, AppError>;

    /// The owner's profiles as compact summaries (counts only), stably ordered.
    async fn list_environments(&self, id: i64) -> Result<Vec<EnvSummary>, AppError>;

    /// Count the owner's profiles (for the per-user cap).
    async fn count_environments(&self, id: i64) -> Result<usize, AppError>;

    /// Delete one profile. Idempotent; returns whether anything existed.
    async fn delete_environment(&self, id: i64, name: &str) -> Result<bool, AppError>;

    /// SERVER-SIDE ONLY: resolve one profile into `(install, merged_env, secret_keys)`
    /// for the session launcher — the ONLY method that reads secret VALUES.
    /// `secret_keys` are the NAMES of the env vars whose values are secrets (so the
    /// launcher can inject them but keep them out of the codex config). Never wired
    /// to a user-facing route. `None` when absent.
    async fn load_environment_for_session(
        &self,
        id: i64,
        name: &str,
    ) -> Result<Option<(Vec<String>, BTreeMap<String, String>, Vec<String>)>, AppError>;
}

/// Build the configured environment-profile store. REST calls use this helper;
/// startup separately initializes and migrates the same durable configuration
/// before the router begins serving.
pub async fn default_store(
    config: &EnvConfig,
    legacy_namespace: &str,
) -> Result<Arc<dyn EnvironmentProfileStore>, AppError> {
    match (&config.store_namespace, &config.store_encryption_key) {
        (Some(namespace), Some(key)) => Ok(Arc::new(
            DurableEnvStore::from_inferred(namespace, key).await?,
        )),
        (None, None) => EnvStore::from_inferred(legacy_namespace)
            .await
            .map(|store| Arc::new(store) as Arc<dyn EnvironmentProfileStore>)
            .map_err(|error| {
                tracing::error!(error = %error, "env store: kubernetes client unavailable");
                AppError::Unavailable("environment store backend unavailable".to_string())
            }),
        _ => Err(AppError::Config(
            "durable environment store configuration is incomplete".to_string(),
        )),
    }
}

/// Build and initialize the durable store before reconciliation or HTTP serving.
/// `None` preserves the legacy lazy store when the durable backend is unconfigured.
pub async fn initialize_configured_store(
    config: &EnvConfig,
) -> Result<Option<Arc<dyn EnvironmentProfileStore>>, AppError> {
    let (Some(namespace), Some(key)) = (
        config.store_namespace.as_deref(),
        config.store_encryption_key.as_ref(),
    ) else {
        if config.store_namespace.is_some() || config.store_encryption_key.is_some() {
            return Err(AppError::Config(
                "durable environment store configuration is incomplete".to_string(),
            ));
        }
        return Ok(None);
    };
    let store = DurableEnvStore::from_inferred(namespace, key).await?;
    store
        .initialize(config.store_legacy_namespace.as_deref())
        .await?;
    Ok(Some(Arc::new(store)))
}
