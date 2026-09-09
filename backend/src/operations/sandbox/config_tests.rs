//! Unit tests for the sandbox-inventory configuration bounds.

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn an_unset_deployment_gets_the_documented_defaults() {
    let config = SandboxInventoryConfig::from_vars(&vars(&[])).expect("defaults parse");
    assert_eq!(config.max_result_items, 5_000);
    assert_eq!(config.timeout_ms, 5_000);
    assert_eq!(config.timeout(), std::time::Duration::from_millis(5_000));
    assert_eq!(config, SandboxInventoryConfig::default());
}

#[test]
fn explicit_values_are_read_from_their_documented_variables() {
    let config = SandboxInventoryConfig::from_vars(&vars(&[
        ("FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS", "250"),
        ("FKST_OPERATIONS_SANDBOX_TIMEOUT_MS", "1500"),
    ]))
    .expect("explicit values parse");
    assert_eq!(config.max_result_items, 250);
    assert_eq!(config.timeout_ms, 1_500);
}

/// A zero result ceiling would fail EVERY read as oversize, silently taking the
/// view down. It must be a deploy-time failure, not a runtime surprise.
#[test]
fn a_zero_or_oversized_ceiling_is_rejected_by_name() {
    for (key, value) in [
        ("FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS", "0"),
        ("FKST_OPERATIONS_SANDBOX_MAX_RESULT_ITEMS", "50001"),
        ("FKST_OPERATIONS_SANDBOX_TIMEOUT_MS", "0"),
        ("FKST_OPERATIONS_SANDBOX_TIMEOUT_MS", "60001"),
    ] {
        let error = SandboxInventoryConfig::from_vars(&vars(&[(key, value)]))
            .expect_err("the bound is enforced");
        let rendered = format!("{error}");
        assert!(rendered.contains(key), "{rendered}");
    }
}

#[test]
fn a_non_numeric_value_fails_closed_naming_the_family() {
    let error =
        SandboxInventoryConfig::from_vars(&vars(&[("FKST_OPERATIONS_SANDBOX_TIMEOUT_MS", "soon")]))
            .expect_err("a non-numeric budget is rejected");
    assert!(
        format!("{error}").contains("FKST_OPERATIONS_SANDBOX_*"),
        "{error}"
    );
}

/// The advisory must be measured against the ceiling the ROUTE runs under. The
/// `/api/v1` nest carries the environments-PUT budget, so a deployment with the
/// default 5s inventory budget is comfortably below it — and the check must fire
/// only when the budget genuinely reaches or exceeds that number.
#[test]
fn the_route_budget_is_compared_against_the_api_subtree_ceiling() {
    let ceiling = crate::router::api_subtree_timeout(&crate::env_config::EnvConfig::default());
    assert!(
        SandboxInventoryConfig::default().is_below(ceiling),
        "the shipped defaults must not warn: budget {:?} vs ceiling {ceiling:?}",
        SandboxInventoryConfig::default().timeout()
    );

    let ceiling_ms = u64::try_from(ceiling.as_millis()).expect("a representable ceiling");
    let exactly_at = SandboxInventoryConfig {
        timeout_ms: ceiling_ms,
        ..SandboxInventoryConfig::default()
    };
    assert!(
        !exactly_at.is_below(ceiling),
        "a budget EQUAL to the ceiling can never be observed, so it warns too"
    );
    let just_under = SandboxInventoryConfig {
        timeout_ms: ceiling_ms - 1,
        ..SandboxInventoryConfig::default()
    };
    assert!(just_under.is_below(ceiling));
    // The emitting half must stay total over both answers.
    exactly_at.warn_unless_below(ceiling);
    just_under.warn_unless_below(ceiling);
}
