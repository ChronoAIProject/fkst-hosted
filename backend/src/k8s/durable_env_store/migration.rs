//! Interruption-safe migration from legacy ConfigMap/Secret pairs.

use std::collections::{BTreeMap, BTreeSet};

use k8s_openapi::api::core::v1::{ConfigMap, Secret};

use super::record::ProfileEnvelope;
use super::DurableEnvStore;
use crate::error::AppError;
use crate::k8s::env_store::meta::{
    content_hash, COMPONENT_LABEL, COMPONENT_VALUE, CONTENT_HASH_ANNOTATION, ENV_NAME_ANNOTATION,
    INSTALL_KEY, LOGIN_LABEL, STATUS_ANNOTATION, STATUS_READY, USER_ID_LABEL,
    VALIDATED_AT_ANNOTATION, VALIDATION_IMAGE_ANNOTATION, VARIABLES_KEY,
};

struct LegacyProfile {
    envelope: ProfileEnvelope,
}

fn object_name(metadata: &kube::core::ObjectMeta) -> Result<String, AppError> {
    metadata.name.clone().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "legacy environment object has no metadata.name"
        ))
    })
}

fn identity_from_object_name(object: &str) -> Result<(i64, String), AppError> {
    let remainder = object.strip_prefix("fkst-env-").ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("legacy environment object name is invalid"))
    })?;
    let (id, name) = remainder.split_once('-').ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("legacy environment object name is invalid"))
    })?;
    let id = id.parse::<i64>().map_err(|_| {
        AppError::Internal(anyhow::anyhow!(
            "legacy environment owner identity is invalid"
        ))
    })?;
    if id <= 0 || name.is_empty() {
        return Err(AppError::Internal(anyhow::anyhow!(
            "legacy environment identity is invalid"
        )));
    }
    Ok((id, name.to_string()))
}

fn required_annotation<'a>(
    metadata: &'a kube::core::ObjectMeta,
    key: &str,
) -> Result<&'a str, AppError> {
    metadata
        .annotations
        .as_ref()
        .and_then(|annotations| annotations.get(key))
        .map(String::as_str)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| {
            AppError::Internal(anyhow::anyhow!("legacy environment metadata is incomplete"))
        })
}

fn validate_identity(
    metadata: &kube::core::ObjectMeta,
    expected_id: i64,
    expected_name: &str,
) -> Result<String, AppError> {
    let labels = metadata.labels.as_ref().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("legacy environment labels are missing"))
    })?;
    if labels.get(COMPONENT_LABEL).map(String::as_str) != Some(COMPONENT_VALUE)
        || labels.get(USER_ID_LABEL).map(String::as_str) != Some(expected_id.to_string().as_str())
        || required_annotation(metadata, ENV_NAME_ANNOTATION)? != expected_name
    {
        return Err(AppError::Internal(anyhow::anyhow!(
            "legacy environment identity is inconsistent"
        )));
    }
    labels
        .get(LOGIN_LABEL)
        .cloned()
        .ok_or_else(|| AppError::Internal(anyhow::anyhow!("legacy environment login is missing")))
}

fn legacy_profile(
    id: i64,
    name: &str,
    config_map: &ConfigMap,
    secret: &Secret,
) -> Result<LegacyProfile, AppError> {
    let login = validate_identity(&config_map.metadata, id, name)?;
    if validate_identity(&secret.metadata, id, name)? != login {
        return Err(AppError::Internal(anyhow::anyhow!(
            "legacy environment pair owner metadata differs"
        )));
    }
    for key in [
        STATUS_ANNOTATION,
        VALIDATED_AT_ANNOTATION,
        CONTENT_HASH_ANNOTATION,
        VALIDATION_IMAGE_ANNOTATION,
    ] {
        if required_annotation(&config_map.metadata, key)?
            != required_annotation(&secret.metadata, key)?
        {
            return Err(AppError::Internal(anyhow::anyhow!(
                "legacy environment pair metadata differs"
            )));
        }
    }
    if required_annotation(&config_map.metadata, STATUS_ANNOTATION)? != STATUS_READY {
        return Err(AppError::Internal(anyhow::anyhow!(
            "legacy environment is not ready"
        )));
    }
    let data = config_map.data.as_ref().ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!("legacy environment config is missing"))
    })?;
    let install = serde_json::from_str::<Vec<String>>(data.get(INSTALL_KEY).ok_or_else(|| {
        AppError::Internal(anyhow::anyhow!(
            "legacy environment install data is missing"
        ))
    })?)
    .map_err(|_| AppError::Internal(anyhow::anyhow!("legacy install data is invalid")))?;
    let variables =
        serde_json::from_str::<BTreeMap<String, String>>(data.get(VARIABLES_KEY).ok_or_else(
            || AppError::Internal(anyhow::anyhow!("legacy environment variables are missing")),
        )?)
        .map_err(|_| AppError::Internal(anyhow::anyhow!("legacy variable data is invalid")))?;
    let mut secrets = BTreeMap::new();
    for (key, value) in secret.data.as_ref().into_iter().flatten() {
        let value = String::from_utf8(value.0.clone())
            .map_err(|_| AppError::Internal(anyhow::anyhow!("legacy secret data is invalid")))?;
        secrets.insert(key.clone(), value);
    }
    let secret_keys: Vec<String> = secrets.keys().cloned().collect();
    let supplied_hash = required_annotation(&config_map.metadata, CONTENT_HASH_ANNOTATION)?;
    if content_hash(&install, &variables, &secret_keys) != supplied_hash {
        return Err(AppError::Internal(anyhow::anyhow!(
            "legacy environment content hash is invalid"
        )));
    }
    let validated_at = required_annotation(&config_map.metadata, VALIDATED_AT_ANNOTATION)?;
    let validation_image = required_annotation(&config_map.metadata, VALIDATION_IMAGE_ANNOTATION)?;
    let envelope = ProfileEnvelope::first_revision(
        id,
        &login,
        name,
        &install,
        &variables,
        &secrets,
        validated_at,
        supplied_hash,
        validation_image,
        validated_at,
        validated_at,
    )
    .map_err(|_| {
        AppError::Internal(anyhow::anyhow!(
            "legacy environment record failed validation"
        ))
    })?;
    Ok(LegacyProfile { envelope })
}

impl DurableEnvStore {
    pub(super) async fn migrate_legacy(&self, legacy_namespace: &str) -> Result<usize, AppError> {
        let selector = format!("{COMPONENT_LABEL}={COMPONENT_VALUE}");
        let config_maps = self
            .api
            .list_config_maps(legacy_namespace, &selector)
            .await
            .map_err(|error| self.map_api_failure("list legacy config maps", error))?;
        let secrets = self
            .api
            .list_secrets(legacy_namespace, &selector)
            .await
            .map_err(|error| self.map_api_failure("list legacy secrets", error))?;

        let mut config_maps_by_name = BTreeMap::new();
        for config_map in config_maps {
            let name = object_name(&config_map.metadata)?;
            config_maps_by_name.insert(name, config_map);
        }
        let mut secrets_by_name = BTreeMap::new();
        for secret in secrets {
            let name = object_name(&secret.metadata)?;
            secrets_by_name.insert(name, secret);
        }
        let names: BTreeSet<String> = config_maps_by_name
            .keys()
            .chain(secrets_by_name.keys())
            .cloned()
            .collect();

        let mut migrated = 0;
        for object in names {
            let (id, name) = identity_from_object_name(&object)?;
            let config_map = config_maps_by_name.get(&object);
            let secret = secrets_by_name.get(&object);

            // A durable record is authoritative. This branch also completes an
            // interrupted cleanup where only one legacy half remains.
            if self.load_envelope(id, &name).await?.is_some() {
                self.cleanup_legacy(legacy_namespace, &object, config_map, secret)
                    .await?;
                continue;
            }

            let (Some(config_map), Some(secret)) = (config_map, secret) else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "legacy environment pair is incomplete"
                )));
            };
            let legacy = legacy_profile(id, &name, config_map, secret)?;
            match self.create_envelope(&legacy.envelope).await {
                Ok(()) => {}
                Err(AppError::Conflict(_)) => {
                    // Another startup instance won the create. Its complete,
                    // decryptable record wins just like any pre-existing one.
                    if self.load_envelope(id, &name).await?.is_none() {
                        return Err(AppError::Conflict(
                            "env migration: concurrent create did not converge".to_string(),
                        ));
                    }
                }
                Err(error) => return Err(error),
            }
            let Some((persisted, _)) = self.load_envelope(id, &name).await? else {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "migrated environment could not be verified"
                )));
            };
            if persisted != legacy.envelope {
                return Err(AppError::Internal(anyhow::anyhow!(
                    "migrated environment verification mismatch"
                )));
            }
            self.cleanup_legacy(legacy_namespace, &object, Some(config_map), Some(secret))
                .await?;
            migrated += 1;
        }
        Ok(migrated)
    }

    async fn cleanup_legacy(
        &self,
        namespace: &str,
        object: &str,
        config_map: Option<&ConfigMap>,
        secret: Option<&Secret>,
    ) -> Result<(), AppError> {
        if let Some(config_map) = config_map {
            self.api
                .delete_config_map(
                    namespace,
                    object,
                    config_map.metadata.resource_version.as_deref(),
                )
                .await
                .map_err(|error| self.map_api_failure("delete legacy config map", error))?;
        }
        if let Some(secret) = secret {
            self.api
                .delete_secret(
                    namespace,
                    object,
                    secret.metadata.resource_version.as_deref(),
                )
                .await
                .map_err(|error| self.map_api_failure("delete legacy secret", error))?;
        }
        Ok(())
    }
}
