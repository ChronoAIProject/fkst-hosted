//! The per-session codex `config.toml` renderer.
//!
//! v1 runs every session against the NyxID-proxied chrono-llm DEFAULT provider:
//! the model + base URL are operator-pinned (`FKST_HOSTED_CODEX_MODEL` /
//! `FKST_HOSTED_CHRONO_LLM_BASE_URL`), and the per-session NyxID token rides the
//! `env_key` ([`DEFAULT_ENV_KEY`]) — never embedded in the config. (The old
//! vault-driven Raw/Structured provider selection was removed with the in-memory
//! vault; a custom-provider path can return as a typed layer if v1 needs it.)

/// `env_key` for the chrono-llm DEFAULT: the per-session NyxID user token (#111),
/// sent as `Authorization: Bearer` to the NyxID proxy.
pub const DEFAULT_ENV_KEY: &str = "NYXID_ACCESS_TOKEN";

/// wire_api for the chrono-llm DEFAULT: the OpenAI Responses API the proxy serves.
pub const DEFAULT_WIRE_API: &str = "responses";

/// `model_provider` id + provider name for the chrono-llm DEFAULT.
pub const CHRONO_LLM_PROVIDER_ID: &str = "chrono-llm";

/// The resolved provider layer. v1 has a single layer (the chrono-llm DEFAULT);
/// kept as an enum so a future custom-provider layer can be added without
/// changing the renderer's call sites.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProviderChoice {
    /// The NyxID-proxied `chrono-llm` default.
    DefaultChronoLlm,
}

/// Render the codex `config.toml` body for the chosen layer. `default_model` /
/// `default_base_url` are the operator-pinned chrono-llm values.
pub fn render_codex_config(
    choice: &ProviderChoice,
    default_model: &str,
    default_base_url: &str,
) -> String {
    match choice {
        ProviderChoice::DefaultChronoLlm => render_default(default_model, default_base_url),
    }
}

/// Render the chrono-llm DEFAULT toml. `disable_response_storage = true` because
/// the proxy is stateless for the session user.
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
mod tests {
    use super::*;

    #[test]
    fn default_renders_chrono_llm_with_pinned_model_and_token_env_key() {
        let toml = render_codex_config(
            &ProviderChoice::DefaultChronoLlm,
            "gpt-5-codex",
            "https://nyx/p",
        );
        assert!(toml.contains("model_provider = \"chrono-llm\""));
        assert!(toml.contains("model = \"gpt-5-codex\""));
        assert!(toml.contains("base_url = \"https://nyx/p\""));
        assert!(toml.contains("wire_api = \"responses\""));
        assert!(toml.contains("env_key = \"NYXID_ACCESS_TOKEN\""));
        assert!(toml.contains("disable_response_storage = true"));
    }
}
