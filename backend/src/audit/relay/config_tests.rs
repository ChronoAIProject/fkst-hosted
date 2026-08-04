//! Delivery-configuration tests: mode parsing, the fail-closed pairing rules,
//! bounds, and credential redaction.

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| ((*key).to_string(), (*value).to_string()))
        .collect()
}

#[test]
fn the_default_mode_is_disabled_and_needs_no_relay() {
    let config = AuditDeliveryConfig::from_vars(&[]).expect("defaults load");
    assert_eq!(config.mode, AuditDeliveryMode::Disabled);
    assert!(!config.mode.uses_relay());
    assert!(!config.write_configured());
    assert!(!config.read_configured());
    assert_eq!(config.start_timeout_ms, 1_000);
    assert_eq!(config.completion_timeout_ms, 5_000);
}

#[test]
fn every_mode_spelling_round_trips() {
    for mode in AuditDeliveryMode::ALL {
        assert_eq!(AuditDeliveryMode::parse(mode.as_str()), Some(mode));
    }
    assert_eq!(
        AuditDeliveryMode::parse("Required"),
        Some(AuditDeliveryMode::Required)
    );
    assert_eq!(AuditDeliveryMode::parse("mandatory"), None);
}

#[test]
fn an_unknown_mode_is_a_startup_failure() {
    let error = AuditDeliveryConfig::from_vars(&vars(&[("FKST_AUDIT_DELIVERY_MODE", "mandatory")]))
        .expect_err("an unknown mode is refused");
    assert!(error.to_string().contains("FKST_AUDIT_DELIVERY_MODE"));
}

#[test]
fn a_relay_mode_without_a_url_or_token_fails_closed() {
    for mode in ["best_effort", "required"] {
        let error = AuditDeliveryConfig::from_vars(&vars(&[("FKST_AUDIT_DELIVERY_MODE", mode)]))
            .expect_err("a relay mode needs a URL");
        assert!(error.to_string().contains("FKST_AUDIT_RELAY_URL"));

        let error = AuditDeliveryConfig::from_vars(&vars(&[
            ("FKST_AUDIT_DELIVERY_MODE", mode),
            ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090"),
        ]))
        .expect_err("a relay mode needs a write token");
        assert!(error.to_string().contains("FKST_AUDIT_RELAY_WRITE_TOKEN"));
    }
}

#[test]
fn a_fully_configured_required_deployment_loads() {
    let config = AuditDeliveryConfig::from_vars(&vars(&[
        ("FKST_AUDIT_DELIVERY_MODE", "required"),
        ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090/"),
        ("FKST_AUDIT_RELAY_WRITE_TOKEN", "write-secret"),
        ("FKST_AUDIT_RELAY_READ_TOKEN", "read-secret"),
        ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "45"),
    ]))
    .expect("a required deployment loads");
    assert_eq!(config.mode, AuditDeliveryMode::Required);
    assert_eq!(
        config.relay_url.as_deref(),
        Some("http://relay.internal:8090")
    );
    assert!(config.write_configured());
    assert!(config.read_configured());
    assert_eq!(config.incomplete_grace_secs, 45);
}

#[test]
fn the_read_half_is_configured_independently_of_the_mode() {
    // A `disabled` deployment may still merge relay rows into activity.
    let config = AuditDeliveryConfig::from_vars(&vars(&[
        ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090"),
        ("FKST_AUDIT_RELAY_READ_TOKEN", "read-secret"),
    ]))
    .expect("read-only configuration loads");
    assert_eq!(config.mode, AuditDeliveryMode::Disabled);
    assert!(config.read_configured());
    assert!(!config.write_configured());
}

#[test]
fn a_url_with_embedded_credentials_is_refused_even_when_disabled() {
    let error = AuditDeliveryConfig::from_vars(&vars(&[(
        "FKST_AUDIT_RELAY_URL",
        "http://user:pass@relay.internal:8090",
    )]))
    .expect_err("embedded userinfo is refused");
    assert!(error.to_string().contains("userinfo"));
}

#[test]
fn a_non_http_url_is_refused() {
    let error =
        AuditDeliveryConfig::from_vars(&vars(&[("FKST_AUDIT_RELAY_URL", "ftp://relay.internal")]))
            .expect_err("a non-http scheme is refused");
    assert!(error.to_string().contains("http(s)"));
}

#[test]
fn budgets_are_bounded_unconditionally() {
    for (var, value) in [
        ("FKST_AUDIT_RELAY_START_TIMEOUT_MS", "0"),
        ("FKST_AUDIT_RELAY_START_TIMEOUT_MS", "60000"),
        ("FKST_AUDIT_RELAY_COMPLETION_TIMEOUT_MS", "0"),
        ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "0"),
    ] {
        let error =
            AuditDeliveryConfig::from_vars(&vars(&[(var, value)])).unwrap_err_for_test(var, value);
        assert!(error.to_string().contains(var), "{var}={value}");
    }
}

/// A tiny helper so the loop above reads as one assertion per case.
trait UnwrapErrForTest {
    fn unwrap_err_for_test(self, var: &str, value: &str) -> crate::error::AppError;
}

impl UnwrapErrForTest for Result<AuditDeliveryConfig, crate::error::AppError> {
    fn unwrap_err_for_test(self, var: &str, value: &str) -> crate::error::AppError {
        match self {
            Ok(_) => panic!("{var}={value} must be refused"),
            Err(error) => error,
        }
    }
}

#[test]
fn debug_output_redacts_both_credentials() {
    let config = AuditDeliveryConfig::from_vars(&vars(&[
        ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090"),
        ("FKST_AUDIT_RELAY_WRITE_TOKEN", "canary-write-token"),
        ("FKST_AUDIT_RELAY_READ_TOKEN", "canary-read-token"),
    ]))
    .expect("config loads");
    let rendered = format!("{config:?}");
    assert!(!rendered.contains("canary-write-token"));
    assert!(!rendered.contains("canary-read-token"));
    assert!(rendered.contains("<redacted>"));
}

#[test]
fn a_grace_shorter_than_the_request_budget_is_refused_for_a_relay_mode() {
    // The failure this prevents: a 200s request against a 120s force-close
    // horizon. The relay would synthesize `incomplete` while the handler is
    // still running, the real completion would then conflict, and the caller
    // would get a `503` for work that actually succeeded — a fabricated terminal
    // state, which the epic forbids in either direction.
    let config = AuditDeliveryConfig::from_vars(&vars(&[
        ("FKST_AUDIT_DELIVERY_MODE", "required"),
        ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090"),
        ("FKST_AUDIT_RELAY_WRITE_TOKEN", "write-secret"),
        ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "60"),
    ]))
    .expect("config loads");
    let error = config
        .ensure_grace_covers(std::time::Duration::from_secs(360))
        .expect_err("60s cannot cover a 360s request");
    let rendered = error.to_string();
    assert!(
        rendered.contains("FKST_AUDIT_INCOMPLETE_GRACE_SECS"),
        "{rendered}"
    );
    assert!(
        rendered.contains("390"),
        "the minimum must be stated: {rendered}"
    );
}

#[test]
fn a_grace_that_covers_the_budget_plus_the_margin_is_accepted() {
    let config = AuditDeliveryConfig::from_vars(&vars(&[
        ("FKST_AUDIT_DELIVERY_MODE", "required"),
        ("FKST_AUDIT_RELAY_URL", "http://relay.internal:8090"),
        ("FKST_AUDIT_RELAY_WRITE_TOKEN", "write-secret"),
        ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "390"),
    ]))
    .expect("config loads");
    config
        .ensure_grace_covers(std::time::Duration::from_secs(360))
        .expect("exactly budget + margin is enough");
}

#[test]
fn a_disabled_deployment_is_never_failed_over_an_inert_grace() {
    // `disabled` writes no `completion_deadline_at` at all, so its grace cannot
    // close anything. Failing an unrelated deploy over it would be noise.
    let config =
        AuditDeliveryConfig::from_vars(&vars(&[("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "5")]))
            .expect("config loads");
    config
        .ensure_grace_covers(std::time::Duration::from_secs(3_600))
        .expect("an inert grace is not a deployment error");
}
