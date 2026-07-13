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
//! Today the sole implementation is the Kubernetes ConfigMap/Secret pair
//! ([`crate::k8s::env_store::EnvStore`]); swapping to another backend (a database,
//! or any other store) is a new `impl EnvironmentProfileStore` plus a one-line
//! change in [`default_store`] — no route or spawn-path change.
//!
//! Secret-value discipline is part of the contract: every reader except
//! [`EnvironmentProfileStore::load_environment_for_session`] exposes secret KEY
//! NAMES only, so a value never crosses the API boundary. An implementation MUST
//! preserve that (the K8s impl does).

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;

use crate::error::AppError;
use crate::k8s::env_store::{EnvRecord, EnvStore, EnvSummary};
use crate::k8s::KubeError;

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
    /// `ready` (the K8s impl writes the secret first, the ready-marked config
    /// last).
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
    ) -> Result<(), AppError>;

    /// One profile's public view (install + variables + status + secret KEY NAMES).
    /// `None` when absent. MUST NOT read secret values.
    async fn get_environment(&self, id: i64, name: &str) -> Result<Option<EnvRecord>, AppError>;

    /// The owner's profiles as compact summaries (counts only), stably ordered.
    async fn list_environments(&self, id: i64) -> Result<Vec<EnvSummary>, AppError>;

    /// Count the owner's profiles (for the per-user cap).
    async fn count_environments(&self, id: i64) -> Result<usize, AppError>;

    /// Delete one profile. Idempotent; returns whether anything existed.
    async fn delete_environment(&self, id: i64, name: &str) -> Result<bool, AppError>;

    /// SERVER-SIDE ONLY: resolve one profile into `(install, merged_env)` for the
    /// session launcher — the ONLY method that reads secret VALUES. Never wired to
    /// a user-facing route. `None` when absent.
    async fn load_environment_for_session(
        &self,
        id: i64,
        name: &str,
    ) -> Result<Option<(Vec<String>, BTreeMap<String, String>)>, AppError>;
}

/// Build the configured environment-profile store — THE single place the concrete
/// backend is selected. Today: the Kubernetes ConfigMap/Secret store bound to the
/// control-plane `namespace`. A future backend is added here (e.g. switch on a
/// `FKST_ENV_PROFILE_BACKEND` knob) without touching any caller.
pub async fn default_store(namespace: &str) -> Result<Arc<dyn EnvironmentProfileStore>, KubeError> {
    Ok(Arc::new(EnvStore::from_inferred(namespace).await?))
}
