//! Namespace-independent encrypted environment-profile store.
//!
//! Each profile is one Kubernetes Secret in an operator-selected namespace
//! outside the application namespace. The Secret contains only an AES-GCM nonce
//! and ciphertext; one successful create/replace is therefore the atomic commit
//! point for public configuration and secret values together.

use std::collections::BTreeMap;
use std::sync::Arc;

use async_trait::async_trait;
use secrecy::SecretString;

use crate::environment_profile::EnvironmentProfileStore;
use crate::error::AppError;
use crate::k8s::env_store::{env_object_name, EnvRecord, EnvSummary};

mod api;
mod crypto;
mod migration;
mod record;

use api::{ApiFailure, EnvironmentKubeApi, KubernetesEnvironmentApi};
use crypto::ProfileCipher;
use record::{
    envelope_from_secret, environment_selector, identity_from_secret, now_rfc3339,
    secret_from_envelope, ProfileEnvelope, RecordError,
};

#[derive(Clone)]
pub struct DurableEnvStore {
    api: Arc<dyn EnvironmentKubeApi>,
    namespace: String,
    cipher: ProfileCipher,
}

impl std::fmt::Debug for DurableEnvStore {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DurableEnvStore")
            .field("namespace", &self.namespace)
            .field("cipher", &self.cipher)
            .finish_non_exhaustive()
    }
}

impl DurableEnvStore {
    pub async fn from_inferred(
        namespace: impl Into<String>,
        encoded_key: &SecretString,
    ) -> Result<Self, AppError> {
        let client = kube::Client::try_default().await.map_err(|error| {
            tracing::error!(error = %error, "durable env store: kubernetes client unavailable");
            AppError::Unavailable("environment store backend unavailable".to_string())
        })?;
        Self::from_client(client, namespace, encoded_key)
    }

    pub fn from_client(
        client: kube::Client,
        namespace: impl Into<String>,
        encoded_key: &SecretString,
    ) -> Result<Self, AppError> {
        Ok(Self {
            api: Arc::new(KubernetesEnvironmentApi::new(client)),
            namespace: namespace.into(),
            cipher: ProfileCipher::from_base64(encoded_key)?,
        })
    }

    #[cfg(test)]
    fn from_api(
        api: Arc<dyn EnvironmentKubeApi>,
        namespace: impl Into<String>,
        encoded_key: &SecretString,
    ) -> Result<Self, AppError> {
        Ok(Self {
            api,
            namespace: namespace.into(),
            cipher: ProfileCipher::from_base64(encoded_key)?,
        })
    }

    /// Prove connectivity, validate every durable record with the configured
    /// key, optionally migrate the complete legacy set, then validate again.
    pub async fn initialize(&self, legacy_namespace: Option<&str>) -> Result<(), AppError> {
        self.verify_all_records().await?;
        if let Some(namespace) = legacy_namespace {
            let migrated = self.migrate_legacy(namespace).await?;
            tracing::info!(migrated, "durable env store: legacy migration converged");
        }
        self.verify_all_records().await?;
        tracing::info!(
            namespace = %self.namespace,
            "durable env store: connectivity and record integrity verified"
        );
        Ok(())
    }

    async fn verify_all_records(&self) -> Result<(), AppError> {
        let secrets = self
            .api
            .list_secrets(&self.namespace, &environment_selector())
            .await
            .map_err(|error| self.map_api_failure("list durable records", error))?;
        for secret in &secrets {
            let (id, name) = identity_from_secret(secret)
                .map_err(|error| self.map_record_failure(None, None, error))?;
            self.decode_secret(secret, id, &name)?;
        }
        Ok(())
    }

    async fn load_envelope(
        &self,
        id: i64,
        name: &str,
    ) -> Result<Option<(ProfileEnvelope, String)>, AppError> {
        let object = env_object_name(id, name);
        let Some(secret) = self
            .api
            .get_secret(&self.namespace, &object)
            .await
            .map_err(|error| self.map_api_failure("get durable record", error))?
        else {
            return Ok(None);
        };
        let version =
            secret.metadata.resource_version.clone().ok_or_else(|| {
                self.map_record_failure(Some(id), Some(name), RecordError::Metadata)
            })?;
        let record = self.decode_secret(&secret, id, name)?;
        Ok(Some((record, version)))
    }

    fn decode_secret(
        &self,
        secret: &k8s_openapi::api::core::v1::Secret,
        id: i64,
        name: &str,
    ) -> Result<ProfileEnvelope, AppError> {
        envelope_from_secret(secret, id, name, &self.cipher)
            .map_err(|error| self.map_record_failure(Some(id), Some(name), error))
    }

    async fn create_envelope(&self, record: &ProfileEnvelope) -> Result<(), AppError> {
        let secret = secret_from_envelope(record, &self.cipher, None).map_err(|error| {
            self.map_record_failure(Some(record.github_user_id), Some(&record.name), error)
        })?;
        self.api
            .create_secret(&self.namespace, &secret)
            .await
            .map_err(|error| self.map_api_failure("create durable record", error))
    }

    fn map_api_failure(&self, operation: &'static str, error: ApiFailure) -> AppError {
        match error {
            ApiFailure::Conflict => {
                AppError::Conflict("env store: a concurrent update won; please retry".to_string())
            }
            ApiFailure::Other(detail) => {
                tracing::error!(operation, error = %detail, "durable env store kubernetes API error");
                AppError::Internal(anyhow::anyhow!(
                    "durable environment store kubernetes API failure"
                ))
            }
        }
    }

    fn map_record_failure(
        &self,
        github_user_id: Option<i64>,
        name: Option<&str>,
        error: RecordError,
    ) -> AppError {
        tracing::error!(
            github_user_id,
            environment = name.unwrap_or("<unknown>"),
            error = %error,
            "durable env store record rejected"
        );
        AppError::Internal(anyhow::anyhow!(
            "durable environment profile integrity verification failed"
        ))
    }
}

#[async_trait]
impl EnvironmentProfileStore for DurableEnvStore {
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
    ) -> Result<(), AppError> {
        let now = now_rfc3339();
        let record = match expected_version {
            None => ProfileEnvelope::first_revision(
                id,
                login,
                name,
                install,
                variables,
                secrets,
                validated_at,
                content_hash,
                validation_image,
                &now,
                &now,
            )
            .map_err(|error| self.map_record_failure(Some(id), Some(name), error))?,
            Some(expected) => {
                let Some((current, actual)) = self.load_envelope(id, name).await? else {
                    return Err(AppError::Conflict(
                        "env store: the environment changed; please retry".to_string(),
                    ));
                };
                if actual != expected {
                    return Err(AppError::Conflict(
                        "env store: a concurrent update won; please retry".to_string(),
                    ));
                }
                current
                    .next_revision(
                        login,
                        install,
                        variables,
                        secrets,
                        validated_at,
                        content_hash,
                        validation_image,
                        &now,
                    )
                    .map_err(|error| self.map_record_failure(Some(id), Some(name), error))?
            }
        };
        let secret = secret_from_envelope(&record, &self.cipher, expected_version)
            .map_err(|error| self.map_record_failure(Some(id), Some(name), error))?;
        let result = match expected_version {
            None => self.api.create_secret(&self.namespace, &secret).await,
            Some(_) => {
                self.api
                    .replace_secret(&self.namespace, &env_object_name(id, name), &secret)
                    .await
            }
        };
        result.map_err(|error| self.map_api_failure("write durable record", error))?;
        tracing::info!(
            github_user_id = id,
            env = %name,
            revision = record.revision,
            "durable env store: environment written"
        );
        Ok(())
    }

    async fn get_environment(&self, id: i64, name: &str) -> Result<Option<EnvRecord>, AppError> {
        Ok(self
            .load_envelope(id, name)
            .await?
            .map(|(record, version)| record.public_record(Some(version))))
    }

    async fn list_environments(&self, id: i64) -> Result<Vec<EnvSummary>, AppError> {
        let selector = format!(
            "{},fkst.chrono-ai.fun/github-user-id={id}",
            environment_selector()
        );
        let secrets = self
            .api
            .list_secrets(&self.namespace, &selector)
            .await
            .map_err(|error| self.map_api_failure("list durable records", error))?;
        let mut summaries = Vec::with_capacity(secrets.len());
        for secret in &secrets {
            let (metadata_id, name) = identity_from_secret(secret)
                .map_err(|error| self.map_record_failure(Some(id), None, error))?;
            if metadata_id != id {
                return Err(self.map_record_failure(Some(id), Some(&name), RecordError::Metadata));
            }
            summaries.push(self.decode_secret(secret, id, &name)?.summary());
        }
        summaries.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(summaries)
    }

    async fn count_environments(&self, id: i64) -> Result<usize, AppError> {
        self.list_environments(id)
            .await
            .map(|records| records.len())
    }

    async fn delete_environment(&self, id: i64, name: &str) -> Result<bool, AppError> {
        self.api
            .delete_secret(&self.namespace, &env_object_name(id, name), None)
            .await
            .map_err(|error| self.map_api_failure("delete durable record", error))
    }

    async fn load_environment_for_session(
        &self,
        id: i64,
        name: &str,
    ) -> Result<Option<(Vec<String>, BTreeMap<String, String>, Vec<String>)>, AppError> {
        let Some((record, _)) = self.load_envelope(id, name).await? else {
            return Ok(None);
        };
        let mut merged = record.variables;
        let secret_keys: Vec<String> = record.secrets.keys().cloned().collect();
        merged.extend(record.secrets);
        Ok(Some((record.install, merged, secret_keys)))
    }
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
mod tests;
