//! The per-session codex `config.toml` renderer (Model B, issue #359 §5).
//!
//! Every session runs against a single, operator-pinned LLM provider. The model,
//! base URL, and wire_api are config-driven (`FKST_LLM_MODEL` / `FKST_LLM_BASE_URL`
//! / `FKST_LLM_WIRE_API`) and injected into the session pod; the static LLM API key
//! rides the `env_key` ([`crate::reserved_env::LLM_ENV_KEY`]) — never embedded in
//! the config. Relocated out of the deleted `sessions/codex_provider` so the
//! `run-substrate` driver keeps its only caller.

/// `model_provider` id + provider name. Neutral (no vendor coupling) so the same
/// renderer serves any OpenAI-compatible backend. The `wire_api` default itself
/// lives on the launch plan (`plan::DEFAULT_LLM_WIRE_API`), which is what the
/// driver passes in.
const LLM_PROVIDER_ID: &str = "llm";

/// Render the codex `config.toml` body for the operator-pinned LLM provider.
///
/// `model` / `base_url` / `wire_api` are the config-driven provider values and
/// `env_key` is the environment variable the codex reads the API key from (the
/// caller passes [`crate::reserved_env::LLM_ENV_KEY`]). `disable_response_storage
/// = true` because the provider is stateless for the session.
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
    use crate::reserved_env::LLM_ENV_KEY;

    #[test]
    fn renders_pinned_model_with_neutral_provider_and_llm_env_key() {
        let toml = render_codex_config("gpt-5-codex", "https://nyx/p", "chat", LLM_ENV_KEY);
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
