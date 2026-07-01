//! `/api/v1/users/me/env`: the per-user environment + secret store API (PR4a).
//!
//! ONE endpoint, two methods:
//! - `GET /api/v1/users/me/env` — read the caller's variables (values) + secret
//!   key NAMES (secret *values* are never serialized into any response).
//! - `PATCH /api/v1/users/me/env` — apply all mutations in a single body:
//!   `variables` to upsert (non-secret), `secrets` to upsert (write-only), and
//!   `delete` (key names to remove from either store). Returns the updated view.
//!
//! Identity is the GitHub token. Every handler takes the [`GithubUser`]
//! extractor, which trades the request's `Authorization: Bearer <github token>`
//! for the verified `{ login, id }` (see [`crate::github_identity`]). The
//! numeric `id` — and ONLY the verified id, never a request path/body value —
//! selects the `fkst-user-<id>` ConfigMap/Secret, so a caller can read or write
//! their OWN store and no other.
//!
//! Validation: every key must be a valid env var name (`^[A-Za-z_][A-Za-z0-9_]*$`,
//! which is also a valid Kubernetes data key); the per-request entry count is
//! capped by `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP` and each value's byte size
//! by `FKST_HOSTED_VAULT_VALUE_BYTE_CAP`. A violation is `422` via the standard
//! [`ErrorEnvelope`].

use std::collections::BTreeMap;
use std::sync::OnceLock;

use axum::extract::State;
use axum::Json;
use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::Config;
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::k8s::{user_store, KubeClient};
use crate::state::AppState;

/// `PATCH /users/me/env` body — every field is OPTIONAL, so one call can upsert
/// variables, upsert secrets, and/or delete keys. Deletes are applied first,
/// then the upserts, so a key present in both `delete` and a `variables`/`secrets`
/// map ends up SET.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EnvPatchRequest {
    /// Non-secret env variables (`NAME` -> value) to add/overwrite.
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// Secret env variables (`NAME` -> value) to add/overwrite. Write-only —
    /// the values are NEVER echoed back in any response.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
    /// Key names to remove (from the variable store AND the secret store).
    #[serde(default)]
    pub delete: Vec<String>,
}

/// `GET`/`PATCH /users/me/env` response: the non-secret variable values plus the
/// secret key NAMES. Secret values are deliberately absent.
#[derive(Debug, Serialize, ToSchema)]
pub struct UserEnvView {
    pub variables: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
}

/// Anchored env-var-name pattern. Also a valid Kubernetes ConfigMap/Secret data
/// key (which permits `[-._a-zA-Z0-9]+`), so a key that passes here is safe to
/// store unescaped.
fn env_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("static env key regex"))
}

/// True when `key` is a valid env var name.
fn valid_env_key(key: &str) -> bool {
    env_key_regex().is_match(key)
}

/// Reject an invalid env var name with `422`.
fn validate_key(key: &str) -> Result<(), AppError> {
    if valid_env_key(key) {
        Ok(())
    } else {
        Err(AppError::Unprocessable(format!(
            "invalid env var name {key:?}: must match ^[A-Za-z_][A-Za-z0-9_]*$"
        )))
    }
}

/// Validate a whole request map: key shape, per-value byte cap, and entry count
/// cap. All violations render as `422`.
fn validate_entries(entries: &BTreeMap<String, String>, config: &Config) -> Result<(), AppError> {
    if entries.len() > config.vault_entries_per_scope_cap {
        return Err(AppError::Unprocessable(format!(
            "too many entries: {} exceeds the per-user cap of {}",
            entries.len(),
            config.vault_entries_per_scope_cap
        )));
    }
    for (key, value) in entries {
        validate_key(key)?;
        if value.len() > config.vault_value_byte_cap {
            return Err(AppError::Unprocessable(format!(
                "value for {key:?} is {} bytes, exceeding the cap of {}",
                value.len(),
                config.vault_value_byte_cap
            )));
        }
    }
    Ok(())
}

/// Build a Kubernetes client bound to the control-plane namespace the user-store
/// objects live in. The user store is independent of pod-per-session dispatch,
/// so it is NOT gated on `FKST_POD_DISPATCH`; an unreachable cluster surfaces as
/// `503` (a dependency failure), never a leaked client detail.
async fn user_store_client(state: &AppState) -> Result<KubeClient, AppError> {
    KubeClient::from_inferred(&state.config.pod.namespace)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "user store: kubernetes client unavailable");
            AppError::Unavailable("user store backend unavailable".to_string())
        })
}

/// Read the caller's full store view (variable values + secret key names).
async fn read_view(kube: &KubeClient, user_id: i64) -> Result<UserEnvView, AppError> {
    let variables = user_store::get_env(kube, user_id).await?;
    let secret_keys = user_store::get_secret_keys(kube, user_id).await?;
    Ok(UserEnvView {
        variables,
        secret_keys,
    })
}

/// `GET /api/v1/users/me/env` — the caller's variables + secret key names.
#[utoipa::path(
    get,
    path = "/users/me/env",
    tag = "users",
    operation_id = "get_user_env",
    responses(
        (status = 200, description = "The caller's variables and secret key names", body = UserEnvView),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn get_env(
    State(state): State<AppState>,
    user: GithubUser,
) -> Result<Json<UserEnvView>, AppError> {
    let kube = user_store_client(&state).await?;
    Ok(Json(read_view(&kube, user.id).await?))
}

/// `PATCH /api/v1/users/me/env` — upsert variables/secrets and/or delete keys in
/// one call. Returns the updated view.
#[utoipa::path(
    patch,
    path = "/users/me/env",
    tag = "users",
    operation_id = "patch_user_env",
    request_body = EnvPatchRequest,
    responses(
        (status = 200, description = "The updated variables + secret key names", body = UserEnvView),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Invalid key, oversize value, or cap exceeded", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn patch_env(
    State(state): State<AppState>,
    user: GithubUser,
    Json(body): Json<EnvPatchRequest>,
) -> Result<Json<UserEnvView>, AppError> {
    // Validate everything up front (fail-closed before any write).
    validate_entries(&body.variables, &state.config)?;
    validate_entries(&body.secrets, &state.config)?;
    for key in &body.delete {
        validate_key(key)?;
    }

    let cap = state.config.vault_entries_per_scope_cap;
    let kube = user_store_client(&state).await?;

    // Deletes first (from BOTH stores; idempotent), then upserts, so a key in
    // both `delete` and a set map ends up set.
    for key in &body.delete {
        user_store::delete_env_key(&kube, user.id, key).await?;
        user_store::delete_secret_key(&kube, user.id, key).await?;
    }
    if !body.variables.is_empty() {
        user_store::merge_env(&kube, user.id, &user.login, body.variables, cap).await?;
    }
    if !body.secrets.is_empty() {
        user_store::merge_secrets(&kube, user.id, &user.login, body.secrets, cap).await?;
    }
    tracing::info!(github_user_id = user.id, "user store: env patched");

    Ok(Json(read_view(&kube, user.id).await?))
}

/// The user-store router (nested under `/api/v1`). Open at the app layer — the
/// per-request GitHub token IS the auth (the [`GithubUser`] extractor), so no
/// middleware and no documented security scheme. GET + PATCH share one path.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(get_env, patch_env))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> Config {
        Config::default()
    }

    #[test]
    fn valid_env_key_accepts_env_var_names() {
        for key in ["FOO", "_x", "API_KEY", "a1", "MY_VAR_2", "_"] {
            assert!(valid_env_key(key), "must accept {key:?}");
        }
    }

    #[test]
    fn valid_env_key_rejects_non_env_var_names() {
        for key in [
            "", "1FOO", "a-b", "a.b", "a b", "a/b", "FÓO", "MY-VAR", "key=val",
        ] {
            assert!(!valid_env_key(key), "must reject {key:?}");
        }
    }

    #[test]
    fn validate_key_maps_invalid_to_422() {
        assert!(validate_key("GOOD_KEY").is_ok());
        let err = validate_key("bad-key").expect_err("must reject");
        assert!(matches!(err, AppError::Unprocessable(_)));
    }

    #[test]
    fn validate_entries_accepts_within_caps() {
        let mut m = BTreeMap::new();
        m.insert("FOO".to_string(), "bar".to_string());
        assert!(validate_entries(&m, &config()).is_ok());
    }

    #[test]
    fn validate_entries_rejects_a_bad_key() {
        let mut m = BTreeMap::new();
        m.insert("not ok".to_string(), "bar".to_string());
        let err = validate_entries(&m, &config()).expect_err("bad key must fail");
        assert!(matches!(err, AppError::Unprocessable(_)));
    }

    #[test]
    fn validate_entries_rejects_oversize_value() {
        let mut cfg = config();
        cfg.vault_value_byte_cap = 4;
        let mut m = BTreeMap::new();
        m.insert("FOO".to_string(), "toolong".to_string());
        let err = validate_entries(&m, &cfg).expect_err("oversize value must fail");
        assert!(matches!(err, AppError::Unprocessable(_)));
    }

    #[test]
    fn validate_entries_rejects_too_many_entries() {
        let mut cfg = config();
        cfg.vault_entries_per_scope_cap = 1;
        let mut m = BTreeMap::new();
        m.insert("A".to_string(), "1".to_string());
        m.insert("B".to_string(), "2".to_string());
        let err = validate_entries(&m, &cfg).expect_err("over cap must fail");
        assert!(matches!(err, AppError::Unprocessable(_)));
    }

    // ---- request body -------------------------------------------------------

    #[test]
    fn patch_request_deserializes_all_fields() {
        let req: EnvPatchRequest = serde_json::from_value(serde_json::json!({
            "variables": { "FOO": "bar" },
            "secrets": { "TOKEN": "s3cr3t" },
            "delete": ["OLD"]
        }))
        .expect("deserializes");
        assert_eq!(req.variables["FOO"], "bar");
        assert_eq!(req.secrets["TOKEN"], "s3cr3t");
        assert_eq!(req.delete, vec!["OLD".to_string()]);
    }

    #[test]
    fn patch_request_fields_are_optional() {
        // A body with only `variables` leaves `secrets`/`delete` empty (not an
        // error) — one endpoint supports partial mutations.
        let req: EnvPatchRequest =
            serde_json::from_value(serde_json::json!({ "variables": { "FOO": "bar" } }))
                .expect("deserializes");
        assert_eq!(req.variables["FOO"], "bar");
        assert!(req.secrets.is_empty());
        assert!(req.delete.is_empty());

        // An empty body is valid (a no-op patch).
        let empty: EnvPatchRequest =
            serde_json::from_value(serde_json::json!({})).expect("deserializes");
        assert!(empty.variables.is_empty() && empty.secrets.is_empty() && empty.delete.is_empty());
    }

    #[test]
    fn user_env_view_never_carries_secret_values() {
        // The view is variables + secret KEY NAMES; there is no field that could
        // ever hold a secret value. Assert the serialized shape to lock that in.
        let mut variables = BTreeMap::new();
        variables.insert("FOO".to_string(), "bar".to_string());
        let view = UserEnvView {
            variables,
            secret_keys: vec!["TOKEN".to_string()],
        };
        let json = serde_json::to_value(&view).expect("serializes");
        assert_eq!(json["variables"]["FOO"], "bar");
        assert_eq!(json["secret_keys"], serde_json::json!(["TOKEN"]));
        // Exactly two top-level fields — no `secrets`/value leak.
        let obj = json.as_object().expect("object");
        assert_eq!(obj.len(), 2);
        assert!(obj.get("secrets").is_none());
    }
}
