//! `/api/v1/users/me/environments`: the named-environment REST API (issue #338
//! §2.2), replacing the flat per-user env store.
//!
//! Each NAMED environment bundles ordered install commands, non-secret variables,
//! and write-only secrets. The keystone is the `PUT` handler (§2.4/§3.3): before
//! an environment is persisted, its install commands are run once inside a
//! throwaway, hard-isolated validation pod ([`crate::k8s::env_validator`]); it is
//! stored ONLY on a passing verdict, and a failure aborts with a detailed `422`
//! that never touches the store.
//!
//! Every handler takes the [`GithubUser`] extractor, which trades the request's
//! `Authorization: Bearer <github token>` for the verified `{ login, id }`. The
//! numeric `id` — and ONLY the verified id, never a request path/body value —
//! keys the `fkst-env-<id>-<name>` objects, so a caller can only ever touch their
//! OWN environments. Secret VALUES are write-only: no response type carries one
//! ([`EnvironmentView`] exposes secret KEY NAMES), locked by a serialization test.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use k8s_openapi::chrono::Utc;
use regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::config::Config;
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::k8s::env_store::{
    content_hash, count_environments, delete_environment, env_object_name, get_environment,
    list_environments, put_environment, EnvRecord, EnvSummary,
};
use crate::k8s::env_validator::{validate_environment, ValidationOutcome};
use crate::k8s::KubeClient;
use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};
use crate::state::AppState;

/// The maximum length of an environment `name` (before the id/prefix budget).
const MAX_NAME_LEN: usize = 40;
/// The DNS-1123 label budget the composed `fkst-env-<id>-<name>` must fit.
const MAX_OBJECT_NAME_LEN: usize = 63;
/// The status stamped on a fully-written (validated) environment.
const STATUS_READY: &str = "ready";

/// `PUT /users/me/environments/{name}` body: the desired environment contents.
/// Every field defaults to empty so a caller can omit any of them.
#[derive(Debug, Default, Deserialize, ToSchema)]
pub struct EnvironmentSpec {
    /// Ordered install commands validated in the isolated pod before persisting.
    #[serde(default)]
    pub install: Vec<String>,
    /// Non-secret env variables (`NAME` -> value).
    #[serde(default)]
    pub variables: BTreeMap<String, String>,
    /// Secret env variables (`NAME` -> value). Write-only — the values are NEVER
    /// echoed back in any response.
    #[serde(default)]
    pub secrets: BTreeMap<String, String>,
}

/// The full view of one named environment: its install commands, non-secret
/// variables, and secret key NAMES. Secret values are deliberately absent.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentView {
    pub name: String,
    pub status: String,
    pub validated_at: String,
    pub install: Vec<String>,
    pub variables: BTreeMap<String, String>,
    pub secret_keys: Vec<String>,
}

/// A compact list-view of one named environment: counts only, no contents.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentSummary {
    pub name: String,
    pub status: String,
    pub validated_at: String,
    pub install_command_count: u32,
    pub variable_count: u32,
    pub secret_count: u32,
}

/// `GET /users/me/environments` response: the caller's environments as summaries.
#[derive(Debug, Serialize, ToSchema)]
pub struct EnvironmentList {
    pub environments: Vec<EnvironmentSummary>,
}

/// The detailed `422` body returned by `PUT` (and ONLY `PUT`) when the install
/// commands fail validation in the isolated pod. Nothing is persisted.
#[derive(Debug, Serialize, ToSchema)]
pub struct InstallValidationError {
    /// Fixed machine-readable code: `install_validation_failed`.
    pub error: String,
    /// Human-readable summary (which command failed, or the timed-out message).
    pub message: String,
    /// Zero-based index of the failing command (0 when the run timed out).
    pub failed_command_index: u32,
    /// The exact command that failed (empty when the run timed out).
    pub failed_command: String,
    /// The command's exit code (`-1` when unknown / timed out).
    pub exit_code: i32,
    /// Whether the sequence exceeded the validation deadline.
    pub timed_out: bool,
    /// Trailing bytes of the failing command's stderr (bounded by config).
    pub stderr_tail: String,
}

/// Anchored env-var-name pattern (also a valid Kubernetes data key). Ported from
/// the flat user-env store the named-environment API replaces.
fn env_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("static env key regex"))
}

/// Anchored environment-NAME pattern: DNS-1123-label-ish, lower-case only, so the
/// composed object name is a valid Kubernetes object name.
fn env_name_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[a-z0-9]([a-z0-9-]*[a-z0-9])?$").expect("static env name regex"))
}

/// True when `key` is a valid env var name.
fn valid_env_key(key: &str) -> bool {
    env_key_regex().is_match(key)
}

/// Reject an invalid or reserved env var name with `422`. A user may not shadow a
/// platform-owned var (the whole `FKST_*` / git-credential family, via
/// [`is_reserved_env_key`]) or the engine's `LLM_API_KEY` credential slot.
fn validate_key(key: &str) -> Result<(), AppError> {
    if !valid_env_key(key) {
        return Err(AppError::Unprocessable(format!(
            "invalid env var name {key:?}: must match ^[A-Za-z_][A-Za-z0-9_]*$"
        )));
    }
    if is_reserved_env_key(key) || key == LLM_ENV_KEY {
        return Err(AppError::Unprocessable(format!(
            "env var name {key:?} is reserved and cannot be set"
        )));
    }
    Ok(())
}

/// Validate a variable/secret map: key shape (+ reserved-key rejection), per-value
/// byte cap, and per-scope entry count cap. All violations render as `422`.
fn validate_entries(entries: &BTreeMap<String, String>, config: &Config) -> Result<(), AppError> {
    if entries.len() > config.vault_entries_per_scope_cap {
        return Err(AppError::Unprocessable(format!(
            "too many entries: {} exceeds the per-scope cap of {}",
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

/// Validate the environment name: shape, length, and the composed-object-name
/// budget so `fkst-env-<id>-<name>` stays within the 63-char DNS-1123 limit.
fn validate_name(name: &str, id: i64) -> Result<(), AppError> {
    if name.len() > MAX_NAME_LEN || !env_name_regex().is_match(name) {
        return Err(AppError::Unprocessable(format!(
            "invalid environment name {name:?}: must match \
             ^[a-z0-9]([a-z0-9-]*[a-z0-9])?$ and be 1..={MAX_NAME_LEN} chars"
        )));
    }
    let object_len = env_object_name(id, name).len();
    if object_len > MAX_OBJECT_NAME_LEN {
        return Err(AppError::Unprocessable(format!(
            "environment name {name:?} is too long for this user: the derived \
             object name is {object_len} chars (max {MAX_OBJECT_NAME_LEN})"
        )));
    }
    Ok(())
}

/// Validate the install list: count 1..=cap, each command non-blank-trimmed and
/// within the per-command byte cap. All violations render as `422`.
fn validate_install(install: &[String], config: &Config) -> Result<(), AppError> {
    let count = install.len();
    if count < 1 || count > config.env.install_max_commands {
        return Err(AppError::Unprocessable(format!(
            "install must list between 1 and {} commands (got {count})",
            config.env.install_max_commands
        )));
    }
    for (i, command) in install.iter().enumerate() {
        if command.trim().is_empty() {
            return Err(AppError::Unprocessable(format!(
                "install command {} must not be blank",
                i + 1
            )));
        }
        if command.len() > config.env.install_max_command_bytes {
            return Err(AppError::Unprocessable(format!(
                "install command {} is {} bytes, exceeding the cap of {}",
                i + 1,
                command.len(),
                config.env.install_max_command_bytes
            )));
        }
    }
    Ok(())
}

/// Project a stored [`EnvRecord`] into the public view (never a secret value).
fn view_from_record(record: EnvRecord) -> EnvironmentView {
    EnvironmentView {
        name: record.name,
        status: record.status,
        validated_at: record.validated_at,
        install: record.install,
        variables: record.variables,
        secret_keys: record.secret_keys,
    }
}

/// Project a stored [`EnvSummary`] into the public summary (counts only).
fn summary_from_record(summary: EnvSummary) -> EnvironmentSummary {
    EnvironmentSummary {
        name: summary.name,
        status: summary.status,
        validated_at: summary.validated_at,
        install_command_count: u32::try_from(summary.install_command_count).unwrap_or(u32::MAX),
        variable_count: u32::try_from(summary.variable_count).unwrap_or(u32::MAX),
        secret_count: u32::try_from(summary.secret_count).unwrap_or(u32::MAX),
    }
}

/// Build a Kubernetes client bound to the control-plane namespace the environment
/// objects live in. An unreachable cluster surfaces as `503`, never a leaked
/// client detail.
async fn env_store_client(state: &AppState) -> Result<KubeClient, AppError> {
    KubeClient::from_inferred(&state.config.pod.namespace)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "env store: kubernetes client unavailable");
            AppError::Unavailable("environment store backend unavailable".to_string())
        })
}

/// `PUT /api/v1/users/me/environments/{name}` — validate the install commands in
/// an isolated pod and persist the environment ONLY on success.
#[utoipa::path(
    put,
    path = "/users/me/environments/{name}",
    tag = "users",
    operation_id = "put_user_environment",
    params(("name" = String, Path, description = "Environment name (^[a-z0-9]([a-z0-9-]*[a-z0-9])?$, 1..=40 chars)")),
    request_body = EnvironmentSpec,
    responses(
        (status = 200, description = "Environment replaced (or unchanged) and validated", body = EnvironmentView),
        (status = 201, description = "Environment created and validated", body = EnvironmentView),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 422, description = "Install validation failed (detailed report)", body = InstallValidationError),
        (status = 429, description = "Validation capacity busy, or the same env is already validating", body = ErrorEnvelope),
        (status = 503, description = "Environment store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn put_user_environment(
    State(state): State<AppState>,
    Path(name): Path<String>,
    user: GithubUser,
    Json(spec): Json<EnvironmentSpec>,
) -> Result<Response, AppError> {
    let config = &state.config;

    // 1. Fail-closed validation BEFORE any cluster call.
    validate_name(&name, user.id)?;
    validate_install(&spec.install, config)?;
    validate_entries(&spec.variables, config)?;
    validate_entries(&spec.secrets, config)?;

    let kube = env_store_client(&state).await?;

    // The sorted secret key NAMES feed the content hash + the view (never values).
    let secret_key_names: Vec<String> = spec.secrets.keys().cloned().collect();
    let new_hash = content_hash(&spec.install, &spec.variables, &secret_key_names);

    // Existing env drives replace-detection, the per-user cap, and idempotency.
    let existing = get_environment(&kube, user.id, &name).await?;

    // 2. Per-user cap: only a NEW name may push the owner over the ceiling; a
    //    replace of an existing env is always allowed (it does not grow the set).
    if existing.is_none() {
        let count = count_environments(&kube, user.id).await?;
        if count >= config.env.max_per_user {
            return Err(AppError::Unprocessable(format!(
                "environment cap reached: at most {} named environments per user",
                config.env.max_per_user
            )));
        }
    }

    // 3. Idempotency short-circuit: an unchanged, already-`ready` env is returned
    //    as-is WITHOUT re-running the (expensive) validation pod. Recomputing the
    //    hash from the record equals the stored content-hash annotation by
    //    construction, so we compare without exposing the annotation.
    if let Some(record) = &existing {
        if record.status == STATUS_READY
            && content_hash(&record.install, &record.variables, &record.secret_keys) == new_hash
        {
            tracing::info!(github_user_id = user.id, env = %name, "env put: unchanged; skipping re-validation");
            return Ok((StatusCode::OK, Json(view_from_record(record.clone()))).into_response());
        }
    }

    // 4. Validate in the isolated pod. Err (429 capacity / infra) propagates; a
    //    Failed verdict persists NOTHING and renders the detailed 422; only a
    //    Passed verdict continues to persistence.
    let command_count = spec.install.len();
    match validate_environment(
        &kube,
        config,
        user.id,
        &user.login,
        &name,
        &spec.install,
        &spec.variables,
    )
    .await?
    {
        ValidationOutcome::Passed { commands } => {
            tracing::info!(github_user_id = user.id, env = %name, commands, "env put: install validation passed");
        }
        ValidationOutcome::Failed {
            failed_command_index,
            failed_command,
            exit_code,
            timed_out,
            stderr_tail,
        } => {
            let message = if timed_out {
                "install validation timed out before completing".to_string()
            } else {
                format!("install command {failed_command_index} of {command_count} failed")
            };
            tracing::info!(github_user_id = user.id, env = %name, timed_out, "env put: install validation failed; nothing persisted");
            let body = InstallValidationError {
                error: "install_validation_failed".to_string(),
                message,
                failed_command_index,
                failed_command,
                exit_code,
                timed_out,
                stderr_tail,
            };
            return Ok((StatusCode::UNPROCESSABLE_ENTITY, Json(body)).into_response());
        }
    }

    // 5. Persist (validate-then-swap inside put_environment). `validated_at` is a
    //    fresh RFC3339 stamp; the validation image records provenance.
    let validated_at = Utc::now().to_rfc3339();
    let image = config.pod.image.clone().unwrap_or_default();
    put_environment(
        &kube,
        user.id,
        &user.login,
        &name,
        &spec.install,
        &spec.variables,
        &spec.secrets,
        &validated_at,
        &new_hash,
        &image,
    )
    .await?;

    let replaced = existing.is_some();
    let status = if replaced {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    tracing::info!(github_user_id = user.id, env = %name, replaced, "env put: validated and persisted");
    let view = EnvironmentView {
        name,
        status: STATUS_READY.to_string(),
        validated_at,
        install: spec.install,
        variables: spec.variables,
        secret_keys: secret_key_names,
    };
    Ok((status, Json(view)).into_response())
}

/// `GET /api/v1/users/me/environments` — the caller's environments as summaries.
#[utoipa::path(
    get,
    path = "/users/me/environments",
    tag = "users",
    operation_id = "list_user_environments",
    responses(
        (status = 200, description = "The caller's named environments (summaries)", body = EnvironmentList),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 503, description = "Environment store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn list_user_environments(
    State(state): State<AppState>,
    user: GithubUser,
) -> Result<Json<EnvironmentList>, AppError> {
    let kube = env_store_client(&state).await?;
    let summaries = list_environments(&kube, user.id).await?;
    let environments = summaries.into_iter().map(summary_from_record).collect();
    Ok(Json(EnvironmentList { environments }))
}

/// `GET /api/v1/users/me/environments/{name}` — one environment (no secret values).
#[utoipa::path(
    get,
    path = "/users/me/environments/{name}",
    tag = "users",
    operation_id = "get_user_environment",
    params(("name" = String, Path, description = "The environment name")),
    responses(
        (status = 200, description = "The named environment (secret values omitted)", body = EnvironmentView),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 404, description = "No environment by that name", body = ErrorEnvelope),
        (status = 503, description = "Environment store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn get_user_environment(
    State(state): State<AppState>,
    Path(name): Path<String>,
    user: GithubUser,
) -> Result<Json<EnvironmentView>, AppError> {
    let kube = env_store_client(&state).await?;
    match get_environment(&kube, user.id, &name).await? {
        Some(record) => Ok(Json(view_from_record(record))),
        None => Err(AppError::NotFound(format!("no environment named {name:?}"))),
    }
}

/// `DELETE /api/v1/users/me/environments/{name}` — remove an environment. Idempotent:
/// `204` whether or not it existed.
#[utoipa::path(
    delete,
    path = "/users/me/environments/{name}",
    tag = "users",
    operation_id = "delete_user_environment",
    params(("name" = String, Path, description = "The environment name")),
    responses(
        (status = 204, description = "Deleted (idempotent — 204 even if absent)"),
        (status = 401, description = "Missing/invalid GitHub token", body = ErrorEnvelope),
        (status = 503, description = "Environment store backend unavailable", body = ErrorEnvelope),
    )
)]
async fn delete_user_environment(
    State(state): State<AppState>,
    Path(name): Path<String>,
    user: GithubUser,
) -> Result<StatusCode, AppError> {
    let kube = env_store_client(&state).await?;
    let existed = delete_environment(&kube, user.id, &name).await?;
    tracing::info!(github_user_id = user.id, env = %name, existed, "env delete");
    Ok(StatusCode::NO_CONTENT)
}

/// The named-environment router (nested under `/api/v1`). Open at the app layer —
/// the per-request GitHub token IS the auth (the [`GithubUser`] extractor), so no
/// middleware and no documented security scheme. PUT/GET/DELETE on `{name}` share
/// one `routes!` group; the collection GET is its own.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(list_user_environments))
        .routes(routes!(
            put_user_environment,
            get_user_environment,
            delete_user_environment
        ))
}

#[cfg(test)]
#[path = "environments_tests.rs"]
mod tests;
