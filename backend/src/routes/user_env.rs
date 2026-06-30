//! `/api/v1/users/me/*`: the per-user environment + secret store API (PR4a).
//!
//! Identity is the GitHub token. Every handler takes the [`GithubUser`]
//! extractor, which trades the request's `Authorization: Bearer <github token>`
//! for the verified `{ login, id }` (see [`crate::github_identity`]). The
//! numeric `id` — and ONLY the verified id, never a request path/body value —
//! selects the `fkst-user-<id>` ConfigMap/Secret, so a caller can read or write
//! their OWN store and no other.
//!
//! Secret values are write-only at this layer: a `PUT .../secrets` returns the
//! resulting key NAMES, and `GET .../env` returns the variable values plus the
//! secret key names — the secret *values* are never serialized into any
//! response.
//!
//! Validation: every key must be a valid env var name (`^[A-Za-z_][A-Za-z0-9_]*$`,
//! which is also a valid Kubernetes data key); the per-request entry count is
//! capped by `FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP` and each value's byte size
//! by `FKST_HOSTED_VAULT_VALUE_BYTE_CAP` (reused from the existing vault caps).
//! A violation is `422` via the standard [`ErrorEnvelope`].

use std::collections::BTreeMap;
use std::sync::OnceLock;

use axum::extract::{Path, State};
use axum::http::StatusCode;
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

/// `PUT /users/me/env` body: variables to merge-upsert.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PutEnvRequest {
    /// Non-secret env variables (`NAME` -> value) to add/overwrite.
    pub variables: BTreeMap<String, String>,
}

/// The resulting non-secret variable map (returned by `PUT`/`GET`).
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvVariablesResponse {
    pub variables: BTreeMap<String, String>,
}

/// `PUT /users/me/secrets` body: secrets to merge-upsert. Values are
/// write-only — they are NEVER echoed back in any response.
#[derive(Debug, Deserialize, ToSchema)]
pub struct PutSecretsRequest {
    /// Secret env variables (`NAME` -> value) to add/overwrite.
    pub secrets: BTreeMap<String, String>,
}

/// The NAMES of the user's secret keys (never the values).
#[derive(Debug, Serialize, ToSchema)]
pub struct SecretKeysResponse {
    pub secret_keys: Vec<String>,
}

/// `GET /users/me/env` view: the non-secret variable values plus the secret key
/// NAMES. Secret values are deliberately absent.
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
    let variables = user_store::get_env(&kube, user.id).await?;
    let secret_keys = user_store::get_secret_keys(&kube, user.id).await?;
    Ok(Json(UserEnvView {
        variables,
        secret_keys,
    }))
}

/// `PUT /api/v1/users/me/env` — merge-upsert the caller's variables.
#[utoipa::path(
    put,
    path = "/users/me/env",
    tag = "users",
    operation_id = "put_user_env",
    request_body = PutEnvRequest,
    responses(
        (status = 200, description = "The resulting variable map", body = EnvVariablesResponse),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Invalid key, oversize value, or cap exceeded", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn put_env(
    State(state): State<AppState>,
    user: GithubUser,
    Json(body): Json<PutEnvRequest>,
) -> Result<Json<EnvVariablesResponse>, AppError> {
    validate_entries(&body.variables, &state.config)?;
    let kube = user_store_client(&state).await?;
    let variables = user_store::merge_env(
        &kube,
        user.id,
        &user.login,
        body.variables,
        state.config.vault_entries_per_scope_cap,
    )
    .await?;
    tracing::info!(github_user_id = user.id, "user store: env upserted");
    Ok(Json(EnvVariablesResponse { variables }))
}

/// `PUT /api/v1/users/me/secrets` — merge-upsert the caller's secrets.
#[utoipa::path(
    put,
    path = "/users/me/secrets",
    tag = "users",
    operation_id = "put_user_secrets",
    request_body = PutSecretsRequest,
    responses(
        (status = 200, description = "The resulting secret key names (never values)", body = SecretKeysResponse),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Invalid key, oversize value, or cap exceeded", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn put_secrets(
    State(state): State<AppState>,
    user: GithubUser,
    Json(body): Json<PutSecretsRequest>,
) -> Result<Json<SecretKeysResponse>, AppError> {
    validate_entries(&body.secrets, &state.config)?;
    let kube = user_store_client(&state).await?;
    let secret_keys = user_store::merge_secrets(
        &kube,
        user.id,
        &user.login,
        body.secrets,
        state.config.vault_entries_per_scope_cap,
    )
    .await?;
    tracing::info!(github_user_id = user.id, "user store: secrets upserted");
    Ok(Json(SecretKeysResponse { secret_keys }))
}

/// `DELETE /api/v1/users/me/env/{key}` — remove one variable (idempotent).
///
/// A missing object or missing key is treated as already-gone and still returns
/// `204` — the operation is safe to retry (e.g. on a redelivered request).
#[utoipa::path(
    delete,
    path = "/users/me/env/{key}",
    tag = "users",
    operation_id = "delete_user_env_key",
    params(
        ("key" = String, Path, description = "Env var name to remove"),
    ),
    responses(
        (status = 204, description = "Key removed or already absent (idempotent)"),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Invalid key", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn delete_env(
    State(state): State<AppState>,
    user: GithubUser,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    validate_key(&key)?;
    let kube = user_store_client(&state).await?;
    let removed = user_store::delete_env_key(&kube, user.id, &key).await?;
    tracing::info!(
        github_user_id = user.id,
        removed,
        "user store: env key delete"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// `DELETE /api/v1/users/me/secrets/{key}` — remove one secret (idempotent).
#[utoipa::path(
    delete,
    path = "/users/me/secrets/{key}",
    tag = "users",
    operation_id = "delete_user_secret_key",
    params(
        ("key" = String, Path, description = "Secret env var name to remove"),
    ),
    responses(
        (status = 204, description = "Key removed or already absent (idempotent)"),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Invalid key", body = ErrorEnvelope),
        (status = 503, description = "User store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn delete_secrets(
    State(state): State<AppState>,
    user: GithubUser,
    Path(key): Path<String>,
) -> Result<StatusCode, AppError> {
    validate_key(&key)?;
    let kube = user_store_client(&state).await?;
    let removed = user_store::delete_secret_key(&kube, user.id, &key).await?;
    tracing::info!(
        github_user_id = user.id,
        removed,
        "user store: secret key delete"
    );
    Ok(StatusCode::NO_CONTENT)
}

/// The user-store router (nested under `/api/v1`). Open at the app layer — the
/// per-request GitHub token IS the auth (the [`GithubUser`] extractor), so no
/// middleware and no documented security scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        // GET + PUT share the same path, so they group into one `routes!`.
        .routes(routes!(get_env, put_env))
        .routes(routes!(put_secrets))
        .routes(routes!(delete_env))
        .routes(routes!(delete_secrets))
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

    // ---- DTO round-trips -------------------------------------------------------

    #[test]
    fn put_env_request_deserializes_from_variables_object() {
        let req: PutEnvRequest =
            serde_json::from_value(serde_json::json!({ "variables": { "FOO": "bar" } }))
                .expect("deserializes");
        assert_eq!(req.variables["FOO"], "bar");
    }

    #[test]
    fn put_secrets_request_deserializes_from_secrets_object() {
        let req: PutSecretsRequest =
            serde_json::from_value(serde_json::json!({ "secrets": { "TOKEN": "s3cr3t" } }))
                .expect("deserializes");
        assert_eq!(req.secrets["TOKEN"], "s3cr3t");
    }

    #[test]
    fn env_variables_response_serializes_under_variables() {
        let mut variables = BTreeMap::new();
        variables.insert("FOO".to_string(), "bar".to_string());
        let json = serde_json::to_value(EnvVariablesResponse { variables }).expect("serializes");
        assert_eq!(json["variables"]["FOO"], "bar");
    }

    #[test]
    fn secret_keys_response_serializes_names_only() {
        let json = serde_json::to_value(SecretKeysResponse {
            secret_keys: vec!["TOKEN".to_string(), "API_KEY".to_string()],
        })
        .expect("serializes");
        assert_eq!(json["secret_keys"], serde_json::json!(["TOKEN", "API_KEY"]));
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
