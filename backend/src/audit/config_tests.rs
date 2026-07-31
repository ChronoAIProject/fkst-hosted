//! `FKST_POSTHOG_*` / `FKST_DEPLOYMENT_ENVIRONMENT` parsing and fail-closed
//! validation, plus the redacted-`Debug` guarantee for the project token.

use super::*;
use secrecy::ExposeSecret;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// A minimal enabled configuration; individual tests override or drop one key.
fn enabled(extra: &[(&str, &str)]) -> Vec<(String, String)> {
    crate::audit::test_support::merge_vars(
        &[
            ("FKST_POSTHOG_ENABLED", "true"),
            ("FKST_POSTHOG_HOST", "https://posthog.example"),
            ("FKST_POSTHOG_PROJECT_TOKEN", "phc_token"),
        ],
        extra,
    )
}

#[test]
fn disabled_defaults_apply_when_nothing_is_set() {
    let config = AuditConfig::from_vars(&vars(&[])).expect("empty env is valid");
    assert!(!config.enabled);
    assert_eq!(config.host, None);
    assert!(config.project_token.expose_secret().is_empty());
    assert_eq!(config.capture_timeout_ms, 2_000);
    assert_eq!(config.batch_size, 100);
    assert_eq!(config.flush_interval_ms, 1_000);
    assert_eq!(config.queue_capacity, 10_000);
    assert_eq!(config.max_retries, 5);
    assert_eq!(config.retry_initial_ms, 100);
    assert_eq!(config.retry_max_ms, 5_000);
    assert_eq!(config.shutdown_flush_secs, 10);
    assert_eq!(config.max_event_bytes, 65_536);
    assert_eq!(config.environment, "");
    assert_eq!(config.service_version, env!("CARGO_PKG_VERSION"));
    // The documented defaults must equal `Default`, so a test fixture and a real
    // empty environment can never diverge.
    let fallback = AuditConfig::default();
    assert_eq!(config.batch_size, fallback.batch_size);
    assert_eq!(config.queue_capacity, fallback.queue_capacity);
    assert_eq!(config.max_event_bytes, fallback.max_event_bytes);
}

#[test]
fn enabled_config_parses_and_exposes_capture_urls() {
    let config = AuditConfig::from_vars(&enabled(&[
        ("FKST_DEPLOYMENT_ENVIRONMENT", "production"),
        ("FKST_POSTHOG_BATCH_SIZE", "25"),
        ("FKST_POSTHOG_QUEUE_CAPACITY", "500"),
    ]))
    .expect("valid enabled config");
    assert!(config.enabled);
    assert_eq!(config.host.as_deref(), Some("https://posthog.example"));
    assert_eq!(config.project_token.expose_secret(), "phc_token");
    assert_eq!(config.environment, "production");
    assert_eq!(config.batch_size, 25);
    assert_eq!(
        config.capture_url().as_deref(),
        Some("https://posthog.example/capture/")
    );
    assert_eq!(
        config.batch_url().as_deref(),
        Some("https://posthog.example/batch/")
    );
}

#[test]
fn enabled_requires_host_and_project_token() {
    let missing_host: Vec<(String, String)> = enabled(&[])
        .into_iter()
        .filter(|(k, _)| k != "FKST_POSTHOG_HOST")
        .collect();
    let err = AuditConfig::from_vars(&missing_host).expect_err("host is required when enabled");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_POSTHOG_HOST"), "{err}");

    let missing_token: Vec<(String, String)> = enabled(&[])
        .into_iter()
        .filter(|(k, _)| k != "FKST_POSTHOG_PROJECT_TOKEN")
        .collect();
    let err = AuditConfig::from_vars(&missing_token).expect_err("token is required when enabled");
    assert!(
        err.to_string().contains("FKST_POSTHOG_PROJECT_TOKEN"),
        "{err}"
    );
}

#[test]
fn blank_host_and_token_are_treated_as_unset() {
    // A blank ConfigMap value must not masquerade as a configured host.
    let err = AuditConfig::from_vars(&enabled(&[
        ("FKST_POSTHOG_HOST", "   "),
        ("FKST_POSTHOG_PROJECT_TOKEN", ""),
    ]))
    .expect_err("blank values fail exactly like unset ones");
    assert!(err.to_string().contains("FKST_POSTHOG_HOST"), "{err}");
}

#[test]
fn one_trailing_slash_is_normalized_away() {
    let config = AuditConfig::from_vars(&enabled(&[(
        "FKST_POSTHOG_HOST",
        "https://posthog.example/",
    )]))
    .expect("trailing slash is normalized");
    assert_eq!(config.host.as_deref(), Some("https://posthog.example"));
    assert_eq!(
        config.batch_url().as_deref(),
        Some("https://posthog.example/batch/")
    );
}

#[test]
fn a_path_prefixed_host_keeps_its_prefix() {
    // Self-hosted PostHog behind a path prefix must compose correctly.
    let config = AuditConfig::from_vars(&enabled(&[(
        "FKST_POSTHOG_HOST",
        "https://obs.example/posthog/",
    )]))
    .expect("prefixed host is valid");
    assert_eq!(
        config.capture_url().as_deref(),
        Some("https://obs.example/posthog/capture/")
    );
}

#[test]
fn plaintext_host_is_rejected_outside_test_and_local() {
    for environment in ["production", "staging", ""] {
        let err = AuditConfig::from_vars(&enabled(&[
            ("FKST_POSTHOG_HOST", "http://posthog.internal"),
            ("FKST_DEPLOYMENT_ENVIRONMENT", environment),
        ]))
        .expect_err("plaintext must fail closed outside test/local");
        let msg = err.to_string();
        assert!(msg.contains("FKST_POSTHOG_HOST"), "{environment}: {msg}");
        assert!(msg.contains("https"), "{environment}: {msg}");
    }
}

#[test]
fn plaintext_host_is_allowed_in_an_explicit_test_or_local_environment() {
    for environment in ["test", "Local"] {
        let config = AuditConfig::from_vars(&enabled(&[
            ("FKST_POSTHOG_HOST", "http://127.0.0.1:8000"),
            ("FKST_DEPLOYMENT_ENVIRONMENT", environment),
        ]))
        .unwrap_or_else(|e| panic!("{environment} must permit plaintext: {e}"));
        assert_eq!(config.host.as_deref(), Some("http://127.0.0.1:8000"));
    }
}

#[test]
fn embedded_userinfo_is_rejected() {
    let err = AuditConfig::from_vars(&enabled(&[(
        "FKST_POSTHOG_HOST",
        "https://user:pass@posthog.example",
    )]))
    .expect_err("userinfo must fail closed");
    assert!(err.to_string().contains("userinfo"), "{err}");
}

/// Assert the load failed, naming the case in the panic message.
fn expect_rejected(result: Result<AuditConfig, AppError>, case: &str) -> AppError {
    match result {
        Ok(_) => panic!("{case} must be rejected"),
        Err(error) => error,
    }
}

#[test]
fn a_malformed_or_non_http_host_is_rejected() {
    for bad in ["not a url", "ftp://posthog.example", "posthog.example"] {
        let err = expect_rejected(
            AuditConfig::from_vars(&enabled(&[("FKST_POSTHOG_HOST", bad)])),
            bad,
        );
        assert!(err.to_string().contains("FKST_POSTHOG_HOST"), "{bad}");
    }
}

#[test]
fn numeric_bounds_fail_closed_even_while_disabled() {
    // Validated unconditionally so a typo surfaces at deploy time rather than at
    // the moment someone flips the feature on in production.
    for (var, value) in [
        ("FKST_POSTHOG_CAPTURE_TIMEOUT_MS", "0"),
        ("FKST_POSTHOG_FLUSH_INTERVAL_MS", "0"),
        ("FKST_POSTHOG_RETRY_INITIAL_MS", "0"),
        ("FKST_POSTHOG_SHUTDOWN_FLUSH_SECS", "0"),
        ("FKST_POSTHOG_BATCH_SIZE", "0"),
        ("FKST_POSTHOG_QUEUE_CAPACITY", "0"),
        ("FKST_POSTHOG_MAX_EVENT_BYTES", "512"),
        ("FKST_POSTHOG_MAX_RETRIES", "21"),
    ] {
        let err = expect_rejected(
            AuditConfig::from_vars(&vars(&[(var, value)])),
            &format!("{var}={value}"),
        );
        assert!(err.to_string().contains(var), "{var}: {err}");
    }
}

#[test]
fn an_inverted_retry_window_is_rejected() {
    let err = AuditConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_RETRY_INITIAL_MS", "5000"),
        ("FKST_POSTHOG_RETRY_MAX_MS", "100"),
    ]))
    .expect_err("retry_max below retry_initial must fail closed");
    assert!(
        err.to_string().contains("FKST_POSTHOG_RETRY_MAX_MS"),
        "{err}"
    );
}

#[test]
fn a_batch_larger_than_the_queue_is_rejected() {
    let err = AuditConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_BATCH_SIZE", "200"),
        ("FKST_POSTHOG_QUEUE_CAPACITY", "100"),
    ]))
    .expect_err("an unfillable batch must fail closed");
    assert!(err.to_string().contains("FKST_POSTHOG_BATCH_SIZE"), "{err}");
}

#[test]
fn a_non_numeric_knob_fails_closed_naming_the_block() {
    let err = AuditConfig::from_vars(&vars(&[("FKST_POSTHOG_BATCH_SIZE", "many")]))
        .expect_err("a non-numeric value must fail closed");
    assert!(err.to_string().contains("FKST_POSTHOG"), "{err}");
}

#[test]
fn a_staged_host_does_not_fail_a_disabled_deploy() {
    // Feature off: the host is kept (normalized) but never judged, so a
    // half-prepared rollout cannot break an unrelated deploy.
    let config = AuditConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_HOST", "http://posthog.internal/"),
        ("FKST_DEPLOYMENT_ENVIRONMENT", "production"),
    ]))
    .expect("a staged host is inert while disabled");
    assert!(!config.enabled);
    assert_eq!(config.host.as_deref(), Some("http://posthog.internal"));
}

#[test]
fn debug_output_redacts_the_project_token() {
    let config = AuditConfig::from_vars(&enabled(&[(
        "FKST_POSTHOG_PROJECT_TOKEN",
        "phc_canary_do_not_leak",
    )]))
    .expect("valid");
    let debug = format!("{config:?}");
    assert!(
        !debug.contains("phc_canary_do_not_leak"),
        "Debug leaked the project token: {debug}"
    );
    assert!(debug.contains("<redacted>"), "{debug}");
    // Non-secret fields stay visible for diagnostics.
    assert!(debug.contains("posthog.example"), "{debug}");
}
