//! Unit tests for the activity-query configuration: defaults, bounds, the
//! never-reuse-the-ingestion-token rule, and secret hygiene.

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_string(), (*v).to_string()))
        .collect()
}

#[test]
fn an_empty_environment_loads_the_documented_defaults_and_is_unconfigured() {
    let config = ActivityQueryConfig::from_vars(&vars(&[])).expect("defaults load");
    assert_eq!(config.query_timeout_ms, 5_000);
    assert_eq!(config.activity_max_range_days, 30);
    assert_eq!(config.activity_default_limit, 100);
    assert_eq!(config.activity_max_limit, 200);
    assert!(
        !config.is_configured(),
        "no project id and no read key means the endpoint must answer 503"
    );
}

#[test]
fn both_read_credentials_are_required_before_the_endpoint_is_configured() {
    let only_project = ActivityQueryConfig::from_vars(&vars(&[("FKST_POSTHOG_PROJECT_ID", "42")]))
        .expect("project id alone parses");
    assert!(!only_project.is_configured());

    let only_key =
        ActivityQueryConfig::from_vars(&vars(&[("FKST_POSTHOG_QUERY_API_KEY", "phx_1")]))
            .expect("key alone parses");
    assert!(!only_key.is_configured());

    let both = ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_ID", "42"),
        ("FKST_POSTHOG_QUERY_API_KEY", "phx_1"),
    ]))
    .expect("the pair parses");
    assert!(both.is_configured());
}

/// The ingestion token is a WRITE credential; it must never satisfy the read
/// path, because that would silently promote it to "reads the whole audit trail".
#[test]
fn the_ingestion_project_token_never_satisfies_the_read_credential() {
    let config = ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_ID", "42"),
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_ingestion_write_token"),
    ]))
    .expect("capture-only config parses");
    assert!(
        !config.is_configured(),
        "a capture token must not stand in for FKST_POSTHOG_QUERY_API_KEY"
    );
    assert!(config.query_api_key.expose_secret().is_empty());
}

#[test]
fn blank_values_are_treated_as_absent() {
    let config = ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_ID", "   "),
        ("FKST_POSTHOG_QUERY_API_KEY", "  "),
    ]))
    .expect("blank values parse");
    assert!(config.project_id.is_none());
    assert!(!config.is_configured());
}

#[test]
fn the_query_url_is_the_project_scoped_query_endpoint() {
    let config = ActivityQueryConfig::from_vars(&vars(&[("FKST_POSTHOG_PROJECT_ID", "42")]))
        .expect("project id parses");
    assert_eq!(
        config.query_url("https://posthog.example/").as_deref(),
        Some("https://posthog.example/api/projects/42/query/")
    );
    assert!(ActivityQueryConfig::default()
        .query_url("https://posthog.example")
        .is_none());
}

/// The id lands in a URL path segment, so escape characters are refused at
/// startup rather than sanitized per request.
#[test]
fn a_project_id_that_could_escape_its_url_segment_is_refused() {
    for bad in ["../../admin", "42/query", "4 2", "42?x=1", "42#frag"] {
        let error = ActivityQueryConfig::from_vars(&vars(&[("FKST_POSTHOG_PROJECT_ID", bad)]))
            .expect_err(bad);
        assert!(matches!(error, AppError::Config(_)), "{bad}: {error:?}");
    }
}

#[test]
fn numeric_bounds_are_validated_even_with_no_credentials_present() {
    for (var, value) in [
        ("FKST_POSTHOG_QUERY_TIMEOUT_MS", "0"),
        ("FKST_POSTHOG_QUERY_TIMEOUT_MS", "600000"),
        ("FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS", "0"),
        ("FKST_POSTHOG_ACTIVITY_MAX_RANGE_DAYS", "5000"),
        ("FKST_POSTHOG_ACTIVITY_MAX_LIMIT", "0"),
        ("FKST_POSTHOG_ACTIVITY_MAX_LIMIT", "100000"),
    ] {
        let error =
            ActivityQueryConfig::from_vars(&vars(&[(var, value)])).expect_err("{var}={value}");
        assert!(matches!(error, AppError::Config(_)), "{var}={value}");
    }
}

/// A default page larger than the maximum could never be served, so it is a
/// deploy-time error rather than a silent clamp.
#[test]
fn the_default_limit_may_not_exceed_the_maximum_limit() {
    let error = ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_ACTIVITY_DEFAULT_LIMIT", "500"),
        ("FKST_POSTHOG_ACTIVITY_MAX_LIMIT", "200"),
    ]))
    .expect_err("an unreachable default must fail closed");
    assert!(matches!(error, AppError::Config(_)), "{error:?}");
}

#[test]
fn debug_output_never_contains_the_read_key() {
    let config = ActivityQueryConfig::from_vars(&vars(&[
        ("FKST_POSTHOG_PROJECT_ID", "42"),
        ("FKST_POSTHOG_QUERY_API_KEY", "phx_super_secret_value"),
    ]))
    .expect("config parses");
    let rendered = format!("{config:?}");
    assert!(!rendered.contains("phx_super_secret_value"), "{rendered}");
    assert!(rendered.contains("<redacted>"), "{rendered}");
}
