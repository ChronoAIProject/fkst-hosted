//! The per-session codex `config.toml` renderer (Model B, issue #359 §5).
//!
//! Every session runs against a single, operator-pinned LLM provider. The model,
//! base URL, and wire_api are config-driven (`FKST_LLM_MODEL` / `FKST_LLM_BASE_URL`
//! / `FKST_LLM_WIRE_API`) and injected into the session pod; the static LLM API key
//! rides the `env_key` ([`crate::reserved_env::LLM_ENV_KEY`]) — never embedded in
//! the config. Relocated out of the deleted `sessions/codex_provider` so the
//! `run-substrate` driver keeps its only caller.

use std::collections::BTreeMap;

/// `model_provider` id + provider name. Neutral (no vendor coupling) so the same
/// renderer serves any OpenAI-compatible backend. The `wire_api` default itself
/// lives on the launch plan (`plan::DEFAULT_LLM_WIRE_API`), which is what the
/// driver passes in.
const LLM_PROVIDER_ID: &str = "llm";

/// The named-environment shell context codex must forward to the commands it runs.
///
/// A profile's install commands land tools in a session-writable env-bin that the
/// driver prepends to the supervise `PATH`; its non-secret variables ride the
/// session env too. But codex runs the agent's shell commands under its OWN
/// environment policy, so without this those tools/vars are invisible to codex —
/// the render step calling `ffmpeg` fails with "not on PATH". This carries exactly
/// the NON-SECRET bits to expose ([`render_codex_config`] writes them into
/// `[shell_environment_policy].set`); secrets never belong in `config.toml`.
pub struct CodexShellEnv<'a> {
    /// The full `PATH` to expose (env-bin already prepended), so a profile-installed
    /// tool resolves from inside codex.
    pub path: &'a str,
    /// The `FKST_ENV_BIN` value (the writable tool dir), or `""` when the profile
    /// installed no tools (variables-only profile).
    pub tool_dir: &'a str,
    /// Non-secret env-profile variables (e.g. `BRAND_COLOR`, `FONT_FILE`).
    pub variables: &'a BTreeMap<String, String>,
}

/// Render the codex `config.toml` body for the operator-pinned LLM provider.
///
/// `model` / `base_url` / `wire_api` are the config-driven provider values and
/// `env_key` is the environment variable the codex reads the API key from (the
/// caller passes [`crate::reserved_env::LLM_ENV_KEY`]). `disable_response_storage
/// = true` because the provider is stateless for the session.
///
/// When `shell_env` carries an active named environment, a
/// `[shell_environment_policy]` is appended so the profile's tools/variables reach
/// codex's shell commands. `None` (or an empty profile) renders no policy, keeping
/// codex's default behavior for profile-less sessions.
pub fn render_codex_config(
    model: &str,
    base_url: &str,
    wire_api: &str,
    env_key: &str,
    shell_env: Option<&CodexShellEnv>,
) -> String {
    let mut toml = format!(
        "model_provider = \"{LLM_PROVIDER_ID}\"\n\
         model = \"{model}\"\n\
         disable_response_storage = true\n\
         \n\
         [model_providers.{LLM_PROVIDER_ID}]\n\
         name = \"{LLM_PROVIDER_ID}\"\n\
         base_url = \"{base_url}\"\n\
         wire_api = \"{wire_api}\"\n\
         env_key = \"{env_key}\"\n"
    );
    if let Some(policy) = shell_env.and_then(render_shell_env_policy) {
        toml.push('\n');
        toml.push_str(&policy);
    }
    toml
}

/// Build the `[shell_environment_policy]` block exposing the profile's env to
/// codex's shell commands, or `None` when there is nothing profile-specific to add.
///
/// `inherit = "all"` keeps everything codex already forwarded (git/gh wiring, HOME,
/// …) and ADDS the profile env on top — a strict superset, so nothing that worked
/// before a profile was attached can break. `set` overrides `PATH` with the
/// env-bin-prepended value and injects `FKST_ENV_BIN` + the non-secret variables.
fn render_shell_env_policy(shell_env: &CodexShellEnv) -> Option<String> {
    let mut set: Vec<(String, &str)> = Vec::new();
    if !shell_env.tool_dir.is_empty() {
        set.push(("PATH".to_string(), shell_env.path));
        set.push((crate::install::TOOL_DIR_ENV.to_string(), shell_env.tool_dir));
    }
    for (name, value) in shell_env.variables {
        set.push((name.clone(), value.as_str()));
    }
    if set.is_empty() {
        return None;
    }
    let entries: Vec<String> = set
        .iter()
        .map(|(name, value)| format!("\"{}\" = \"{}\"", toml_escape(name), toml_escape(value)))
        .collect();
    Some(format!(
        "[shell_environment_policy]\ninherit = \"all\"\nset = {{ {} }}\n",
        entries.join(", ")
    ))
}

/// Escape a bare string for a double-quoted TOML value/key (backslash + quote).
fn toml_escape(value: &str) -> String {
    value.replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reserved_env::LLM_ENV_KEY;

    #[test]
    fn renders_pinned_model_with_neutral_provider_and_llm_env_key() {
        let toml = render_codex_config("gpt-5-codex", "https://nyx/p", "chat", LLM_ENV_KEY, None);
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
        // No profile → no shell-environment policy.
        assert!(!toml.contains("shell_environment_policy"));
    }

    #[test]
    fn wire_api_is_a_render_parameter() {
        let toml = render_codex_config("m", "https://b", "responses", LLM_ENV_KEY, None);
        // The renderer honours whatever wire_api the caller passes (the safe
        // default is enforced by the caller / config, not hard-coded here).
        assert!(toml.contains("wire_api = \"responses\""));
    }

    #[test]
    fn renders_shell_policy_exposing_env_profile_tools_and_variables() {
        let vars = BTreeMap::from([
            ("BRAND_COLOR".to_string(), "0x0B5FFF".to_string()),
            ("FONT_FILE".to_string(), "OpenSans-Regular.ttf".to_string()),
        ]);
        let shell = CodexShellEnv {
            path: "/rt/env-bin:/usr/bin:/bin",
            tool_dir: "/rt/env-bin",
            variables: &vars,
        };
        let toml = render_codex_config("m", "https://b", "chat", LLM_ENV_KEY, Some(&shell));
        assert!(toml.contains("[shell_environment_policy]"));
        // A strict superset of the default — nothing that worked before breaks.
        assert!(toml.contains("inherit = \"all\""));
        // The env-bin-prepended PATH so profile tools (ffmpeg) resolve inside codex.
        assert!(toml.contains("\"PATH\" = \"/rt/env-bin:/usr/bin:/bin\""));
        assert!(toml.contains("\"FKST_ENV_BIN\" = \"/rt/env-bin\""));
        assert!(toml.contains("\"BRAND_COLOR\" = \"0x0B5FFF\""));
        assert!(toml.contains("\"FONT_FILE\" = \"OpenSans-Regular.ttf\""));
    }

    #[test]
    fn variables_only_profile_sets_vars_without_path_or_tool_dir() {
        let vars = BTreeMap::from([("REGION".to_string(), "us".to_string())]);
        let shell = CodexShellEnv {
            path: "/unused",
            tool_dir: "",
            variables: &vars,
        };
        let toml = render_codex_config("m", "https://b", "chat", LLM_ENV_KEY, Some(&shell));
        assert!(toml.contains("[shell_environment_policy]"));
        assert!(toml.contains("\"REGION\" = \"us\""));
        // No install → no tool dir → PATH/FKST_ENV_BIN are not overridden.
        assert!(!toml.contains("\"PATH\""));
        assert!(!toml.contains("FKST_ENV_BIN"));
    }

    #[test]
    fn empty_profile_renders_no_policy() {
        let vars = BTreeMap::new();
        let shell = CodexShellEnv {
            path: "",
            tool_dir: "",
            variables: &vars,
        };
        let toml = render_codex_config("m", "https://b", "chat", LLM_ENV_KEY, Some(&shell));
        assert!(!toml.contains("shell_environment_policy"));
    }
}
