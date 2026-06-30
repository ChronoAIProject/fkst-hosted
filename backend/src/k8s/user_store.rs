//! Per-user environment + secret store, backed by Kubernetes objects (PR4a).
//!
//! Each user's config lives in TWO objects in the control-plane namespace
//! ([`crate::config::PodConfig::namespace`]), both named `fkst-user-<id>` and
//! keyed by the caller's immutable numeric GitHub id:
//!   - a **ConfigMap** holding the non-secret env "variables", and
//!   - a **Secret** holding the secret env values.
//!
//! Differentiating by object *name* (not a separate namespace) keeps every
//! user's store side-by-side with the control plane's own objects. The id keys
//! the objects, so the verified GitHub identity — never a request value —
//! determines which store is touched.
//!
//! These functions are the ONLY place that reads/writes a user store; the route
//! handlers ([`crate::routes::user_env`]) call through them. Writes are
//! merge-upserts (get → merge → replace, create-if-absent) so a `PUT` adds/over-
//! writes the named keys without clobbering the rest. Secret values are NEVER
//! returned — only key names leave this module for a Secret.

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::{ConfigMap, Secret};
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use k8s_openapi::ByteString;
use kube::api::{Api, PostParams};

use crate::error::AppError;
use crate::k8s::KubeClient;

/// Label key carrying the user's GitHub login (readability only — the object
/// name carries the authoritative numeric id).
const LOGIN_LABEL: &str = "fkst.chrono-ai.fun/github-login";

/// The deterministic object name for a user's ConfigMap + Secret. Keyed by the
/// immutable numeric GitHub id (logins are renamable).
pub fn user_object_name(github_user_id: i64) -> String {
    format!("fkst-user-{github_user_id}")
}

/// Common labels stamped on a user's ConfigMap + Secret.
pub fn store_labels(login: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (
            "app.kubernetes.io/part-of".to_string(),
            "fkst-hosted".to_string(),
        ),
        (
            "app.kubernetes.io/component".to_string(),
            "user-store".to_string(),
        ),
        (LOGIN_LABEL.to_string(), sanitize_label_value(login)),
    ])
}

/// Coerce an arbitrary string into a valid Kubernetes label *value* (≤63 chars,
/// `[A-Za-z0-9._-]`, alphanumeric ends). GitHub logins are already label-safe,
/// but the login crosses a trust boundary (GitHub's `/user` response), so we
/// fail safe rather than let an odd value 500 the whole store. An empty result
/// is itself a valid label value.
fn sanitize_label_value(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .take(63)
        .collect();
    cleaned
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_string()
}

/// Map a Kubernetes API error onto the unified type. A `409` (a concurrent
/// writer won the optimistic get→replace race) is a client-retriable conflict;
/// everything else is an internal failure whose detail is logged, never echoed.
fn map_kube_err(e: kube::Error) -> AppError {
    if let kube::Error::Api(ref api_err) = e {
        if api_err.code == 409 {
            return AppError::Conflict(
                "user store: a concurrent update won; please retry".to_string(),
            );
        }
    }
    tracing::error!(error = %e, "user store kubernetes api error");
    AppError::Internal(anyhow::anyhow!("user store kubernetes api: {e}"))
}

/// Reject a resulting store that would exceed the per-user entry cap.
fn enforce_entries_cap(count: usize, cap: usize) -> Result<(), AppError> {
    if count > cap {
        return Err(AppError::Unprocessable(format!(
            "too many entries: {count} exceeds the per-user cap of {cap}"
        )));
    }
    Ok(())
}

/// Merge `store_labels` into an object's metadata (idempotent on re-write).
fn apply_labels(meta: &mut ObjectMeta, login: &str) {
    meta.labels
        .get_or_insert_with(BTreeMap::new)
        .extend(store_labels(login));
}

/// The Secret's data key NAMES, sorted — never the values.
fn secret_key_names(secret: &Secret) -> Vec<String> {
    let mut keys: Vec<String> = secret
        .data
        .as_ref()
        .map(|d| d.keys().cloned().collect())
        .unwrap_or_default();
    // A freshly created Secret may echo `string_data` instead of `data`.
    if let Some(sd) = secret.string_data.as_ref() {
        for k in sd.keys() {
            keys.push(k.clone());
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

fn configmap_api(kube: &KubeClient) -> Api<ConfigMap> {
    Api::namespaced(kube.client().clone(), kube.namespace())
}

fn secret_api(kube: &KubeClient) -> Api<Secret> {
    Api::namespaced(kube.client().clone(), kube.namespace())
}

/// The user's non-secret variables (empty when the ConfigMap is absent).
pub async fn get_env(
    kube: &KubeClient,
    github_user_id: i64,
) -> Result<BTreeMap<String, String>, AppError> {
    let name = user_object_name(github_user_id);
    match configmap_api(kube)
        .get_opt(&name)
        .await
        .map_err(map_kube_err)?
    {
        Some(cm) => Ok(cm.data.unwrap_or_default()),
        None => Ok(BTreeMap::new()),
    }
}

/// The NAMES of the user's secret keys (empty when the Secret is absent). The
/// values never leave the cluster.
pub async fn get_secret_keys(
    kube: &KubeClient,
    github_user_id: i64,
) -> Result<Vec<String>, AppError> {
    let name = user_object_name(github_user_id);
    match secret_api(kube)
        .get_opt(&name)
        .await
        .map_err(map_kube_err)?
    {
        Some(secret) => Ok(secret_key_names(&secret)),
        None => Ok(Vec::new()),
    }
}

/// Merge-upsert `variables` into the user's ConfigMap (creating it if absent),
/// returning the resulting variable map. Existing keys not named here are kept.
pub async fn merge_env(
    kube: &KubeClient,
    github_user_id: i64,
    login: &str,
    variables: BTreeMap<String, String>,
    entries_cap: usize,
) -> Result<BTreeMap<String, String>, AppError> {
    let api = configmap_api(kube);
    let name = user_object_name(github_user_id);
    match api.get_opt(&name).await.map_err(map_kube_err)? {
        Some(mut cm) => {
            let data = cm.data.get_or_insert_with(BTreeMap::new);
            data.extend(variables);
            enforce_entries_cap(data.len(), entries_cap)?;
            apply_labels(&mut cm.metadata, login);
            let replaced = api
                .replace(&name, &PostParams::default(), &cm)
                .await
                .map_err(map_kube_err)?;
            Ok(replaced.data.unwrap_or_default())
        }
        None => {
            enforce_entries_cap(variables.len(), entries_cap)?;
            let cm = ConfigMap {
                metadata: ObjectMeta {
                    name: Some(name.clone()),
                    labels: Some(store_labels(login)),
                    ..Default::default()
                },
                data: Some(variables),
                ..Default::default()
            };
            let created = api
                .create(&PostParams::default(), &cm)
                .await
                .map_err(map_kube_err)?;
            Ok(created.data.unwrap_or_default())
        }
    }
}

/// Merge-upsert `secrets` into the user's Secret (creating it if absent),
/// returning the resulting key NAMES (never values). Existing keys not named
/// here are kept.
pub async fn merge_secrets(
    kube: &KubeClient,
    github_user_id: i64,
    login: &str,
    secrets: BTreeMap<String, String>,
    entries_cap: usize,
) -> Result<Vec<String>, AppError> {
    let api = secret_api(kube);
    let name = user_object_name(github_user_id);
    match api.get_opt(&name).await.map_err(map_kube_err)? {
        Some(mut secret) => {
            let data = secret.data.get_or_insert_with(BTreeMap::new);
            for (k, v) in secrets {
                data.insert(k, ByteString(v.into_bytes()));
            }
            enforce_entries_cap(data.len(), entries_cap)?;
            // `string_data` is a write-only convenience; the merged truth is in
            // `data`, so clear it to avoid a stale double-write.
            secret.string_data = None;
            apply_labels(&mut secret.metadata, login);
            let replaced = api
                .replace(&name, &PostParams::default(), &secret)
                .await
                .map_err(map_kube_err)?;
            Ok(secret_key_names(&replaced))
        }
        None => {
            enforce_entries_cap(secrets.len(), entries_cap)?;
            let data: BTreeMap<String, ByteString> = secrets
                .into_iter()
                .map(|(k, v)| (k, ByteString(v.into_bytes())))
                .collect();
            let secret = Secret {
                metadata: ObjectMeta {
                    name: Some(name.clone()),
                    labels: Some(store_labels(login)),
                    ..Default::default()
                },
                data: Some(data),
                type_: Some("Opaque".to_string()),
                ..Default::default()
            };
            let created = api
                .create(&PostParams::default(), &secret)
                .await
                .map_err(map_kube_err)?;
            Ok(secret_key_names(&created))
        }
    }
}

/// Remove one variable from the user's ConfigMap. Idempotent: a missing object
/// or missing key is reported as `false` (already gone), never an error.
pub async fn delete_env_key(
    kube: &KubeClient,
    github_user_id: i64,
    key: &str,
) -> Result<bool, AppError> {
    let api = configmap_api(kube);
    let name = user_object_name(github_user_id);
    match api.get_opt(&name).await.map_err(map_kube_err)? {
        Some(mut cm) => {
            let removed = cm
                .data
                .as_mut()
                .map(|d| d.remove(key).is_some())
                .unwrap_or(false);
            if removed {
                api.replace(&name, &PostParams::default(), &cm)
                    .await
                    .map_err(map_kube_err)?;
            }
            Ok(removed)
        }
        None => Ok(false),
    }
}

/// Remove one secret from the user's Secret. Idempotent like [`delete_env_key`].
pub async fn delete_secret_key(
    kube: &KubeClient,
    github_user_id: i64,
    key: &str,
) -> Result<bool, AppError> {
    let api = secret_api(kube);
    let name = user_object_name(github_user_id);
    match api.get_opt(&name).await.map_err(map_kube_err)? {
        Some(mut secret) => {
            let removed = secret
                .data
                .as_mut()
                .map(|d| d.remove(key).is_some())
                .unwrap_or(false);
            if removed {
                secret.string_data = None;
                api.replace(&name, &PostParams::default(), &secret)
                    .await
                    .map_err(map_kube_err)?;
            }
            Ok(removed)
        }
        None => Ok(false),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn object_name_is_keyed_by_numeric_id() {
        assert_eq!(user_object_name(42), "fkst-user-42");
        assert_eq!(user_object_name(583231), "fkst-user-583231");
        // The login plays NO part in the object name (only the immutable id).
    }

    #[test]
    fn store_labels_carry_login_and_component() {
        let labels = store_labels("octocat");
        assert_eq!(labels["app.kubernetes.io/part-of"], "fkst-hosted");
        assert_eq!(labels["app.kubernetes.io/component"], "user-store");
        assert_eq!(labels[LOGIN_LABEL], "octocat");
    }

    #[test]
    fn sanitize_label_value_coerces_to_a_valid_label() {
        // Already-safe logins pass through untouched.
        assert_eq!(sanitize_label_value("octo-cat_1.2"), "octo-cat_1.2");
        // Disallowed chars become hyphens; leading/trailing non-alnum trimmed.
        assert_eq!(sanitize_label_value("a/b c"), "a-b-c");
        assert_eq!(sanitize_label_value("-weird-"), "weird");
        // Over-long values are truncated to 63 chars.
        let long = "a".repeat(100);
        assert_eq!(sanitize_label_value(&long).len(), 63);
    }

    #[test]
    fn secret_key_names_returns_sorted_names_never_values() {
        let mut data = BTreeMap::new();
        data.insert("ZED".to_string(), ByteString(b"super-secret".to_vec()));
        data.insert(
            "API_KEY".to_string(),
            ByteString(b"another-secret".to_vec()),
        );
        let secret = Secret {
            data: Some(data),
            ..Default::default()
        };
        let names = secret_key_names(&secret);
        assert_eq!(names, vec!["API_KEY".to_string(), "ZED".to_string()]);
        // The values must never appear in the projected key list.
        assert!(!names.iter().any(|n| n.contains("secret")));
    }

    #[test]
    fn secret_key_names_dedups_data_and_string_data() {
        let mut data = BTreeMap::new();
        data.insert("A".to_string(), ByteString(b"x".to_vec()));
        let mut string_data = BTreeMap::new();
        string_data.insert("A".to_string(), "x".to_string());
        string_data.insert("B".to_string(), "y".to_string());
        let secret = Secret {
            data: Some(data),
            string_data: Some(string_data),
            ..Default::default()
        };
        assert_eq!(
            secret_key_names(&secret),
            vec!["A".to_string(), "B".to_string()]
        );
    }

    #[test]
    fn enforce_entries_cap_rejects_over_cap() {
        assert!(enforce_entries_cap(5, 10).is_ok());
        assert!(enforce_entries_cap(10, 10).is_ok());
        let err = enforce_entries_cap(11, 10).expect_err("over cap must fail");
        assert!(matches!(err, AppError::Unprocessable(_)));
    }
}
