//! Unit tests for the codex provider module (issue #112): layer
//! selection + precedence, per-layer `config.toml` rendering, and the
//! chrono-llm connection precondition via the `ChronoLlmCheck` seam.

use super::*;
use secrecy::SecretString;

/// Build resolved vault entries from (key, value) pairs. The kind is
/// irrelevant to the resolver (the vault returns every value as a
/// `SecretString` regardless), so the helper mirrors `list_for_scope`.
fn entries(pairs: &[(&str, &str)]) -> Vec<ResolvedEntry> {
    pairs
        .iter()
        .map(|(k, v)| ResolvedEntry {
            key: k.to_string(),
            value: SecretString::from(v.to_string()),
        })
        .collect()
}

const DEFAULT_MODEL: &str = "gpt-5-codex";
const DEFAULT_BASE_URL: &str = "https://nyx.chrono-ai.fun/api/v1/proxy/s/chrono-llm";

// ---- classify_entries: layer selection + precedence ----------------------

#[test]
fn no_entries_selects_default() {
    assert_eq!(classify_entries(&[]), ProviderChoice::DefaultChronoLlm);
}

#[test]
fn unrelated_entries_select_default() {
    let resolved = entries(&[("OPENAI_API_KEY", "sk-x"), ("FEATURE_FLAG", "on")]);
    assert_eq!(
        classify_entries(&resolved),
        ProviderChoice::DefaultChronoLlm
    );
}

#[test]
fn all_four_structured_fields_select_structured() {
    let resolved = entries(&[
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
        (STRUCTURED_ENV_KEY_KEY, "OPENAI_API_KEY"),
    ]);
    assert_eq!(
        classify_entries(&resolved),
        ProviderChoice::Structured(StructuredProvider {
            provider_id: STRUCTURED_PROVIDER_ID.to_string(),
            model: "gpt-4.1".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            wire_api: "responses".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
        })
    );
}

#[test]
fn partial_structured_fields_fall_back_to_default() {
    // Missing CODEX_ENV_KEY: not enough to render a provider table.
    let resolved = entries(&[
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
    ]);
    assert_eq!(
        classify_entries(&resolved),
        ProviderChoice::DefaultChronoLlm
    );
}

#[test]
fn raw_config_selects_raw() {
    let resolved = entries(&[(RAW_CONFIG_KEY, "model = \"custom\"\n")]);
    assert_eq!(
        classify_entries(&resolved),
        ProviderChoice::Raw("model = \"custom\"\n".to_string())
    );
}

#[test]
fn raw_wins_over_structured_precedence() {
    // RAW present alongside the full structured set: RAW must win.
    let resolved = entries(&[
        (RAW_CONFIG_KEY, "model = \"raw-wins\"\n"),
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
        (STRUCTURED_ENV_KEY_KEY, "OPENAI_API_KEY"),
    ]);
    assert_eq!(
        classify_entries(&resolved),
        ProviderChoice::Raw("model = \"raw-wins\"\n".to_string())
    );
}

#[test]
fn structured_wins_over_default_precedence() {
    // Structured set present (no RAW): structured beats the default.
    let resolved = entries(&[
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
        (STRUCTURED_ENV_KEY_KEY, "OPENAI_API_KEY"),
    ]);
    assert!(matches!(
        classify_entries(&resolved),
        ProviderChoice::Structured(_)
    ));
}

// ---- render_codex_config: per-layer toml ---------------------------------

#[test]
fn render_default_uses_injected_config_values_and_default_only_flags() {
    let toml = render_codex_config(
        &ProviderChoice::DefaultChronoLlm,
        DEFAULT_MODEL,
        DEFAULT_BASE_URL,
    )
    .expect("render default");

    // model/base_url come from the injected config, NOT a placeholder.
    assert!(
        toml.contains(&format!("model = \"{DEFAULT_MODEL}\"")),
        "{toml}"
    );
    assert!(
        toml.contains(&format!("base_url = \"{DEFAULT_BASE_URL}\"")),
        "{toml}"
    );
    assert!(
        !toml.contains("FKST_HOSTED"),
        "no placeholder leaked:\n{toml}"
    );
    // chrono-llm provider table + responses wire + the NyxID env_key.
    assert!(toml.contains("model_provider = \"chrono-llm\""), "{toml}");
    assert!(toml.contains("[model_providers.chrono-llm]"), "{toml}");
    assert!(toml.contains("wire_api = \"responses\""), "{toml}");
    assert!(toml.contains("env_key = \"NYXID_ACCESS_TOKEN\""), "{toml}");
    // disable_response_storage is the DEFAULT-only flag.
    assert!(toml.contains("disable_response_storage = true"), "{toml}");
}

#[test]
fn render_structured_omits_disable_response_storage() {
    let provider = StructuredProvider {
        provider_id: STRUCTURED_PROVIDER_ID.to_string(),
        model: "gpt-4.1".to_string(),
        base_url: "https://api.openai.com/v1".to_string(),
        wire_api: "responses".to_string(),
        env_key: "OPENAI_API_KEY".to_string(),
    };
    let toml = render_codex_config(
        &ProviderChoice::Structured(provider),
        DEFAULT_MODEL,
        DEFAULT_BASE_URL,
    )
    .expect("render structured");

    assert!(toml.contains("model_provider = \"user\""), "{toml}");
    assert!(toml.contains("[model_providers.user]"), "{toml}");
    assert!(toml.contains("model = \"gpt-4.1\""), "{toml}");
    assert!(
        toml.contains("base_url = \"https://api.openai.com/v1\""),
        "{toml}"
    );
    assert!(toml.contains("wire_api = \"responses\""), "{toml}");
    assert!(toml.contains("env_key = \"OPENAI_API_KEY\""), "{toml}");
    // The DEFAULT-only flag must NOT appear on a structured override.
    assert!(
        !toml.contains("disable_response_storage"),
        "structured must not carry the default flag:\n{toml}"
    );
    // The chrono-llm default values must not bleed into a structured render.
    assert!(!toml.contains("chrono-llm"), "{toml}");
    assert!(!toml.contains(DEFAULT_BASE_URL), "{toml}");
}

#[test]
fn render_raw_is_verbatim() {
    let raw = "model = \"verbatim\"\n[model_providers.x]\nname = \"x\"\n";
    let toml = render_codex_config(
        &ProviderChoice::Raw(raw.to_string()),
        DEFAULT_MODEL,
        DEFAULT_BASE_URL,
    )
    .expect("render raw");
    assert_eq!(toml, raw);
}

#[test]
fn rendered_layers_are_well_formed_key_value_lines() {
    // Defence in depth without a toml parser dependency: every emitted
    // line is either blank, a `[table]` header, or a `key = value` pair —
    // a quoting/format bug would otherwise ship a config codex rejects.
    for toml in [
        render_codex_config(
            &ProviderChoice::DefaultChronoLlm,
            DEFAULT_MODEL,
            DEFAULT_BASE_URL,
        )
        .unwrap(),
        render_codex_config(
            &ProviderChoice::Structured(StructuredProvider {
                provider_id: STRUCTURED_PROVIDER_ID.to_string(),
                model: "m".to_string(),
                base_url: "https://x/v1".to_string(),
                wire_api: "responses".to_string(),
                env_key: "OPENAI_API_KEY".to_string(),
            }),
            DEFAULT_MODEL,
            DEFAULT_BASE_URL,
        )
        .unwrap(),
    ] {
        for line in toml.lines() {
            let trimmed = line.trim();
            let ok = trimmed.is_empty()
                || (trimmed.starts_with('[') && trimmed.ends_with(']'))
                || trimmed
                    .split_once('=')
                    .is_some_and(|(k, _)| !k.trim().is_empty());
            assert!(ok, "malformed config line: {line:?}\n{toml}");
        }
    }
}

// ---- resolve_from_entries: precedence + the chrono-llm precondition ------
// The `list_for_scope` fetch is plumbing covered by the vault's own tests;
// here we drive the resolver core with canned `ResolvedEntry`s (the brief's
// "canned list_for_scope" seam) + a `ChronoLlmCheck` fake, so precedence and
// the missing-connection -> 422 mapping are tested with NO live vault.

/// A [`ChronoLlmCheck`] fake returning a canned connection verdict, so the
/// missing-connection -> 422 mapping is tested without a live NyxID account.
struct FakeCheck(Result<bool, ()>);

#[async_trait]
impl ChronoLlmCheck for FakeCheck {
    async fn is_chrono_llm_connected(
        &self,
        _owner: &str,
        _org: Option<&str>,
    ) -> Result<bool, AppError> {
        match self.0 {
            Ok(connected) => Ok(connected),
            Err(()) => Err(AppError::Unavailable("nyxid lookup failed".to_string())),
        }
    }
}

#[tokio::test]
async fn resolve_no_entries_is_default_when_connected() {
    let choice = resolve_from_entries(&[], "owner", None, &FakeCheck(Ok(true)))
        .await
        .expect("resolve");
    assert_eq!(choice, ProviderChoice::DefaultChronoLlm);
}

#[tokio::test]
async fn resolve_default_without_chrono_llm_connection_is_422() {
    let err = resolve_from_entries(&[], "owner", None, &FakeCheck(Ok(false)))
        .await
        .expect_err("missing chrono-llm connection must 422");
    assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
}

#[tokio::test]
async fn resolve_propagates_transient_check_error() {
    // Default path, check errors transiently -> the error propagates (not a
    // 422, which is reserved for a definitive "not connected").
    let err = resolve_from_entries(&[], "owner", None, &FakeCheck(Err(())))
        .await
        .expect_err("transient check error must propagate");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

#[tokio::test]
async fn resolve_structured_entries_skip_the_connection_check() {
    let resolved = entries(&[
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
        (STRUCTURED_ENV_KEY_KEY, "OPENAI_API_KEY"),
    ]);
    // A check that would error proves the connection check is NOT consulted
    // off the default path (a structured override carries its own key).
    let choice = resolve_from_entries(&resolved, "owner", None, &FakeCheck(Err(())))
        .await
        .expect("resolve structured");
    assert_eq!(
        choice,
        ProviderChoice::Structured(StructuredProvider {
            provider_id: STRUCTURED_PROVIDER_ID.to_string(),
            model: "gpt-4.1".to_string(),
            base_url: "https://api.openai.com/v1".to_string(),
            wire_api: "responses".to_string(),
            env_key: "OPENAI_API_KEY".to_string(),
        })
    );
}

#[tokio::test]
async fn resolve_raw_entry_skips_the_connection_check() {
    // RAW present alongside a full structured set: RAW wins and the check is
    // not consulted (a failing check fake would otherwise surface).
    let resolved = entries(&[
        (RAW_CONFIG_KEY, "model = \"raw\"\n"),
        (STRUCTURED_BASE_URL_KEY, "https://api.openai.com/v1"),
        (STRUCTURED_MODEL_KEY, "gpt-4.1"),
        (STRUCTURED_WIRE_API_KEY, "responses"),
        (STRUCTURED_ENV_KEY_KEY, "OPENAI_API_KEY"),
    ]);
    let choice = resolve_from_entries(&resolved, "owner", None, &FakeCheck(Err(())))
        .await
        .expect("resolve raw");
    assert_eq!(choice, ProviderChoice::Raw("model = \"raw\"\n".to_string()));
}
