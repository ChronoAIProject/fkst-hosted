//! Fail-closed configuration for the chat concierge (`FKST_CHAT_*`).
//!
//! The whole feature is dark by default: [`from_vars`] returns `Ok(None)` unless
//! `FKST_CHAT_ENABLED=true`, mirroring how [`crate::osb_config::from_vars`] skips
//! its whole block when the backend it configures is not selected. That way a
//! half-staged chat block can never fail an unrelated deploy.
//!
//! Every knob falls back to the session-engine `FKST_LLM_*` value where one
//! exists, so the common case ("chat talks to the same provider the sessions
//! do") needs no extra configuration — while a deployment that wants chat on a
//! cheaper/faster model overrides only what differs.
//!
//! Recognized variables:
//!
//! | variable | default | notes |
//! |---|---|---|
//! | `FKST_CHAT_ENABLED` | `false` | master switch; everything below is only read when true |
//! | `FKST_CHAT_BASE_URL` | `FKST_LLM_BASE_URL` | OpenAI-compatible base; `/chat/completions` is appended |
//! | `FKST_CHAT_API_KEY` | `FKST_LLM_API_KEY` | bearer credential; never logged |
//! | `FKST_CHAT_MODEL` | `FKST_LLM_MODEL` | model id sent on every turn |
//! | `FKST_CHAT_MAX_TOOL_ITERATIONS` | `8` | ≥ 1; bounds the model↔tools loop |
//! | `FKST_CHAT_TURN_DEADLINE_SECS` | `120` | ≥ 10; whole-turn wall clock |
//! | `FKST_CHAT_MAX_CONCURRENT_TURNS` | `4` | ≥ 1; process-wide admission |
//! | `FKST_CHAT_HISTORY_MAX_MESSAGES` | `40` | ≥ 2; oldest-first truncation target |
//! | `FKST_CHAT_REQUEST_MAX_BYTES` | `262144` | ≥ 4096; request-body limit |

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;

/// Env prefix for the chat block.
const CHAT_ENV_PREFIX: &str = "FKST_CHAT_";
/// Env prefix of the session-engine LLM block the chat knobs fall back to.
const LLM_ENV_PREFIX: &str = "FKST_LLM_";

/// Raw `FKST_CHAT_*` values. Every field is `Option`/defaulted so the pass never
/// fails at deserialize time — the rules are enforced in [`from_vars`], which can
/// name the exact offending variable in the error (an envy error cannot).
#[derive(Debug, Deserialize)]
struct ChatVars {
    #[serde(default)]
    enabled: bool,
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
    #[serde(default = "defaults::max_tool_iterations")]
    max_tool_iterations: u32,
    #[serde(default = "defaults::turn_deadline_secs")]
    turn_deadline_secs: u64,
    #[serde(default = "defaults::max_concurrent_turns")]
    max_concurrent_turns: usize,
    #[serde(default = "defaults::history_max_messages")]
    history_max_messages: usize,
    #[serde(default = "defaults::request_max_bytes")]
    request_max_bytes: usize,
}

/// The `FKST_LLM_*` subset the chat block inherits. These are read RAW (no serde
/// defaults): "chat var unset AND LLM fallback unset" must be a fail-closed error
/// naming both, not a silent slide into the session engine's default model.
#[derive(Debug, Deserialize)]
struct LlmFallbackVars {
    #[serde(default)]
    base_url: Option<String>,
    #[serde(default)]
    api_key: Option<String>,
    #[serde(default)]
    model: Option<String>,
}

mod defaults {
    pub(super) fn max_tool_iterations() -> u32 {
        8
    }
    pub(super) fn turn_deadline_secs() -> u64 {
        120
    }
    pub(super) fn max_concurrent_turns() -> usize {
        4
    }
    pub(super) fn history_max_messages() -> usize {
        40
    }
    pub(super) fn request_max_bytes() -> usize {
        256 * 1024
    }
}

/// Lower bounds. Each exists because the value below it makes the feature
/// nonsensical rather than merely aggressive: zero tool iterations means the model
/// can never answer a data question, a sub-10s deadline expires before a slow
/// provider's first token, a 2-message cap cannot hold one exchange, and a 4 KiB
/// body cannot hold a real conversation.
const MIN_TOOL_ITERATIONS: u32 = 1;
const MIN_TURN_DEADLINE_SECS: u64 = 10;
const MIN_CONCURRENT_TURNS: usize = 1;
const MIN_HISTORY_MESSAGES: usize = 2;
const MIN_REQUEST_MAX_BYTES: usize = 4096;

/// Resolved chat configuration. Only ever constructed by [`from_vars`], and only
/// when the feature is enabled — so every consumer can assume the values are
/// already validated.
#[derive(Clone)]
pub struct ChatConfig {
    /// OpenAI-compatible provider base URL. `/chat/completions` is appended by the
    /// client, so this is the `/v1`-style prefix, with or without a trailing slash.
    pub base_url: reqwest::Url,
    /// Provider bearer credential. Never logged; redacted in `Debug`.
    pub api_key: SecretString,
    /// Model id sent on every turn.
    pub model: String,
    /// Maximum model↔tools round trips in one turn before the orchestrator gives up.
    pub max_tool_iterations: u32,
    /// Whole-turn wall-clock budget (tool calls included).
    pub turn_deadline_secs: u64,
    /// Process-wide cap on turns running at once.
    pub max_concurrent_turns: usize,
    /// Message-count cap the orchestrator truncates over-long histories down to
    /// (oldest-first) rather than rejecting them.
    pub history_max_messages: usize,
    /// Request-body limit for `POST /api/v1/chat`.
    pub request_max_bytes: usize,
}

// Manual `Debug` rendering the credential as `<redacted>` (the config-module
// convention, mirroring `OpensandboxConfig`) so an accidental `{:?}` on this — or
// on the `Config` embedding it — can never spill the provider key into a log.
impl std::fmt::Debug for ChatConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChatConfig")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .field("model", &self.model)
            .field("max_tool_iterations", &self.max_tool_iterations)
            .field("turn_deadline_secs", &self.turn_deadline_secs)
            .field("max_concurrent_turns", &self.max_concurrent_turns)
            .field("history_max_messages", &self.history_max_messages)
            .field("request_max_bytes", &self.request_max_bytes)
            .finish()
    }
}

/// Trim a raw env value; a blank string counts as absent so a stray empty
/// ConfigMap value never masquerades as a real setting (the repo-wide convention,
/// see [`crate::osb_config`]).
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Deserialize the chat configuration from environment-style pairs.
///
/// Returns `Ok(None)` when `FKST_CHAT_ENABLED` is not true — the feature is then
/// entirely dark and no other `FKST_CHAT_*` value is validated. When enabled,
/// every rule is enforced fail-closed with an error naming the exact variable
/// (and its `FKST_LLM_*` fallback where one applies).
///
/// Testable seam: shares the caller's already-collected `vars` snapshot (see
/// [`crate::config::Config::from_vars`]) instead of reading the process env.
pub fn from_vars(vars: &[(String, String)]) -> Result<Option<ChatConfig>, AppError> {
    let raw: ChatVars = envy::prefixed(CHAT_ENV_PREFIX)
        .from_iter(vars.iter().cloned())
        .map_err(|e| AppError::Config(e.to_string()))?;

    // Feature off: nothing below is read, so a partially-staged chat block cannot
    // break an unrelated deploy.
    if !raw.enabled {
        return Ok(None);
    }

    let fallback: LlmFallbackVars = envy::prefixed(LLM_ENV_PREFIX)
        .from_iter(vars.iter().cloned())
        .map_err(|e| AppError::Config(e.to_string()))?;

    let base_url_raw = non_blank(raw.base_url)
        .or_else(|| non_blank(fallback.base_url))
        .ok_or_else(|| {
            AppError::Config(
                "FKST_CHAT_BASE_URL (or fallback FKST_LLM_BASE_URL) must be set \
                 when FKST_CHAT_ENABLED=true"
                    .to_string(),
            )
        })?;
    let base_url = reqwest::Url::parse(&base_url_raw).map_err(|e| {
        AppError::Config(format!(
            "FKST_CHAT_BASE_URL (or fallback FKST_LLM_BASE_URL) must be a valid URL \
             when FKST_CHAT_ENABLED=true: {e}"
        ))
    })?;

    let api_key = non_blank(raw.api_key)
        .or_else(|| non_blank(fallback.api_key))
        .ok_or_else(|| {
            AppError::Config(
                "FKST_CHAT_API_KEY (or fallback FKST_LLM_API_KEY) must be set \
                 when FKST_CHAT_ENABLED=true"
                    .to_string(),
            )
        })?;

    let model = non_blank(raw.model)
        .or_else(|| non_blank(fallback.model))
        .ok_or_else(|| {
            AppError::Config(
                "FKST_CHAT_MODEL (or fallback FKST_LLM_MODEL) must be set \
                 when FKST_CHAT_ENABLED=true"
                    .to_string(),
            )
        })?;

    if raw.max_tool_iterations < MIN_TOOL_ITERATIONS {
        return Err(AppError::Config(format!(
            "FKST_CHAT_MAX_TOOL_ITERATIONS must be at least {MIN_TOOL_ITERATIONS}"
        )));
    }
    if raw.turn_deadline_secs < MIN_TURN_DEADLINE_SECS {
        return Err(AppError::Config(format!(
            "FKST_CHAT_TURN_DEADLINE_SECS must be at least {MIN_TURN_DEADLINE_SECS}"
        )));
    }
    if raw.max_concurrent_turns < MIN_CONCURRENT_TURNS {
        return Err(AppError::Config(format!(
            "FKST_CHAT_MAX_CONCURRENT_TURNS must be at least {MIN_CONCURRENT_TURNS}"
        )));
    }
    if raw.history_max_messages < MIN_HISTORY_MESSAGES {
        return Err(AppError::Config(format!(
            "FKST_CHAT_HISTORY_MAX_MESSAGES must be at least {MIN_HISTORY_MESSAGES}"
        )));
    }
    if raw.request_max_bytes < MIN_REQUEST_MAX_BYTES {
        return Err(AppError::Config(format!(
            "FKST_CHAT_REQUEST_MAX_BYTES must be at least {MIN_REQUEST_MAX_BYTES}"
        )));
    }

    Ok(Some(ChatConfig {
        base_url,
        api_key: SecretString::from(api_key),
        model,
        max_tool_iterations: raw.max_tool_iterations,
        turn_deadline_secs: raw.turn_deadline_secs,
        max_concurrent_turns: raw.max_concurrent_turns,
        history_max_messages: raw.history_max_messages,
        request_max_bytes: raw.request_max_bytes,
    }))
}

#[cfg(test)]
#[path = "config_tests.rs"]
mod tests;
