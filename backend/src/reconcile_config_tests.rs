//! Tests for [`super::ReconcileConfig`]: envy defaults, per-knob overrides,
//! fail-closed bounds, and the I9 install-time seeding defaults (seed-on-install
//! ON by default + the default-workflows manifest, with blank-coercion to the
//! legacy body).

use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn defaults_apply_when_nothing_is_set() {
    let config = ReconcileConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
    assert_eq!(config.substrate_trigger_label, "fkst-substrate-trigger");
    assert_eq!(config.github_bot_login, None);
    assert_eq!(config.reconcile_interval_secs, 30);
    assert_eq!(config.pod_full_resync_interval_secs, 600);
    assert_eq!(config.startup_resync_retry_initial_secs, 5);
    assert_eq!(config.startup_resync_retry_max_secs, 60);
    assert_eq!(config.startup_resync_retry_jitter_percent, 20);
    assert_eq!(config.session_idle_grace_secs, 300);
    assert_eq!(config.pod_min_lifetime_secs, 120);
    assert_eq!(config.pod_termination_grace_secs, 60);
    assert_eq!(config.pod_token_refresh_secs, 2700);
    assert_eq!(config.pod_session_max_lifetime_secs, 0);
    assert_eq!(config.health_scrape_secs, 150);
    // R3 authority gate is OFF by default (today's permissive behavior).
    assert!(!config.enforce_work_issue_authz);
    // I9: install-time seeding is ON by default (behaviour change) and points
    // at the default-workflows manifest.
    assert!(config.seed_trigger_issue_on_install);
    assert_eq!(
        config.default_manifest.as_deref(),
        Some("ChronoAIProject/fkst-packages@fkst-hosted:manifests/default-workflows.json")
    );
    assert_eq!(
        config.seed_packages,
        vec!["ChronoAIProject/fkst-packages@dev:packages/github-devloop-workflow".to_string()]
    );
}

#[test]
fn seed_on_install_defaults_on_and_is_overridable_off() {
    // Unset → ON (the I9 behaviour change).
    let on = ReconcileConfig::from_vars(&vars(&[])).expect("defaults");
    assert!(on.seed_trigger_issue_on_install);
    // Explicitly disabled.
    let off = ReconcileConfig::from_vars(&vars(&[("FKST_SEED_TRIGGER_ISSUE_ON_INSTALL", "false")]))
        .expect("override");
    assert!(!off.seed_trigger_issue_on_install);
}

#[test]
fn default_manifest_overrides_and_blank_coerces_to_none() {
    // A custom manifest ref surfaces verbatim.
    let custom = ReconcileConfig::from_vars(&vars(&[(
        "FKST_DEFAULT_MANIFEST",
        "octo/pkgs@main:manifests/custom.json",
    )]))
    .expect("override");
    assert_eq!(
        custom.default_manifest.as_deref(),
        Some("octo/pkgs@main:manifests/custom.json")
    );
    // A blank/whitespace value disables the manifest-driven seed (→ legacy body).
    let blank =
        ReconcileConfig::from_vars(&vars(&[("FKST_DEFAULT_MANIFEST", "   ")])).expect("blank");
    assert_eq!(blank.default_manifest, None);
}

#[test]
fn enforce_work_issue_authz_is_opt_in() {
    // Unset → false (permissive), the only value that preserves pre-R3 behavior.
    let off = ReconcileConfig::from_vars(&vars(&[])).expect("defaults");
    assert!(!off.enforce_work_issue_authz);
    // Explicitly opted in.
    let on = ReconcileConfig::from_vars(&vars(&[("FKST_ENFORCE_WORK_ISSUE_AUTHZ", "true")]))
        .expect("override");
    assert!(on.enforce_work_issue_authz);
}

#[test]
fn default_impl_matches_env_defaults() {
    let from_env = ReconcileConfig::from_vars(&vars(&[])).expect("defaults");
    let from_default = ReconcileConfig::default();
    assert_eq!(
        from_default.substrate_trigger_label,
        from_env.substrate_trigger_label
    );
    assert_eq!(from_default.github_bot_login, from_env.github_bot_login);
    assert_eq!(
        from_default.reconcile_interval_secs,
        from_env.reconcile_interval_secs
    );
    assert_eq!(
        from_default.pod_full_resync_interval_secs,
        from_env.pod_full_resync_interval_secs
    );
    assert_eq!(
        from_default.startup_resync_retry_initial_secs,
        from_env.startup_resync_retry_initial_secs
    );
    assert_eq!(
        from_default.startup_resync_retry_max_secs,
        from_env.startup_resync_retry_max_secs
    );
    assert_eq!(
        from_default.startup_resync_retry_jitter_percent,
        from_env.startup_resync_retry_jitter_percent
    );
    assert_eq!(
        from_default.session_idle_grace_secs,
        from_env.session_idle_grace_secs
    );
    assert_eq!(
        from_default.pod_min_lifetime_secs,
        from_env.pod_min_lifetime_secs
    );
    assert_eq!(
        from_default.pod_termination_grace_secs,
        from_env.pod_termination_grace_secs
    );
    assert_eq!(
        from_default.pod_token_refresh_secs,
        from_env.pod_token_refresh_secs
    );
    assert_eq!(
        from_default.pod_session_max_lifetime_secs,
        from_env.pod_session_max_lifetime_secs
    );
    assert_eq!(from_default.health_scrape_secs, from_env.health_scrape_secs);
    assert_eq!(
        from_default.enforce_work_issue_authz,
        from_env.enforce_work_issue_authz
    );
    assert_eq!(
        from_default.seed_trigger_issue_on_install,
        from_env.seed_trigger_issue_on_install
    );
    assert_eq!(from_default.default_manifest, from_env.default_manifest);
    assert_eq!(from_default.seed_packages, from_env.seed_packages);
}

#[test]
fn every_knob_is_overridable() {
    let config = ReconcileConfig::from_vars(&vars(&[
        ("FKST_SUBSTRATE_TRIGGER_LABEL", "fkst-run"),
        ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
        ("FKST_RECONCILE_INTERVAL_SECS", "15"),
        ("FKST_POD_FULL_RESYNC_INTERVAL_SECS", "1200"),
        ("FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS", "7"),
        ("FKST_STARTUP_RESYNC_RETRY_MAX_SECS", "70"),
        ("FKST_STARTUP_RESYNC_RETRY_JITTER_PERCENT", "15"),
        ("FKST_SESSION_IDLE_GRACE_SECS", "600"),
        ("FKST_POD_MIN_LIFETIME_SECS", "240"),
        ("FKST_POD_TERMINATION_GRACE_SECS", "90"),
        ("FKST_POD_TOKEN_REFRESH_SECS", "1800"),
        ("FKST_POD_SESSION_MAX_LIFETIME_SECS", "86400"),
        ("FKST_HEALTH_SCRAPE_SECS", "90"),
    ]))
    .expect("overrides should deserialize");
    assert_eq!(config.substrate_trigger_label, "fkst-run");
    assert_eq!(config.github_bot_login.as_deref(), Some("fkst-bot"));
    assert_eq!(config.reconcile_interval_secs, 15);
    assert_eq!(config.pod_full_resync_interval_secs, 1200);
    assert_eq!(config.startup_resync_retry_initial_secs, 7);
    assert_eq!(config.startup_resync_retry_max_secs, 70);
    assert_eq!(config.startup_resync_retry_jitter_percent, 15);
    assert_eq!(config.session_idle_grace_secs, 600);
    assert_eq!(config.pod_min_lifetime_secs, 240);
    assert_eq!(config.pod_termination_grace_secs, 90);
    assert_eq!(config.pod_token_refresh_secs, 1800);
    assert_eq!(config.pod_session_max_lifetime_secs, 86400);
    assert_eq!(config.health_scrape_secs, 90);
}

#[test]
fn blank_bot_login_is_coerced_to_none() {
    let config =
        ReconcileConfig::from_vars(&vars(&[("FKST_GITHUB_BOT_LOGIN", "   ")])).expect("blank");
    assert_eq!(config.github_bot_login, None);
}

#[test]
fn zero_cadence_bounds_are_config_errors_naming_the_var() {
    for var in [
        "FKST_RECONCILE_INTERVAL_SECS",
        "FKST_POD_FULL_RESYNC_INTERVAL_SECS",
        "FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS",
        "FKST_SESSION_IDLE_GRACE_SECS",
        "FKST_POD_TOKEN_REFRESH_SECS",
        "FKST_HEALTH_SCRAPE_SECS",
    ] {
        let err = ReconcileConfig::from_vars(&vars(&[(var, "0")])).expect_err("zero must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(var), "error must name {var}");
    }
}

#[test]
fn startup_resync_retry_bounds_fail_closed() {
    let max_below_initial = ReconcileConfig::from_vars(&vars(&[
        ("FKST_STARTUP_RESYNC_RETRY_INITIAL_SECS", "10"),
        ("FKST_STARTUP_RESYNC_RETRY_MAX_SECS", "9"),
    ]))
    .expect_err("max below initial must fail");
    assert!(max_below_initial
        .to_string()
        .contains("FKST_STARTUP_RESYNC_RETRY_MAX_SECS"));

    let jitter = ReconcileConfig::from_vars(&vars(&[(
        "FKST_STARTUP_RESYNC_RETRY_JITTER_PERCENT",
        "101",
    )]))
    .expect_err("jitter above 100 must fail");
    assert!(jitter
        .to_string()
        .contains("FKST_STARTUP_RESYNC_RETRY_JITTER_PERCENT"));
}

#[test]
fn token_refresh_at_or_over_the_ttl_is_a_config_error() {
    // At the TTL boundary: a refresh that fires exactly at expiry is too late.
    let at = ReconcileConfig::from_vars(&vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "3600")]))
        .expect_err("at TTL must fail");
    assert!(at.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
    // Over the TTL.
    let over = ReconcileConfig::from_vars(&vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "7200")]))
        .expect_err("over TTL must fail");
    assert!(over.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
}

#[test]
fn zero_valued_shield_and_lifetime_knobs_are_allowed() {
    // A zero min-lifetime / termination-grace / max-lifetime are all valid
    // (no shield / no drain / unbounded) — they must NOT fail closed.
    let config = ReconcileConfig::from_vars(&vars(&[
        ("FKST_POD_MIN_LIFETIME_SECS", "0"),
        ("FKST_POD_TERMINATION_GRACE_SECS", "0"),
        ("FKST_POD_SESSION_MAX_LIFETIME_SECS", "0"),
    ]))
    .expect("zero shields are valid");
    assert_eq!(config.pod_min_lifetime_secs, 0);
    assert_eq!(config.pod_termination_grace_secs, 0);
    assert_eq!(config.pod_session_max_lifetime_secs, 0);
}

#[test]
fn non_numeric_interval_is_a_config_error() {
    let err = ReconcileConfig::from_vars(&vars(&[("FKST_RECONCILE_INTERVAL_SECS", "soon")]))
        .expect_err("non-numeric must fail");
    assert!(matches!(err, AppError::Config(_)));
}
