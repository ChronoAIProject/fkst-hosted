//! Named-environment metadata + projection helpers (issue #338 §2).
//!
//! The pure, cluster-free half of the [`super`] (env_store) data layer: object
//! naming, label/annotation composition, secret-name projection (never values),
//! the reserved-key (de)serialization, and the public content hash. Split from
//! the cluster-I/O half purely to keep each file under the 500-line limit.
//!
//! SELF-CONTAINED: it re-implements (does not import) the generic helpers it
//! once shared in spirit with the old flat `user_store` — label-value
//! sanitizing, secret-key-name listing, secret-value decoding — so that store
//! could be deleted cleanly without touching this file (it now has been).

use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::Secret;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::ObjectMeta;
use serde::Serialize;
use sha2::{Digest, Sha256};

// ---- labels & annotations -------------------------------------------------

pub(crate) const PART_OF_LABEL: &str = "app.kubernetes.io/part-of";
pub(crate) const COMPONENT_LABEL: &str = "app.kubernetes.io/component";
/// Label carrying the immutable numeric GitHub id (the authoritative owner key;
/// the object name also embeds it, but the label enables list-by-owner).
pub(crate) const USER_ID_LABEL: &str = "fkst.chrono-ai.fun/github-user-id";
/// Label carrying the GitHub login (readability only — renamable, so never a key).
pub(crate) const LOGIN_LABEL: &str = "fkst.chrono-ai.fun/github-login";
pub(crate) const COMPONENT_VALUE: &str = "user-env";

pub(crate) const ENV_NAME_ANNOTATION: &str = "fkst.chrono-ai.fun/env-name";
pub(crate) const STATUS_ANNOTATION: &str = "fkst.chrono-ai.fun/validation-status";
pub(crate) const VALIDATED_AT_ANNOTATION: &str = "fkst.chrono-ai.fun/validated-at";
pub(crate) const CONTENT_HASH_ANNOTATION: &str = "fkst.chrono-ai.fun/content-hash";
pub(crate) const VALIDATION_IMAGE_ANNOTATION: &str = "fkst.chrono-ai.fun/validation-image";

/// Reserved ConfigMap data key holding the ordered install command list (a JSON
/// array). Dotted so it can never collide with an env KEY (whose regex forbids a
/// leading `.`), yet it is a legal Kubernetes data key (`[-._a-zA-Z0-9]+`).
pub(crate) const INSTALL_KEY: &str = ".install";
/// Reserved ConfigMap data key holding the non-secret variables (a JSON object
/// `{KEY:VALUE}`). Dotted for the same collision-proof reason as [`INSTALL_KEY`].
pub(crate) const VARIABLES_KEY: &str = ".variables";

/// The value written to [`STATUS_ANNOTATION`] once a full write completes. Stamped
/// on the ConfigMap, which the validate-then-swap order persists LAST, so a
/// half-write (Secret written, ConfigMap not) is never observed as ready.
const STATUS_READY: &str = "ready";

/// The deterministic object name for one named environment. Keyed by the
/// immutable numeric GitHub id (logins are renamable) plus the env name.
///
/// The precise DNS-1123 (≤63 char) budget for `<name>` is enforced at the route
/// layer; this function only composes the name.
pub fn env_object_name(id: i64, name: &str) -> String {
    format!("fkst-env-{id}-{name}")
}

/// The `component=user-env,github-user-id=<id>` selector that lists a user's
/// environments by owner.
pub(crate) fn owner_selector(id: i64) -> String {
    format!("{COMPONENT_LABEL}={COMPONENT_VALUE},{USER_ID_LABEL}={id}")
}

/// Common labels stamped on an environment's ConfigMap + Secret. The
/// component + user-id labels are the selector [`owner_selector`] filters on.
pub(crate) fn env_labels(id: i64, login: &str) -> BTreeMap<String, String> {
    BTreeMap::from([
        (PART_OF_LABEL.to_string(), "fkst-hosted".to_string()),
        (COMPONENT_LABEL.to_string(), COMPONENT_VALUE.to_string()),
        (USER_ID_LABEL.to_string(), id.to_string()),
        (LOGIN_LABEL.to_string(), sanitize_label_value(login)),
    ])
}

/// Status annotations stamped on BOTH objects: the raw (un-sanitized) env name,
/// the `ready` marker, and the validation provenance.
pub(crate) fn env_annotations(
    name: &str,
    validated_at: &str,
    content_hash: &str,
    validation_image: &str,
) -> BTreeMap<String, String> {
    BTreeMap::from([
        (ENV_NAME_ANNOTATION.to_string(), name.to_string()),
        (STATUS_ANNOTATION.to_string(), STATUS_READY.to_string()),
        (
            VALIDATED_AT_ANNOTATION.to_string(),
            validated_at.to_string(),
        ),
        (
            CONTENT_HASH_ANNOTATION.to_string(),
            content_hash.to_string(),
        ),
        (
            VALIDATION_IMAGE_ANNOTATION.to_string(),
            validation_image.to_string(),
        ),
    ])
}

/// Coerce an arbitrary string into a valid Kubernetes label *value* (≤63 chars,
/// `[A-Za-z0-9._-]`, alphanumeric ends). GitHub logins are already label-safe,
/// but the login crosses a trust boundary (GitHub's `/user` response), so we fail
/// safe rather than let an odd value error the whole store. An empty result is
/// itself a valid label value. (Ported from `user_store` for self-containment.)
pub(crate) fn sanitize_label_value(value: &str) -> String {
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

// ---- projection helpers (never leak secret values) ------------------------

/// The Secret's data key NAMES, sorted — never the values. (Ported from
/// `user_store`.)
pub(crate) fn secret_key_names(secret: &Secret) -> Vec<String> {
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

/// Decode a Secret's `data` (and any freshly-created `string_data` echo) into a
/// `{ name: value }` map. UNLIKE [`secret_key_names`], this exposes the VALUES, so
/// it is reachable ONLY through `super::load_environment_for_session` — never a
/// route. A value that is not valid UTF-8 is dropped with a warning (env values
/// always originate as UTF-8 strings, so this is purely defensive). The value is
/// NEVER logged. (Ported from `user_store`.)
pub(crate) fn decode_secret_values(secret: &Secret) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    if let Some(data) = secret.data.as_ref() {
        for (k, v) in data {
            match String::from_utf8(v.0.clone()) {
                Ok(value) => {
                    out.insert(k.clone(), value);
                }
                Err(_) => {
                    tracing::warn!(key = %k, "env store: secret value is not valid utf-8; skipped")
                }
            }
        }
    }
    // A just-created Secret may echo `string_data` instead of `data`; fill only
    // the gaps so the persisted `data` always wins.
    if let Some(sd) = secret.string_data.as_ref() {
        for (k, v) in sd {
            out.entry(k.clone()).or_insert_with(|| v.clone());
        }
    }
    out
}

/// Read a single annotation value, or an empty string when absent.
pub(crate) fn annotation(meta: &ObjectMeta, key: &str) -> String {
    meta.annotations
        .as_ref()
        .and_then(|a| a.get(key))
        .cloned()
        .unwrap_or_default()
}

/// Parse the reserved [`INSTALL_KEY`] data value into the ordered command list.
/// A missing or malformed value yields an empty list (fail-soft on read).
pub(crate) fn parse_install(data: &BTreeMap<String, String>) -> Vec<String> {
    data.get(INSTALL_KEY)
        .and_then(|s| serde_json::from_str::<Vec<String>>(s).ok())
        .unwrap_or_default()
}

/// Parse the reserved [`VARIABLES_KEY`] data value into the non-secret variable
/// map. A missing or malformed value yields an empty map (fail-soft on read).
pub(crate) fn parse_variables(data: &BTreeMap<String, String>) -> BTreeMap<String, String> {
    data.get(VARIABLES_KEY)
        .and_then(|s| serde_json::from_str::<BTreeMap<String, String>>(s).ok())
        .unwrap_or_default()
}

/// A stable content hash over an environment's PUBLIC shape: the ordered install
/// commands, the sorted variable map, and the SORTED secret key NAMES — never any
/// secret value. Stamps [`CONTENT_HASH_ANNOTATION`] so an unchanged re-submit is
/// detectable. Order-independent for the maps/keys.
pub fn content_hash(
    install: &[String],
    variables: &BTreeMap<String, String>,
    secret_key_names: &[String],
) -> String {
    #[derive(Serialize)]
    struct Canonical<'a> {
        install: &'a [String],
        variables: &'a BTreeMap<String, String>,
        secret_keys: Vec<String>,
    }
    let mut secret_keys = secret_key_names.to_vec();
    secret_keys.sort();
    secret_keys.dedup();
    let canonical = Canonical {
        install,
        variables,
        secret_keys,
    };
    // Serializing a BTreeMap yields sorted keys, so the JSON is canonical.
    let json = serde_json::to_vec(&canonical).expect("canonical env hash json is infallible");
    let digest = Sha256::digest(&json);
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

// ---- record types (no secret values, ever) --------------------------------

/// The full view of one named environment: its status, install commands,
/// non-secret variables, and secret key NAMES. Carries NO secret values.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvRecord {
    pub name: String,
    pub status: String,
    pub validated_at: String,
    pub install: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
}

/// A compact list-view of one named environment: counts only, no contents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EnvSummary {
    pub name: String,
    pub status: String,
    pub validated_at: String,
    pub install_command_count: usize,
    pub variable_count: usize,
    pub secret_count: usize,
}

#[cfg(test)]
#[path = "env_store_tests.rs"]
mod tests;
