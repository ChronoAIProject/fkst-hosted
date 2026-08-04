//! Relay configuration: required credentials, bounds, retention ordering, and
//! the redaction of both tokens.

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    let mut vars = vec![
        (
            "FKST_AUDIT_RELAY_WRITE_TOKEN".to_string(),
            "write-secret".to_string(),
        ),
        (
            "FKST_AUDIT_RELAY_READ_TOKEN".to_string(),
            "read-secret".to_string(),
        ),
    ];
    for (key, value) in pairs {
        vars.retain(|(existing, _)| existing != key);
        vars.push(((*key).to_string(), (*value).to_string()));
    }
    vars
}

#[test]
fn the_defaults_load() {
    let config = RelayConfig::from_vars(&vars(&[])).expect("defaults load");
    assert_eq!(config.bind_addr.to_string(), "0.0.0.0:8090");
    assert_eq!(
        config.db_path,
        std::path::PathBuf::from("/var/lib/fkst-audit/audit.sqlite3")
    );
    assert_eq!(config.max_body_bytes, 65_536);
    assert_eq!(config.verification_delay_secs, 30);
    assert_eq!(config.verified_retention_days, 7);
    assert_eq!(config.audit_retention_days, 90);
    assert!(!config.capture_configured());
    assert!(!config.verification_configured());
}

#[test]
fn both_tokens_are_required() {
    let missing_write: Vec<(String, String)> = vars(&[])
        .into_iter()
        .filter(|(key, _)| key != "FKST_AUDIT_RELAY_WRITE_TOKEN")
        .collect();
    let error = RelayConfig::from_vars(&missing_write).expect_err("write token required");
    assert!(error.to_string().contains("FKST_AUDIT_RELAY_WRITE_TOKEN"));

    let missing_read: Vec<(String, String)> = vars(&[])
        .into_iter()
        .filter(|(key, _)| key != "FKST_AUDIT_RELAY_READ_TOKEN")
        .collect();
    let error = RelayConfig::from_vars(&missing_read).expect_err("read token required");
    assert!(error.to_string().contains("FKST_AUDIT_RELAY_READ_TOKEN"));
}

#[test]
fn one_value_for_both_tokens_is_refused() {
    let error = RelayConfig::from_vars(&vars(&[("FKST_AUDIT_RELAY_READ_TOKEN", "write-secret")]))
        .expect_err("identical tokens are refused");
    assert!(error.to_string().contains("must differ"));
}

#[test]
fn retention_windows_cannot_be_inverted() {
    let error = RelayConfig::from_vars(&vars(&[
        ("FKST_AUDIT_RELAY_VERIFIED_RETENTION_DAYS", "30"),
        ("FKST_AUDIT_RELAY_AUDIT_RETENTION_DAYS", "7"),
    ]))
    .expect_err("inverted retention is refused");
    assert!(error.to_string().contains("AUDIT_RETENTION_DAYS"));
}

#[test]
fn the_verification_lag_window_cannot_be_inverted() {
    let error = RelayConfig::from_vars(&vars(&[
        ("FKST_AUDIT_RELAY_VERIFICATION_DELAY_SECS", "600"),
        ("FKST_AUDIT_RELAY_VERIFICATION_MAX_AGE_SECS", "60"),
    ]))
    .expect_err("inverted verification window is refused");
    assert!(error.to_string().contains("VERIFICATION_MAX_AGE_SECS"));
}

#[test]
fn the_retry_window_cannot_be_inverted() {
    let error = RelayConfig::from_vars(&vars(&[
        ("FKST_AUDIT_RELAY_RETRY_INITIAL_SECS", "300"),
        ("FKST_AUDIT_RELAY_RETRY_MAX_SECS", "5"),
    ]))
    .expect_err("inverted retry window is refused");
    assert!(error.to_string().contains("RETRY_MAX_SECS"));
}

#[test]
fn a_body_limit_below_the_floor_is_refused() {
    let error = RelayConfig::from_vars(&vars(&[("FKST_AUDIT_RELAY_MAX_BODY_BYTES", "16")]))
        .expect_err("a tiny body limit is refused");
    assert!(error.to_string().contains("MAX_BODY_BYTES"));
}

#[test]
fn a_malformed_bind_address_is_refused() {
    let error = RelayConfig::from_vars(&vars(&[("FKST_AUDIT_RELAY_BIND_ADDR", "not-an-address")]))
        .expect_err("a malformed bind address is refused");
    assert!(error.to_string().contains("BIND_ADDR"));
}

#[test]
fn capture_and_verification_are_configured_independently() {
    let capture_only = RelayConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_HOST", "https://posthog.example"),
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_write"),
    ]))
    .expect("capture-only config loads");
    assert!(capture_only.capture_configured());
    // Without a project id AND a query key nothing can be VERIFIED, and the
    // relay must not pretend otherwise.
    assert!(!capture_only.verification_configured());
    assert_eq!(capture_only.query_url(), None);

    let both = RelayConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_HOST", "https://posthog.example/"),
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_write"),
        ("FKST_POSTHOG_PROJECT_ID", "42"),
        ("FKST_POSTHOG_QUERY_API_KEY", "phx_read"),
    ]))
    .expect("full config loads");
    assert!(both.verification_configured());
    assert_eq!(
        both.query_url().as_deref(),
        Some("https://posthog.example/api/projects/42/query/")
    );
}

#[test]
fn the_delivery_host_is_judged_by_the_shared_rule() {
    // Plaintext ships the project capture token in the clear, so it needs the
    // same explicit test/local opt-in the control plane demands.
    let error = RelayConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_HOST", "http://posthog.internal"),
        ("FKST_DEPLOYMENT_ENVIRONMENT", "production"),
    ]))
    .expect_err("a plaintext production host must fail closed");
    assert!(error.to_string().contains("https"), "{error}");

    let local = RelayConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_HOST", "http://127.0.0.1:8000/"),
        ("FKST_DEPLOYMENT_ENVIRONMENT", "local"),
    ]))
    .expect("an explicitly local deployment may use plaintext");
    assert_eq!(local.posthog_host.as_deref(), Some("http://127.0.0.1:8000"));

    // Userinfo would put a credential in the relay ConfigMap, which is the one
    // object in this deployment guaranteed to hold none.
    let error = RelayConfig::from_vars(&vars(&[(
        "FKST_POSTHOG_HOST",
        "https://svc:phc_canary_do_not_leak@posthog.example",
    )]))
    .expect_err("an embedded credential must fail closed");
    assert!(error.to_string().contains("userinfo"), "{error}");
    assert!(
        !format!("{error:?}").contains("phc_canary_do_not_leak"),
        "the rejection leaked the credential"
    );

    // No host at all stays the supported outbox-only shape.
    let outbox_only = RelayConfig::from_vars(&vars(&[])).expect("an unset host is not an error");
    assert_eq!(outbox_only.posthog_host, None);
    assert!(!outbox_only.capture_configured());
}

#[test]
fn debug_output_redacts_every_credential() {
    let config = RelayConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_write_canary"),
        ("FKST_POSTHOG_QUERY_API_KEY", "phx_read_canary"),
    ]))
    .expect("config loads");
    let rendered = format!("{config:?}");
    for canary in [
        "write-secret",
        "read-secret",
        "phc_write_canary",
        "phx_read_canary",
    ] {
        assert!(
            !rendered.contains(canary),
            "`{canary}` must not appear in Debug output"
        );
    }
    assert!(rendered.contains("<redacted>"));
}
