//! The per-session codex `config.toml` renderer.
//!
//! v1 runs every session against a single, operator-pinned LLM provider. The
//! model, base URL, and wire_api are config-driven (`FKST_LLM_MODEL` /
//! `FKST_LLM_BASE_URL` / `FKST_LLM_WIRE_API`), and the static LLM API key rides
//! the `env_key` ([`LLM_ENV_KEY`]) — never embedded in the config. (The old
//! vault-driven Raw/Structured provider selection was removed with the in-memory
//! vault; a custom-provider path can return as a typed layer if v1 needs it.)

/// `env_key` the engine's codex reads the LLM API key from.
///
/// MUST be `LLM_API_KEY`, NOT `FKST_LLM_API_KEY`: the engine's
/// `is_reserved_env_key` strips any `FKST_`-prefixed env var, so an `FKST_`-named
/// key would be silently dropped and the engine would 401. `FKST_LLM_API_KEY` is
/// the CONTROL-PLANE config var name only.
pub const LLM_ENV_KEY: &str = "LLM_API_KEY";

/// Default `wire_api` for the LLM provider.
///
/// MUST be `chat`: chrono-llm serves only `/chat/completions`; `responses`
/// returns 502 (a verified bug). Never default to `responses`.
pub const DEFAULT_WIRE_API: &str = "chat";

/// `model_provider` id + provider name. Neutral (no vendor coupling) so the same
/// renderer serves any OpenAI-compatible backend.
pub const LLM_PROVIDER_ID: &str = "llm";

/// Render the codex `config.toml` body for the operator-pinned LLM provider.
///
/// `model` / `base_url` / `wire_api` are the config-driven provider values and
/// `env_key` is the environment variable the codex reads the API key from (the
/// caller passes [`LLM_ENV_KEY`]). `disable_response_storage = true` because the
/// provider is stateless for the session.
pub fn render_codex_config(model: &str, base_url: &str, wire_api: &str, env_key: &str) -> String {
    format!(
        "model_provider = \"{LLM_PROVIDER_ID}\"\n\
         model = \"{model}\"\n\
         disable_response_storage = true\n\
         \n\
         [model_providers.{LLM_PROVIDER_ID}]\n\
         name = \"{LLM_PROVIDER_ID}\"\n\
         base_url = \"{base_url}\"\n\
         wire_api = \"{wire_api}\"\n\
         env_key = \"{env_key}\"\n"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_pinned_model_with_neutral_provider_and_llm_env_key() {
        let toml = render_codex_config(
            "gpt-5-codex",
            "https://nyx/p",
            DEFAULT_WIRE_API,
            LLM_ENV_KEY,
        );
        assert!(toml.contains("model_provider = \"llm\""));
        assert!(toml.contains("[model_providers.llm]"));
        assert!(toml.contains("model = \"gpt-5-codex\""));
        assert!(toml.contains("base_url = \"https://nyx/p\""));
        // wire_api is a parameter and defaults to `chat`, NEVER `responses`.
        assert!(toml.contains("wire_api = \"chat\""));
        assert!(!toml.contains("responses"));
        // The engine reads the LLM credential from the `LLM_API_KEY` env key.
        assert!(toml.contains("env_key = \"LLM_API_KEY\""));
        assert!(toml.contains("disable_response_storage = true"));
    }

    #[test]
    fn wire_api_is_a_render_parameter() {
        let toml = render_codex_config("m", "https://b", "responses", LLM_ENV_KEY);
        // The renderer honours whatever wire_api the caller passes (the safe
        // default is enforced by the caller / config, not hard-coded here).
        assert!(toml.contains("wire_api = \"responses\""));
    }
}
