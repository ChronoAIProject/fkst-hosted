//! Versioned encrypted-record shape and Kubernetes metadata projections.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::chrono::{DateTime, Utc};
use k8s_openapi::ByteString;
use serde::{Deserialize, Serialize};

use super::crypto::ProfileCipher;
use crate::k8s::env_store::meta::{
    content_hash, env_annotations, env_labels, env_object_name, private_content_hash, EnvRecord,
    EnvSummary, COMPONENT_LABEL, COMPONENT_VALUE, CONTENT_HASH_ANNOTATION, ENV_NAME_ANNOTATION,
    LOGIN_LABEL, STATUS_ANNOTATION, STATUS_READY, USER_ID_LABEL, VALIDATED_AT_ANNOTATION,
    VALIDATION_IMAGE_ANNOTATION,
};

pub(super) const SCHEMA_VERSION: u32 = 1;
pub(super) const DURABLE_SECRET_TYPE: &str = "fkst.chrono-ai.fun/environment-profile";
pub(super) const NONCE_DATA_KEY: &str = "nonce";
pub(super) const CIPHERTEXT_DATA_KEY: &str = "ciphertext";
pub(super) const SCHEMA_ANNOTATION: &str = "fkst.chrono-ai.fun/env-schema-version";
pub(super) const REVISION_ANNOTATION: &str = "fkst.chrono-ai.fun/env-revision";
pub(super) const CREATED_AT_ANNOTATION: &str = "fkst.chrono-ai.fun/env-created-at";
pub(super) const UPDATED_AT_ANNOTATION: &str = "fkst.chrono-ai.fun/env-updated-at";
pub(super) const SECRET_KEYS_ANNOTATION: &str = "fkst.chrono-ai.fun/env-secret-keys";

pub(super) fn environment_selector() -> String {
    format!("{COMPONENT_LABEL}={COMPONENT_VALUE}")
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct ProfileEnvelope {
    pub schema_version: u32,
    pub revision: u64,
    pub github_user_id: i64,
    pub github_login: String,
    pub name: String,
    pub install: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub secrets: BTreeMap<String, String>,
    pub validation_status: String,
    pub validated_at: String,
    pub validation_image: String,
    pub content_hash: String,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, thiserror::Error)]
pub(super) enum RecordError {
    #[error("record metadata is incomplete or inconsistent")]
    Metadata,
    #[error("record contents failed integrity validation")]
    Integrity,
    #[error(transparent)]
    Crypto(#[from] super::crypto::CryptoError),
}

impl ProfileEnvelope {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn first_revision(
        github_user_id: i64,
        github_login: &str,
        name: &str,
        install: &[String],
        variables: &BTreeMap<String, String>,
        secrets: &BTreeMap<String, String>,
        validated_at: &str,
        supplied_content_hash: &str,
        validation_image: &str,
        created_at: &str,
        updated_at: &str,
    ) -> Result<Self, RecordError> {
        let record = Self {
            schema_version: SCHEMA_VERSION,
            revision: 1,
            github_user_id,
            github_login: github_login.to_string(),
            name: name.to_string(),
            install: install.to_vec(),
            variables: variables.clone(),
            secrets: secrets.clone(),
            validation_status: STATUS_READY.to_string(),
            validated_at: validated_at.to_string(),
            validation_image: validation_image.to_string(),
            content_hash: supplied_content_hash.to_string(),
            created_at: created_at.to_string(),
            updated_at: updated_at.to_string(),
        };
        record.validate(github_user_id, name)?;
        Ok(record)
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn next_revision(
        &self,
        github_login: &str,
        install: &[String],
        variables: &BTreeMap<String, String>,
        secrets: &BTreeMap<String, String>,
        validated_at: &str,
        supplied_content_hash: &str,
        validation_image: &str,
        updated_at: &str,
    ) -> Result<Self, RecordError> {
        let revision = self.revision.checked_add(1).ok_or(RecordError::Integrity)?;
        Self::first_revision(
            self.github_user_id,
            github_login,
            &self.name,
            install,
            variables,
            secrets,
            validated_at,
            supplied_content_hash,
            validation_image,
            &self.created_at,
            updated_at,
        )
        .map(|mut record| {
            record.revision = revision;
            record
        })
    }

    pub(super) fn validate(
        &self,
        expected_id: i64,
        expected_name: &str,
    ) -> Result<(), RecordError> {
        if self.schema_version != SCHEMA_VERSION
            || self.revision == 0
            || self.github_user_id != expected_id
            || self.name != expected_name
            || self.validation_status != STATUS_READY
        {
            return Err(RecordError::Integrity);
        }
        let secret_keys: Vec<String> = self.secrets.keys().cloned().collect();
        if self.content_hash != content_hash(&self.install, &self.variables, &secret_keys) {
            return Err(RecordError::Integrity);
        }
        let created =
            DateTime::parse_from_rfc3339(&self.created_at).map_err(|_| RecordError::Integrity)?;
        let updated =
            DateTime::parse_from_rfc3339(&self.updated_at).map_err(|_| RecordError::Integrity)?;
        DateTime::parse_from_rfc3339(&self.validated_at).map_err(|_| RecordError::Integrity)?;
        if updated < created {
            return Err(RecordError::Integrity);
        }
        Ok(())
    }

    pub(super) fn public_record(&self, store_version: Option<String>) -> EnvRecord {
        EnvRecord {
            name: self.name.clone(),
            status: self.validation_status.clone(),
            validated_at: self.validated_at.clone(),
            install: self.install.clone(),
            variables: self.variables.clone(),
            secret_keys: self.secrets.keys().cloned().collect(),
            store_version,
            private_content_hash: Some(private_content_hash(
                &self.install,
                &self.variables,
                &self.secrets,
            )),
        }
    }

    pub(super) fn summary(&self) -> EnvSummary {
        EnvSummary {
            name: self.name.clone(),
            status: self.validation_status.clone(),
            validated_at: self.validated_at.clone(),
            install_command_count: self.install.len(),
            variable_count: self.variables.len(),
            secret_count: self.secrets.len(),
        }
    }
}

pub(super) fn identity_from_secret(secret: &Secret) -> Result<(i64, String), RecordError> {
    let labels = secret
        .metadata
        .labels
        .as_ref()
        .ok_or(RecordError::Metadata)?;
    let id = labels
        .get(USER_ID_LABEL)
        .ok_or(RecordError::Metadata)?
        .parse::<i64>()
        .map_err(|_| RecordError::Metadata)?;
    if id <= 0 || labels.get(COMPONENT_LABEL).map(String::as_str) != Some(COMPONENT_VALUE) {
        return Err(RecordError::Metadata);
    }
    let object = secret
        .metadata
        .name
        .as_deref()
        .ok_or(RecordError::Metadata)?;
    let prefix = format!("fkst-env-{id}-");
    let name = object.strip_prefix(&prefix).ok_or(RecordError::Metadata)?;
    if name.is_empty() {
        return Err(RecordError::Metadata);
    }
    Ok((id, name.to_string()))
}

pub(super) fn secret_from_envelope(
    record: &ProfileEnvelope,
    cipher: &ProfileCipher,
    resource_version: Option<&str>,
) -> Result<Secret, RecordError> {
    record.validate(record.github_user_id, &record.name)?;
    let sealed = cipher.seal(record)?;
    let mut annotations = env_annotations(
        &record.name,
        &record.validated_at,
        &record.content_hash,
        &record.validation_image,
    );
    annotations.insert(
        SCHEMA_ANNOTATION.to_string(),
        record.schema_version.to_string(),
    );
    annotations.insert(REVISION_ANNOTATION.to_string(), record.revision.to_string());
    annotations.insert(CREATED_AT_ANNOTATION.to_string(), record.created_at.clone());
    annotations.insert(UPDATED_AT_ANNOTATION.to_string(), record.updated_at.clone());
    let secret_keys: Vec<&str> = record.secrets.keys().map(String::as_str).collect();
    annotations.insert(
        SECRET_KEYS_ANNOTATION.to_string(),
        serde_json::to_string(&secret_keys).map_err(|_| RecordError::Integrity)?,
    );
    Ok(Secret {
        metadata: ObjectMeta {
            name: Some(env_object_name(record.github_user_id, &record.name)),
            labels: Some(env_labels(record.github_user_id, &record.github_login)),
            annotations: Some(annotations),
            resource_version: resource_version.map(str::to_string),
            ..ObjectMeta::default()
        },
        data: Some(BTreeMap::from([
            (NONCE_DATA_KEY.to_string(), ByteString(sealed.nonce)),
            (
                CIPHERTEXT_DATA_KEY.to_string(),
                ByteString(sealed.ciphertext),
            ),
        ])),
        type_: Some(DURABLE_SECRET_TYPE.to_string()),
        ..Secret::default()
    })
}

pub(super) fn envelope_from_secret(
    secret: &Secret,
    expected_id: i64,
    expected_name: &str,
    cipher: &ProfileCipher,
) -> Result<ProfileEnvelope, RecordError> {
    let (metadata_id, metadata_name) = identity_from_secret(secret)?;
    if metadata_id != expected_id
        || metadata_name != expected_name
        || secret.type_.as_deref() != Some(DURABLE_SECRET_TYPE)
    {
        return Err(RecordError::Metadata);
    }
    let annotations = secret
        .metadata
        .annotations
        .as_ref()
        .ok_or(RecordError::Metadata)?;
    let schema_version = annotations
        .get(SCHEMA_ANNOTATION)
        .ok_or(RecordError::Metadata)?
        .parse::<u32>()
        .map_err(|_| RecordError::Metadata)?;
    let data = secret.data.as_ref().ok_or(RecordError::Metadata)?;
    let nonce = data.get(NONCE_DATA_KEY).ok_or(RecordError::Metadata)?;
    let ciphertext = data.get(CIPHERTEXT_DATA_KEY).ok_or(RecordError::Metadata)?;
    if data.len() != 2 {
        return Err(RecordError::Metadata);
    }
    let record = cipher.open(
        expected_id,
        expected_name,
        schema_version,
        &nonce.0,
        &ciphertext.0,
    )?;
    record.validate(expected_id, expected_name)?;

    let expected_keys: Vec<&str> = record.secrets.keys().map(String::as_str).collect();
    let expected_keys =
        serde_json::to_string(&expected_keys).map_err(|_| RecordError::Integrity)?;
    let schema_version = record.schema_version.to_string();
    let revision = record.revision.to_string();
    let metadata_matches = [
        (ENV_NAME_ANNOTATION, record.name.as_str()),
        (STATUS_ANNOTATION, record.validation_status.as_str()),
        (VALIDATED_AT_ANNOTATION, record.validated_at.as_str()),
        (CONTENT_HASH_ANNOTATION, record.content_hash.as_str()),
        (
            VALIDATION_IMAGE_ANNOTATION,
            record.validation_image.as_str(),
        ),
        (SCHEMA_ANNOTATION, schema_version.as_str()),
        (REVISION_ANNOTATION, revision.as_str()),
        (CREATED_AT_ANNOTATION, record.created_at.as_str()),
        (UPDATED_AT_ANNOTATION, record.updated_at.as_str()),
        (SECRET_KEYS_ANNOTATION, expected_keys.as_str()),
    ]
    .into_iter()
    .all(|(key, expected)| annotations.get(key).map(String::as_str) == Some(expected));
    let expected_labels = env_labels(record.github_user_id, &record.github_login);
    let labels_match = [COMPONENT_LABEL, USER_ID_LABEL, LOGIN_LABEL]
        .into_iter()
        .all(|key| {
            secret
                .metadata
                .labels
                .as_ref()
                .and_then(|labels| labels.get(key))
                == expected_labels.get(key)
        });
    if !metadata_matches || !labels_match {
        return Err(RecordError::Metadata);
    }
    Ok(record)
}

pub(super) fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}
