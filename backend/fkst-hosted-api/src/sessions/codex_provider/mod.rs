//! Per-session codex LLM-provider selection and `config.toml` rendering (issue
//! #112).
//!
//! The fkst-substrate engine runs `codex exec` as its reasoning agent, and
//! codex reads its provider from `$CODEX_HOME/config.toml`. codex does NOT
//! interpolate `${VAR}` in that file — `base_url`/`wire_api`/`model` are
//! literal, and the only env indirection is `env_key` (the env var holding the
//! API key, sent as `Authorization: Bearer`). So fkst-hosted RENDERS the toml
//! per session here, around the engine, with no engine change.
//!
//! Two responsibilities, split so each is independently testable:
//! - [`resolve_provider_choice`] reads the user's codex settings from the vault
//!   (#100) and decides the provider LAYER (precedence RAW > STRUCTURED >
//!   DEFAULT). The DEFAULT (NyxID-proxied `chrono-llm`) requires the user to
//!   have connected `chrono-llm` on NyxID; that precondition is verified
//!   through the [`ChronoLlmCheck`] seam so the 422 mapping is unit-testable
//!   without a live NyxID account.
//! - [`render_codex_config`] emits the exact `config.toml` body for the chosen
//!   layer. The operator-pinned chrono-llm DEFAULT `model`/`base_url` are
//!   injected (from config), never literal placeholders.
//!
//! Secret hygiene (load-bearing): the provider API key is NEVER embedded in the
//! rendered toml — it rides the `env_key` env var (the chrono-llm DEFAULT uses
//! `NYXID_ACCESS_TOKEN` from #111; a structured/raw override uses the user's
//! own vault secret, injected by #102). This module only ever handles the
//! `env_key` NAME, never the key value, and never logs vault values.

use async_trait::async_trait;
use secrecy::ExposeSecret;

use crate::error::AppError;
use crate::vault::{EnvScopeRef, ResolvedEntry, VaultService};

/// Vault key holding a complete `config.toml` for the RAW override layer. A
/// non-secret `EnvKind::Variable` (it is config, not a credential — codex's
/// secret is its API key, stored separately under the user's `env_key`).
pub const RAW_CONFIG_KEY: &str = "CODEX_CONFIG_TOML";

/// Vault keys forming the STRUCTURED override layer (all `EnvKind::Variable`).
/// Presence of all four (and absence of [`RAW_CONFIG_KEY`]) selects the
/// structured provider; the API-key SECRET lives separately under the value of
/// `CODEX_ENV_KEY` and is injected by #102 — this module only reads the NAME.
pub const STRUCTURED_BASE_URL_KEY: &str = "CODEX_BASE_URL";
pub const STRUCTURED_MODEL_KEY: &str = "CODEX_MODEL";
pub const STRUCTURED_WIRE_API_KEY: &str = "CODEX_WIRE_API";
pub const STRUCTURED_ENV_KEY_KEY: &str = "CODEX_ENV_KEY";

/// `model_provider` id for a STRUCTURED override. A fixed, non-user-derived
/// token: the user does not name their provider, they only supply its fields,
/// so a stable id keeps the rendered `[model_providers.<id>]` table simple and
/// collision-free with the DEFAULT's `chrono-llm` id.
pub const STRUCTURED_PROVIDER_ID: &str = "user";

/// `model_provider` id and provider name for the chrono-llm DEFAULT.
pub const CHRONO_LLM_PROVIDER_ID: &str = "chrono-llm";

/// NyxID service slug for the chrono-llm proxy, used by the connection
/// precondition check on the DEFAULT path.
pub const CHRONO_LLM_SERVICE_SLUG: &str = "chrono-llm";

/// env_key for the chrono-llm DEFAULT: the per-session NyxID user token (#111),
/// sent as `Authorization: Bearer` to the NyxID proxy.
pub const DEFAULT_ENV_KEY: &str = "NYXID_ACCESS_TOKEN";

/// wire_api for the chrono-llm DEFAULT: the OpenAI Responses API the proxy
/// exposes.
pub const DEFAULT_WIRE_API: &str = "responses";

/// A structured provider, sourced from vault *variables*. The API key rides
/// `env_key` as a vault *secret* (injected by #102) — never embedded here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StructuredProvider {
    /// `[model_providers.<id>]` table id / `model_provider` value.
    pub provider_id: String,
    pub model: String,
    pub base_url: String,
    /// Typically `"responses"`.
    pub wire_api: String,
    /// The env var name fkst-hosted injects the user's key into.
    pub env_key: String,
}

/// Resolved provider layer (precedence: Raw > Structured > DefaultChronoLlm).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderChoice {
    /// A full `config.toml`, written verbatim.
    Raw(String),
    /// The user's OpenAI-compatible provider.
    Structured(StructuredProvider),
    /// The NyxID-proxied `chrono-llm` default.
    DefaultChronoLlm,
}

/// Precondition seam for the chrono-llm DEFAULT path: has the session user
/// connected `chrono-llm` on NyxID (so the proxy has a credential to inject)?
///
/// A trait so production wires a real NyxID `service list` preflight while CI
/// fakes a connected/not-connected state — keeping the missing-connection ->
/// 422 mapping unit-testable without a live account (issue #112 test posture).
#[async_trait]
pub trait ChronoLlmCheck: Send + Sync {
    /// `Ok(true)` when the user has connected chrono-llm; `Ok(false)` when not
    /// (the caller maps that to a 422); `Err` for a transient lookup failure.
    async fn is_chrono_llm_connected(
        &self,
        owner_user_id: &str,
        org_id: Option<&str>,
    ) -> Result<bool, AppError>;
}

/// A check that always reports chrono-llm connected. The v1 production default:
/// the genuine connection state is validated by the documented manual/staging
/// preflight (a real NyxID account is required, which CI does not have), so the
/// online path does not gate every session start on an extra NyxID round-trip.
/// The trait seam keeps the 422 mapping exercised by tests and lets a future
/// issue swap in a live `service list` preflight without touching callers.
#[derive(Debug, Default, Clone, Copy)]
pub struct AssumeConnected;

#[async_trait]
impl ChronoLlmCheck for AssumeConnected {
    async fn is_chrono_llm_connected(
        &self,
        _owner_user_id: &str,
        _org_id: Option<&str>,
    ) -> Result<bool, AppError> {
        Ok(true)
    }
}

/// Read the user's codex settings from the vault and pick the provider layer.
///
/// Reads [`VaultService::list_for_scope`] for the session's scope (#100/#102),
/// then applies precedence:
/// 1. RAW — a [`RAW_CONFIG_KEY`] entry holding a full `config.toml`.
/// 2. STRUCTURED — all four [`STRUCTURED_BASE_URL_KEY`]/`MODEL`/`WIRE_API`/
///    `ENV_KEY` entries present (and no RAW).
/// 3. DEFAULT — `chrono-llm`. Selected when no codex entries are stored; the
///    `check` seam verifies the user has connected chrono-llm on NyxID, else
///    [`AppError::Unprocessable`] (422 — there is no `FailedPrecondition`
///    variant; `Unprocessable` is the canonical "dependent resource missing").
///
/// Vault VALUES are exposed only to build the choice and are NEVER logged.
pub async fn resolve_provider_choice(
    vault: &VaultService,
    owner_user_id: &str,
    org_id: Option<&str>,
    scope: &EnvScopeRef,
    check: &dyn ChronoLlmCheck,
) -> Result<ProviderChoice, AppError> {
    let resolved = vault.list_for_scope(owner_user_id, org_id, scope).await?;
    resolve_from_entries(&resolved, owner_user_id, org_id, check).await
}

/// The vault-agnostic core of [`resolve_provider_choice`]: classify the
/// already-resolved entries, then enforce the chrono-llm precondition on the
/// DEFAULT path. Split out from the `list_for_scope` fetch so the layer
/// precedence and the missing-connection -> 422 mapping are unit-testable
/// against canned entries + a [`ChronoLlmCheck`] fake, with no live vault.
async fn resolve_from_entries(
    resolved: &[ResolvedEntry],
    owner_user_id: &str,
    org_id: Option<&str>,
    check: &dyn ChronoLlmCheck,
) -> Result<ProviderChoice, AppError> {
    let choice = classify_entries(resolved);

    // The DEFAULT path needs a connected chrono-llm credential for NyxID to
    // inject; a structured/raw override carries its own key, so no check there.
    if matches!(choice, ProviderChoice::DefaultChronoLlm)
        && !check.is_chrono_llm_connected(owner_user_id, org_id).await?
    {
        return Err(AppError::Unprocessable(format!(
            "the default codex provider requires a connected '{CHRONO_LLM_SERVICE_SLUG}' \
             service on NyxID; connect it or configure a custom codex provider"
        )));
    }

    // The chosen LAYER is non-secret and safe to log; values are not logged.
    tracing::debug!(
        layer = choice_layer_name(&choice),
        "codex provider layer resolved"
    );
    Ok(choice)
}

/// Decide the provider layer from the resolved vault entries (no I/O, no
/// connection check) — the pure core of [`resolve_provider_choice`], split out
/// so the precedence rules are unit-testable against canned entries.
fn classify_entries(resolved: &[ResolvedEntry]) -> ProviderChoice {
    // RAW wins: a full config.toml shadows everything below it.
    if let Some(raw) = lookup(resolved, RAW_CONFIG_KEY) {
        return ProviderChoice::Raw(raw);
    }

    // STRUCTURED requires all four fields; a partial set falls through to the
    // DEFAULT rather than rendering a broken provider table.
    if let (Some(base_url), Some(model), Some(wire_api), Some(env_key)) = (
        lookup(resolved, STRUCTURED_BASE_URL_KEY),
        lookup(resolved, STRUCTURED_MODEL_KEY),
        lookup(resolved, STRUCTURED_WIRE_API_KEY),
        lookup(resolved, STRUCTURED_ENV_KEY_KEY),
    ) {
        return ProviderChoice::Structured(StructuredProvider {
            provider_id: STRUCTURED_PROVIDER_ID.to_string(),
            model,
            base_url,
            wire_api,
            env_key,
        });
    }

    ProviderChoice::DefaultChronoLlm
}

/// Find a resolved entry by key and expose its value as an owned `String`.
/// The value is config (a codex setting), not a credential, but it still comes
/// back as a `SecretString` from the vault resolve API, so it is exposed here
/// purely to render the toml and is never logged.
fn lookup(resolved: &[ResolvedEntry], key: &str) -> Option<String> {
    resolved
        .iter()
        .find(|entry| entry.key == key)
        .map(|entry| entry.value.expose_secret().to_string())
}

/// Non-secret name of the chosen layer, for diagnostics.
fn choice_layer_name(choice: &ProviderChoice) -> &'static str {
    match choice {
        ProviderChoice::Raw(_) => "raw",
        ProviderChoice::Structured(_) => "structured",
        ProviderChoice::DefaultChronoLlm => "default-chrono-llm",
    }
}

/// Render the codex `config.toml` body for the chosen layer.
///
/// `default_model` and `default_base_url` are the operator-pinned values for
/// the chrono-llm DEFAULT (config: `FKST_HOSTED_CODEX_MODEL` /
/// `FKST_HOSTED_CHRONO_LLM_BASE_URL`); they are unused for Raw/Structured.
///
/// - RAW -> the stored string verbatim.
/// - STRUCTURED -> `model_provider`/`model` + a `[model_providers.<id>]` table.
///   `disable_response_storage` is NOT emitted — the user's own provider owns
///   its storage semantics.
/// - DEFAULT -> `chrono-llm` with `disable_response_storage = true` (the proxy
///   is stateless for the session user — DEFAULT-ONLY) and
///   `env_key = "NYXID_ACCESS_TOKEN"` (the per-session user token, #111).
pub fn render_codex_config(
    choice: &ProviderChoice,
    default_model: &str,
    default_base_url: &str,
) -> Result<String, AppError> {
    match choice {
        ProviderChoice::Raw(toml) => Ok(toml.clone()),
        ProviderChoice::Structured(provider) => Ok(render_structured(provider)),
        ProviderChoice::DefaultChronoLlm => Ok(render_default(default_model, default_base_url)),
    }
}

/// Render the STRUCTURED override toml. No `disable_response_storage`.
fn render_structured(provider: &StructuredProvider) -> String {
    let StructuredProvider {
        provider_id,
        model,
        base_url,
        wire_api,
        env_key,
    } = provider;
    format!(
        "model_provider = \"{provider_id}\"\n\
         model = \"{model}\"\n\
         \n\
         [model_providers.{provider_id}]\n\
         name = \"{provider_id}\"\n\
         base_url = \"{base_url}\"\n\
         wire_api = \"{wire_api}\"\n\
         env_key = \"{env_key}\"\n"
    )
}

/// Render the chrono-llm DEFAULT toml. `disable_response_storage = true` is
/// emitted ONLY here.
fn render_default(default_model: &str, default_base_url: &str) -> String {
    format!(
        "model_provider = \"{CHRONO_LLM_PROVIDER_ID}\"\n\
         model = \"{default_model}\"\n\
         disable_response_storage = true\n\
         \n\
         [model_providers.{CHRONO_LLM_PROVIDER_ID}]\n\
         name = \"{CHRONO_LLM_PROVIDER_ID}\"\n\
         base_url = \"{default_base_url}\"\n\
         wire_api = \"{DEFAULT_WIRE_API}\"\n\
         env_key = \"{DEFAULT_ENV_KEY}\"\n"
    )
}

#[cfg(test)]
mod tests;
