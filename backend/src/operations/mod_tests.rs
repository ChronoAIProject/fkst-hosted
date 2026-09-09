//! Unit tests for the operations-state wiring: which combinations of
//! `FKST_POSTHOG_*` produce a usable activity source, which produce an honest
//! `503`, and which are an operator mistake the deploy must refuse.

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

fn audit_with_host(host: &str) -> AuditConfig {
    // The host is staged, not enabled: a control plane that captures through the
    // relay never sets FKST_POSTHOG_ENABLED, yet still needs the host to READ.
    AuditConfig::from_vars(&vars(&[("FKST_POSTHOG_HOST", host)])).expect("staged host parses")
}

fn query_configured() -> ActivityQueryConfig {
    ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_ID", "42"),
        ("FKST_POSTHOG_QUERY_API_KEY", "phx_not_a_real_key"),
    ]))
    .expect("both read credentials parse")
}

#[test]
fn a_host_plus_both_read_credentials_wires_the_source() {
    let state = OperationsState::from_config(
        &audit_with_host("https://posthog.example.invalid"),
        &query_configured(),
    )
    .expect("a fully configured read path builds");
    assert!(
        state.is_configured(),
        "host + project id + query key must produce an answerable source"
    );
}

#[test]
fn missing_read_credentials_stay_a_disabled_endpoint_not_a_boot_failure() {
    let state = OperationsState::from_config(
        &audit_with_host("https://posthog.example.invalid"),
        &ActivityQueryConfig::default(),
    )
    .expect("capture must keep working while the read secret is staged");
    assert!(!state.is_configured());
}

#[test]
fn no_host_and_no_read_credentials_is_simply_disabled() {
    let state =
        OperationsState::from_config(&AuditConfig::default(), &ActivityQueryConfig::default())
            .expect("a deployment that never adopted the trace must still boot");
    assert!(!state.is_configured());
}

#[test]
fn read_credentials_without_a_host_fail_the_deploy() {
    let error = OperationsState::from_config(&AuditConfig::default(), &query_configured())
        .expect_err("a query pair with nowhere to query must fail closed");
    let message = error.to_string();
    assert!(
        message.contains("FKST_POSTHOG_HOST"),
        "the error must name the missing variable: {message}"
    );
    assert!(
        !message.contains("phx_"),
        "the error must never echo a credential: {message}"
    );
}
