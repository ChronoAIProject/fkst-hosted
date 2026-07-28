//! Tests for [`super`] (the `FKST_CHAT_*` config pass). Split into a sibling file
//! to keep `config.rs` under the 500-line limit; included via
//! `#[cfg(test)] #[path = "config_tests.rs"] mod tests;`.

use secrecy::ExposeSecret;

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// The minimum enabled block relying entirely on the `FKST_LLM_*` fallbacks.
fn enabled_via_fallback() -> Vec<(String, String)> {
    vars(&[
        ("FKST_CHAT_ENABLED", "true"),
        ("FKST_LLM_BASE_URL", "https://llm.example/v1"),
        ("FKST_LLM_API_KEY", "llm-key"),
        ("FKST_LLM_MODEL", "llm-model"),
    ])
}

// ---- feature gate ---------------------------------------------------------

#[test]
fn unset_block_is_dark() {
    let config = from_vars(&[]).expect("no vars must parse");
    assert!(
        config.is_none(),
        "chat must be off unless explicitly enabled"
    );
}

#[test]
fn disabled_block_ignores_every_other_value() {
    // Deliberately garbage values: with the feature off they must never be read,
    // so a half-staged block cannot fail an unrelated deploy.
    let config = from_vars(&vars(&[
        ("FKST_CHAT_ENABLED", "false"),
        ("FKST_CHAT_BASE_URL", "not-a-url"),
        ("FKST_CHAT_MAX_TOOL_ITERATIONS", "0"),
    ]))
    .expect("a disabled block must never validate its own values");
    assert!(config.is_none());
}

// ---- fallbacks and overrides ----------------------------------------------

#[test]
fn enabled_inherits_the_llm_block() {
    let config = from_vars(&enabled_via_fallback())
        .expect("must parse")
        .expect("must be enabled");
    assert_eq!(config.base_url.as_str(), "https://llm.example/v1");
    assert_eq!(config.api_key.expose_secret(), "llm-key");
    assert_eq!(config.model, "llm-model");
    // Defaults for everything without an LLM equivalent.
    assert_eq!(config.max_tool_iterations, 8);
    assert_eq!(config.turn_deadline_secs, 120);
    assert_eq!(config.max_concurrent_turns, 4);
    assert_eq!(config.history_max_messages, 40);
    assert_eq!(config.request_max_bytes, 256 * 1024);
}

#[test]
fn explicit_chat_values_win_over_the_fallbacks() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[
        ("FKST_CHAT_BASE_URL", "https://chat.example/v1/"),
        ("FKST_CHAT_API_KEY", "chat-key"),
        ("FKST_CHAT_MODEL", "chat-model"),
        ("FKST_CHAT_MAX_TOOL_ITERATIONS", "3"),
        ("FKST_CHAT_TURN_DEADLINE_SECS", "45"),
        ("FKST_CHAT_MAX_CONCURRENT_TURNS", "9"),
        ("FKST_CHAT_HISTORY_MAX_MESSAGES", "12"),
        ("FKST_CHAT_REQUEST_MAX_BYTES", "8192"),
    ]));
    let config = from_vars(&pairs)
        .expect("must parse")
        .expect("must be enabled");
    assert_eq!(config.base_url.as_str(), "https://chat.example/v1/");
    assert_eq!(config.api_key.expose_secret(), "chat-key");
    assert_eq!(config.model, "chat-model");
    assert_eq!(config.max_tool_iterations, 3);
    assert_eq!(config.turn_deadline_secs, 45);
    assert_eq!(config.max_concurrent_turns, 9);
    assert_eq!(config.history_max_messages, 12);
    assert_eq!(config.request_max_bytes, 8192);
}

#[test]
fn blank_chat_values_fall_back_rather_than_masquerading() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[
        ("FKST_CHAT_BASE_URL", "   "),
        ("FKST_CHAT_API_KEY", ""),
        ("FKST_CHAT_MODEL", "\t"),
    ]));
    let config = from_vars(&pairs)
        .expect("must parse")
        .expect("must be enabled");
    assert_eq!(config.base_url.as_str(), "https://llm.example/v1");
    assert_eq!(config.api_key.expose_secret(), "llm-key");
    assert_eq!(config.model, "llm-model");
}

// ---- fail-closed rules ----------------------------------------------------

/// Assert the error is a `Config` error whose message names the given variable.
fn assert_config_error_names(err: AppError, needle: &str) {
    match err {
        AppError::Config(message) => assert!(
            message.contains(needle),
            "error must name {needle}, got: {message}"
        ),
        other => panic!("expected a Config error, got {other:?}"),
    }
}

#[test]
fn missing_base_url_and_fallback_fails_closed() {
    let err = from_vars(&vars(&[
        ("FKST_CHAT_ENABLED", "true"),
        ("FKST_LLM_API_KEY", "k"),
        ("FKST_LLM_MODEL", "m"),
    ]))
    .expect_err("a URL-less enabled block must fail closed");
    assert_config_error_names(err, "FKST_CHAT_BASE_URL");
}

#[test]
fn garbage_base_url_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_BASE_URL", "not-a-url")]));
    let err = from_vars(&pairs).expect_err("a malformed URL must fail closed");
    assert_config_error_names(err, "FKST_CHAT_BASE_URL");
}

#[test]
fn blank_key_with_no_fallback_fails_closed() {
    let err = from_vars(&vars(&[
        ("FKST_CHAT_ENABLED", "true"),
        ("FKST_CHAT_BASE_URL", "https://chat.example/v1"),
        ("FKST_CHAT_API_KEY", "  "),
        ("FKST_CHAT_MODEL", "m"),
    ]))
    .expect_err("a key-less enabled block must fail closed");
    assert_config_error_names(err, "FKST_CHAT_API_KEY");
}

#[test]
fn missing_model_and_fallback_fails_closed() {
    let err = from_vars(&vars(&[
        ("FKST_CHAT_ENABLED", "true"),
        ("FKST_CHAT_BASE_URL", "https://chat.example/v1"),
        ("FKST_CHAT_API_KEY", "k"),
    ]))
    .expect_err("a model-less enabled block must fail closed");
    assert_config_error_names(err, "FKST_CHAT_MODEL");
}

#[test]
fn zero_tool_iterations_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_MAX_TOOL_ITERATIONS", "0")]));
    let err = from_vars(&pairs).expect_err("zero iterations must fail closed");
    assert_config_error_names(err, "FKST_CHAT_MAX_TOOL_ITERATIONS");
}

#[test]
fn too_short_turn_deadline_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_TURN_DEADLINE_SECS", "5")]));
    let err = from_vars(&pairs).expect_err("a 5s deadline must fail closed");
    assert_config_error_names(err, "FKST_CHAT_TURN_DEADLINE_SECS");
}

#[test]
fn zero_concurrency_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_MAX_CONCURRENT_TURNS", "0")]));
    let err = from_vars(&pairs).expect_err("zero concurrency must fail closed");
    assert_config_error_names(err, "FKST_CHAT_MAX_CONCURRENT_TURNS");
}

#[test]
fn too_small_history_cap_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_HISTORY_MAX_MESSAGES", "1")]));
    let err = from_vars(&pairs).expect_err("a 1-message cap must fail closed");
    assert_config_error_names(err, "FKST_CHAT_HISTORY_MAX_MESSAGES");
}

#[test]
fn too_small_request_cap_fails_closed() {
    let mut pairs = enabled_via_fallback();
    pairs.extend(vars(&[("FKST_CHAT_REQUEST_MAX_BYTES", "1024")]));
    let err = from_vars(&pairs).expect_err("a 1 KiB body cap must fail closed");
    assert_config_error_names(err, "FKST_CHAT_REQUEST_MAX_BYTES");
}

// ---- secret hygiene -------------------------------------------------------

#[test]
fn debug_redacts_the_api_key() {
    let config = from_vars(&enabled_via_fallback())
        .expect("must parse")
        .expect("must be enabled");
    let rendered = format!("{config:?}");
    assert!(
        rendered.contains("<redacted>"),
        "Debug must redact the key: {rendered}"
    );
    assert!(
        !rendered.contains("llm-key"),
        "Debug must never render the key: {rendered}"
    );
}
