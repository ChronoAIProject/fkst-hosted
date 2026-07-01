//! Named-environment data layer (issue #338 §2), backed by a paired Kubernetes
//! ConfigMap + Secret per named environment.
//!
//! A user may define several named environments; each is stored as TWO objects
//! in the control-plane namespace ([`crate::config::PodConfig::namespace`]), both
//! named `fkst-env-<id>-<name>` and keyed by the caller's immutable numeric
//! GitHub id ([`env_object_name`]):
//!   - a **ConfigMap** holding the ordered install commands and the non-secret
//!     variables under two RESERVED dotted data keys (`.install` / `.variables`),
//!   - a **Secret** holding one data key per secret KEY (individual keys so the
//!     names can be listed without ever exposing a value).
//!
//! This file owns the cluster I/O; the pure metadata/projection helpers live in
//! the sibling [`meta`] module (split only to respect the 500-line limit). The
//! whole layer is SELF-CONTAINED — it shares no code with [`crate::k8s::user_store`]
//! — so a later PR can delete `user_store` cleanly.
//!
//! Secret VALUES leave this module through EXACTLY ONE function —
//! [`load_environment_for_session`], the in-cluster session-injection path. Every
//! other reader ([`get_environment`], [`list_environments`]) exposes secret KEY
//! NAMES only, so a value never crosses the API boundary.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::ByteString;
use kube::api::{Api, DeleteParams, ListParams, PostParams};

use crate::error::AppError;
use crate::k8s::KubeClient;

#[path = "env_store_meta.rs"]
mod meta;

// The data-layer vocabulary later PRs (route + launcher) consume through the
// stable `crate::k8s::env_store::*` path.
use meta::*;
pub use meta::{content_hash, env_object_name, EnvRecord, EnvSummary};

/// Map a Kubernetes API error onto the unified type. A `409` (a concurrent writer
/// won a race the create/replace path did not resolve) is a client-retriable
/// conflict; everything else is an internal failure whose detail is logged, never
/// echoed.
fn map_kube_err(e: kube::Error) -> AppError {
    if let kube::Error::Api(ref api_err) = e {
        if api_err.code == 409 {
            return AppError::Conflict(
                "env store: a concurrent update won; please retry".to_string(),
            );
        }
    }
    tracing::error!(error = %e, "env store kubernetes api error");
    AppError::Internal(anyhow::anyhow!("env store kubernetes api: {e}"))
}

fn configmap_api(kube: &KubeClient) -> Api<ConfigMap> {
    Api::namespaced(kube.client().clone(), kube.namespace())
}

fn secret_api(kube: &KubeClient) -> Api<Secret> {
    Api::namespaced(kube.client().clone(), kube.namespace())
}

/// Create the ConfigMap, or replace it when it already exists (`409`). Fetching
/// the live `resourceVersion` first is required for a `replace`.
async fn upsert_configmap(
    api: &Api<ConfigMap>,
    name: &str,
    mut cm: ConfigMap,
) -> Result<(), AppError> {
    match api.create(&PostParams::default(), &cm).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(api_err)) if api_err.code == 409 => {
            let existing = api.get(name).await.map_err(map_kube_err)?;
            cm.metadata.resource_version = existing.metadata.resource_version;
            api.replace(name, &PostParams::default(), &cm)
                .await
                .map_err(map_kube_err)?;
            Ok(())
        }
        Err(e) => Err(map_kube_err(e)),
    }
}

/// Create the Secret, or replace it when it already exists (`409`).
async fn upsert_secret(api: &Api<Secret>, name: &str, mut secret: Secret) -> Result<(), AppError> {
    match api.create(&PostParams::default(), &secret).await {
        Ok(_) => Ok(()),
        Err(kube::Error::Api(api_err)) if api_err.code == 409 => {
            let existing = api.get(name).await.map_err(map_kube_err)?;
            secret.metadata.resource_version = existing.metadata.resource_version;
            api.replace(name, &PostParams::default(), &secret)
                .await
                .map_err(map_kube_err)?;
            Ok(())
        }
        Err(e) => Err(map_kube_err(e)),
    }
}

/// Delete one object by name, treating a `404` as "already gone" (`false`).
async fn delete_configmap(api: &Api<ConfigMap>, name: &str) -> Result<bool, AppError> {
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => Ok(true),
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => Ok(false),
        Err(e) => Err(map_kube_err(e)),
    }
}

async fn delete_secret(api: &Api<Secret>, name: &str) -> Result<bool, AppError> {
    match api.delete(name, &DeleteParams::default()).await {
        Ok(_) => Ok(true),
        Err(kube::Error::Api(api_err)) if api_err.code == 404 => Ok(false),
        Err(e) => Err(map_kube_err(e)),
    }
}

/// Write one named environment as a Secret + ConfigMap pair.
///
/// Validate-then-swap write order: the Secret is written FIRST and the ConfigMap
/// (which carries `validation-status: ready`) LAST, so a partial write is never
/// observed as ready. Each object is create-or-replace (a `409` on create falls
/// through to a `resourceVersion`-matched replace). The Secret is always written
/// — even with no secrets — so the pair exists atomically.
#[allow(clippy::too_many_arguments)]
pub async fn put_environment(
    kube: &KubeClient,
    id: i64,
    login: &str,
    name: &str,
    install: &[String],
    variables: &BTreeMap<String, String>,
    secrets: &BTreeMap<String, String>,
    validated_at: &str,
    content_hash: &str,
    validation_image: &str,
) -> Result<(), AppError> {
    let object = env_object_name(id, name);
    let labels = env_labels(id, login);
    let annotations = env_annotations(name, validated_at, content_hash, validation_image);

    // Secret FIRST.
    let secret_data: BTreeMap<String, ByteString> = secrets
        .iter()
        .map(|(k, v)| (k.clone(), ByteString(v.clone().into_bytes())))
        .collect();
    let secret = Secret {
        metadata: ObjectMeta {
            name: Some(object.clone()),
            labels: Some(labels.clone()),
            annotations: Some(annotations.clone()),
            ..Default::default()
        },
        data: Some(secret_data),
        type_: Some("Opaque".to_string()),
        ..Default::default()
    };
    upsert_secret(&secret_api(kube), &object, secret).await?;

    // ConfigMap LAST (carries the `ready` marker).
    let install_json = serde_json::to_string(install)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize install: {e}")))?;
    let variables_json = serde_json::to_string(variables)
        .map_err(|e| AppError::Internal(anyhow::anyhow!("serialize variables: {e}")))?;
    let mut data = BTreeMap::new();
    data.insert(INSTALL_KEY.to_string(), install_json);
    data.insert(VARIABLES_KEY.to_string(), variables_json);
    let cm = ConfigMap {
        metadata: ObjectMeta {
            name: Some(object.clone()),
            labels: Some(labels),
            annotations: Some(annotations),
            ..Default::default()
        },
        data: Some(data),
        ..Default::default()
    };
    upsert_configmap(&configmap_api(kube), &object, cm).await?;

    tracing::info!(github_user_id = id, env = %name, "env store: environment written");
    Ok(())
}

/// Read one named environment's public view (install + variables + status +
/// secret key NAMES). `None` when the ConfigMap is absent. NEVER reads secret
/// values.
pub async fn get_environment(
    kube: &KubeClient,
    id: i64,
    name: &str,
) -> Result<Option<EnvRecord>, AppError> {
    let object = env_object_name(id, name);
    let cm = match configmap_api(kube)
        .get_opt(&object)
        .await
        .map_err(map_kube_err)?
    {
        Some(cm) => cm,
        None => return Ok(None),
    };
    let data = cm.data.unwrap_or_default();
    let secret_keys = match secret_api(kube)
        .get_opt(&object)
        .await
        .map_err(map_kube_err)?
    {
        Some(secret) => secret_key_names(&secret),
        None => Vec::new(),
    };
    Ok(Some(EnvRecord {
        name: annotation(&cm.metadata, ENV_NAME_ANNOTATION),
        status: annotation(&cm.metadata, STATUS_ANNOTATION),
        validated_at: annotation(&cm.metadata, VALIDATED_AT_ANNOTATION),
        install: parse_install(&data),
        variables: parse_variables(&data),
        secret_keys,
    }))
}

/// List a user's named environments as compact summaries (counts only). Joins the
/// ConfigMaps (install + variable counts) with the Secrets (secret counts) by
/// object name. Sorted by env name for a stable response.
pub async fn list_environments(kube: &KubeClient, id: i64) -> Result<Vec<EnvSummary>, AppError> {
    let selector = owner_selector(id);
    let lp = ListParams::default().labels(&selector);
    let configmaps = configmap_api(kube).list(&lp).await.map_err(map_kube_err)?;
    let secrets = secret_api(kube).list(&lp).await.map_err(map_kube_err)?;

    let secret_counts: BTreeMap<String, usize> = secrets
        .items
        .iter()
        .filter_map(|s| {
            s.metadata
                .name
                .clone()
                .map(|n| (n, s.data.as_ref().map(|d| d.len()).unwrap_or(0)))
        })
        .collect();

    let mut out: Vec<EnvSummary> = configmaps
        .items
        .into_iter()
        .map(|cm| {
            let object = cm.metadata.name.clone().unwrap_or_default();
            let data = cm.data.clone().unwrap_or_default();
            EnvSummary {
                name: annotation(&cm.metadata, ENV_NAME_ANNOTATION),
                status: annotation(&cm.metadata, STATUS_ANNOTATION),
                validated_at: annotation(&cm.metadata, VALIDATED_AT_ANNOTATION),
                install_command_count: parse_install(&data).len(),
                variable_count: parse_variables(&data).len(),
                secret_count: secret_counts.get(&object).copied().unwrap_or(0),
            }
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(out)
}

/// Count a user's named environments — for the per-user cap check.
pub async fn count_environments(kube: &KubeClient, id: i64) -> Result<usize, AppError> {
    let selector = owner_selector(id);
    let lp = ListParams::default().labels(&selector);
    let list = configmap_api(kube).list(&lp).await.map_err(map_kube_err)?;
    Ok(list.items.len())
}

/// Delete both the ConfigMap and Secret for one named environment. Idempotent /
/// `404`-tolerant. Returns whether ANYTHING existed (either object present).
pub async fn delete_environment(kube: &KubeClient, id: i64, name: &str) -> Result<bool, AppError> {
    let object = env_object_name(id, name);
    let cm_existed = delete_configmap(&configmap_api(kube), &object).await?;
    let secret_existed = delete_secret(&secret_api(kube), &object).await?;
    Ok(cm_existed || secret_existed)
}

/// SERVER-SIDE ONLY: resolve one environment into `(install, merged_env)` for the
/// session launcher — install commands plus the variables AND secret VALUES
/// merged into one injection map.
///
/// This is the ONLY function that reads secret VALUES out of the store. It exists
/// solely for the in-cluster session-injection path and is NEVER wired to any
/// user-facing route, so a secret value never crosses the API boundary through
/// here. `None` when the ConfigMap is absent.
pub async fn load_environment_for_session(
    kube: &KubeClient,
    id: i64,
    name: &str,
) -> Result<Option<(Vec<String>, BTreeMap<String, String>)>, AppError> {
    let object = env_object_name(id, name);
    let cm = match configmap_api(kube)
        .get_opt(&object)
        .await
        .map_err(map_kube_err)?
    {
        Some(cm) => cm,
        None => return Ok(None),
    };
    let data = cm.data.unwrap_or_default();
    let install = parse_install(&data);
    let mut merged = parse_variables(&data);
    // Overlay secret values. Keys are unique across the two stores (the route
    // layer forbids a name in both), so this only fills secret-only keys.
    if let Some(secret) = secret_api(kube)
        .get_opt(&object)
        .await
        .map_err(map_kube_err)?
    {
        for (k, v) in decode_secret_values(&secret) {
            merged.insert(k, v);
        }
    }
    Ok(Some((install, merged)))
}
