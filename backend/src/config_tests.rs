//! Unit tests for the top-level environment configuration.
//!
//! Kept in a sibling file rather than an inline `mod tests`, per the repo's
//! module convention and its file-size rule: the assertions are the larger half
//! of this module and were the reason `config.rs` read as one very long file.

use super::*;
use secrecy::ExposeSecret;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn defaults_apply_when_nothing_is_set() {
    // The control plane is datastore-free and has no application-level auth:
    // an otherwise-empty environment loads cleanly.
    let config = Config::from_vars(vars(&[])).expect("defaults should deserialize");
    assert_eq!(config.port, 8080);
    assert_eq!(config.bind_addr, "0.0.0.0");
    assert_eq!(config.log_level, "info");
    // Pod dispatch is OFF by default; the control plane never touches k8s.
    assert!(!config.pod.dispatch);
    assert_eq!(config.pod.namespace, "fkst-sessions");
    assert_eq!(config.pod.service_account, "fkst-session-runner");
    assert!(config.pod.image.is_none());
    assert_eq!(config.request_timeout_secs, 30);
    assert!(!config.leader.enabled);
    assert!(config.delivery_grants.is_empty());
}

#[test]
fn cross_repo_delivery_grants_are_startup_validated_and_wired() {
    let config = Config::from_vars(vars(&[(
        "FKST_CROSS_REPO_DELIVERY_GRANTS",
        r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":41,"implementation_repo":"acme/tools","implementation_branch":"main"}]"#,
    )]))
    .expect("valid exact grant");
    assert!(config
        .delivery_grants
        .find(
            &crate::models::RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            },
            41,
        )
        .is_some());

    let error = Config::from_vars(vars(&[(
        "FKST_CROSS_REPO_DELIVERY_GRANTS",
        r#"[{"lifecycle_repo":"acme/site","lifecycle_issue":41,"implementation_repo":"acme/tools","implementation_branch":"bad branch"}]"#,
    )]))
    .expect_err("unsafe branch must fail at startup");
    assert!(error
        .to_string()
        .contains("FKST_CROSS_REPO_DELIVERY_GRANTS"));
}

#[test]
fn leader_election_requires_dispatch() {
    let err = Config::from_vars(vars(&[
        ("FKST_LEADER_ELECTION_ENABLED", "true"),
        ("FKST_LEADER_IDENTITY", "pod-a"),
    ]))
    .expect_err("election without reconcile work must fail closed");
    assert!(err.to_string().contains("FKST_LEADER_ELECTION_ENABLED"));
    assert!(err.to_string().contains("FKST_POD_DISPATCH"));
}

#[test]
fn pod_dispatch_on_requires_an_image() {
    let err = Config::from_vars(vars(&[("FKST_POD_DISPATCH", "true")]))
        .expect_err("dispatch with no image must fail closed");
    assert!(err.to_string().contains("FKST_POD_IMAGE"));
}

#[test]
fn rate_pools_parse_into_the_pod_config() {
    let config = Config::from_vars(vars(&[("FKST_POD_RATE_POOLS", "GH=50,50 GIT_2=120,1")]))
        .expect("valid pools parse");
    assert_eq!(config.pod.rate_pools.len(), 2);
    assert_eq!(
        config.pod.rate_pools["GH"],
        RatePool {
            burst: 50,
            refill_per_minute: 50
        }
    );
    assert_eq!(
        config.pod.rate_pools["GIT_2"],
        RatePool {
            burst: 120,
            refill_per_minute: 1
        }
    );
}

#[test]
fn rate_pools_default_empty_when_unset_or_blank() {
    assert!(Config::from_vars(vars(&[]))
        .expect("unset ok")
        .pod
        .rate_pools
        .is_empty());
    assert!(Config::from_vars(vars(&[("FKST_POD_RATE_POOLS", "  ")]))
        .expect("blank ok")
        .pod
        .rate_pools
        .is_empty());
}

#[test]
fn rate_pools_fail_closed_on_every_malformed_token() {
    // Each variant names the env var — even with dispatch OFF, so a bad
    // operator value can never be silently dropped (an unthrottled session
    // is exactly what this knob exists to prevent).
    for bad in [
        "GH50,50",                   // missing `=`
        "gh=50,50",                  // lowercase NAME
        "ROOT=1,1",                  // reserved NAME
        "GH=0,5",                    // zero burst
        "GH=5,0",                    // zero refill
        "GH=5",                      // missing `,`
        "GH=18446744073709551616,1", // u64 overflow
        "GH=1,1 GH=2,2",             // duplicate NAME
    ] {
        let err = Config::from_vars(vars(&[("FKST_POD_RATE_POOLS", bad)])).expect_err(bad);
        assert!(
            err.to_string().contains("FKST_POD_RATE_POOLS"),
            "{bad}: {err}"
        );
    }
}

#[test]
fn pod_dispatch_on_with_image_parses_and_keeps_overrides() {
    let config = Config::from_vars(vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "registry/fkst-control-plane:1.0"),
        ("FKST_POD_NAMESPACE", "sessions-prod"),
        ("FKST_LLM_API_KEY", "sk-test"),
        // Required by the PR6 flip whenever dispatch is on.
        ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
    ]))
    .expect("valid dispatch config should load");
    assert!(config.pod.dispatch);
    assert_eq!(
        config.pod.image.as_deref(),
        Some("registry/fkst-control-plane:1.0")
    );
    assert_eq!(config.pod.namespace, "sessions-prod");
    assert_eq!(config.llm_api_key.expose_secret(), "sk-test");
}

#[test]
fn pod_dispatch_on_requires_an_llm_api_key() {
    let err = Config::from_vars(vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "img"),
    ]))
    .expect_err("dispatch with no llm api key must fail closed");
    assert!(err.to_string().contains("FKST_LLM_API_KEY"));
}

#[test]
fn pod_dispatch_on_requires_a_github_bot_login() {
    // The LLM key is set so this passes the earlier LLM check and reaches the
    // bot-login requirement the PR6 flip added.
    let err = Config::from_vars(vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "img"),
        ("FKST_LLM_API_KEY", "sk-test"),
    ]))
    .expect_err("dispatch with no bot login must fail closed");
    assert!(err.to_string().contains("FKST_GITHUB_BOT_LOGIN"));
}

#[test]
fn pod_dispatch_on_with_bot_login_and_key_loads() {
    // The full happy path: dispatch on with an image, an LLM key, and a bot
    // login loads cleanly and surfaces the login on the reconcile config.
    let config = Config::from_vars(vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "img"),
        ("FKST_LLM_API_KEY", "sk-test"),
        ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
    ]))
    .expect("valid dispatch config with a bot login should load");
    assert_eq!(
        config.reconcile.github_bot_login.as_deref(),
        Some("fkst-bot")
    );
}

#[test]
fn pod_image_blank_is_treated_as_absent_when_dispatch_on() {
    let err = Config::from_vars(vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "   "),
    ]))
    .expect_err("blank image must fail closed");
    assert!(err.to_string().contains("FKST_POD_IMAGE"));
}

// ---- pod DNS nameserver tests ---------------------------------------------

#[test]
fn pod_dns_nameservers_default() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.pod.dns_nameservers, vec!["1.1.1.1", "8.8.8.8"]);
}

#[test]
fn pod_dns_nameservers_override_is_split_and_trimmed() {
    let config = Config::from_vars(vars(&[("FKST_POD_DNS_NAMESERVERS", "9.9.9.9, 1.0.0.1")]))
        .expect("override");
    assert_eq!(config.pod.dns_nameservers, vec!["9.9.9.9", "1.0.0.1"]);
}

#[test]
fn blank_pod_dns_nameservers_is_a_config_error_naming_the_var() {
    let err = Config::from_vars(vars(&[("FKST_POD_DNS_NAMESERVERS", "   ")]))
        .expect_err("blank nameservers must fail");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_POD_DNS_NAMESERVERS"));
}

// ---- pod runtime-class (Kata) tests ---------------------------------------

#[test]
fn pod_runtime_class_defaults_to_none() {
    // Unset means the cluster default runtime (runc) — local docker-desktop
    // has no Kata RuntimeClass, so the default must not select one.
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.pod.runtime_class, None);
}

#[test]
fn pod_runtime_class_override_is_kept() {
    let config = Config::from_vars(vars(&[("FKST_POD_RUNTIME_CLASS", "kata")])).expect("override");
    assert_eq!(config.pod.runtime_class.as_deref(), Some("kata"));
}

#[test]
fn blank_pod_runtime_class_is_treated_as_none() {
    // A blank ConfigMap value must fall back to runc, not to an empty (and
    // therefore invalid) runtimeClassName.
    let config =
        Config::from_vars(vars(&[("FKST_POD_RUNTIME_CLASS", "   ")])).expect("blank runtime class");
    assert_eq!(config.pod.runtime_class, None);
}

// ---- pod mode (FKST_POD_MODE) tests ---------------------------------------

#[test]
fn pod_mode_defaults_to_k8s_customized() {
    // Unset means the k8s-customized backend — the only one shipped today.
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.pod.mode, PodMode::K8sCustomized);
}

#[test]
fn pod_mode_k8s_customized_is_accepted() {
    let config = Config::from_vars(vars(&[("FKST_POD_MODE", "k8s-customized")]))
        .expect("explicit k8s-customized");
    assert_eq!(config.pod.mode, PodMode::K8sCustomized);
}

#[test]
fn pod_mode_unknown_value_is_a_config_error_naming_the_var() {
    let err = Config::from_vars(vars(&[("FKST_POD_MODE", "bogus")]))
        .expect_err("unknown mode must fail closed");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains(
        "FKST_POD_MODE must be one of \"k8s-customized\" | \"opensandbox\" (got \"bogus\")"
    ));
}

/// A dispatch-on opensandbox environment: the shared dispatch requirements
/// (image / namespace / LLM key / bot login) plus the full `FKST_OSB_*` block.
fn opensandbox_dispatch_vars() -> Vec<(String, String)> {
    vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_MODE", "opensandbox"),
        ("FKST_POD_IMAGE", "registry/fkst-control-plane:1.0"),
        ("FKST_LLM_API_KEY", "sk-test"),
        ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
        ("FKST_OSB_BASE_URL", "https://sandbox.example/api"),
        ("FKST_OSB_API_KEY", "osb-key"),
        ("FKST_OSB_EXECD_TOKEN_SEED", "execd-seed"),
        ("FKST_OSB_SESSION_CPU", "500m"),
        ("FKST_OSB_SESSION_MEMORY", "512Mi"),
        ("FKST_OSB_ENTRYPOINT", "/usr/local/bin/fkst-control-plane"),
    ])
}

#[test]
fn opensandbox_dispatch_on_fully_configured_loads() {
    // The OpenSandbox backend is available: a dispatch-on opensandbox config with
    // a complete FKST_OSB_* block loads and surfaces the resolved OSB config.
    let config =
        Config::from_vars(opensandbox_dispatch_vars()).expect("full opensandbox config loads");
    assert_eq!(config.pod.mode, PodMode::Opensandbox);
    let osb = config.opensandbox.expect("opensandbox config present");
    assert_eq!(osb.base_url.as_str(), "https://sandbox.example/api");
    assert_eq!(osb.session_memory, "512Mi");
}

#[test]
fn opensandbox_dispatch_on_missing_osb_var_fails() {
    // Drop one required FKST_OSB_* var: the load fails closed with that var's
    // exact message (the OSB block is validated once opensandbox is selected).
    let missing_base: Vec<(String, String)> = opensandbox_dispatch_vars()
        .into_iter()
        .filter(|(k, _)| k != "FKST_OSB_BASE_URL")
        .collect();
    let err =
        Config::from_vars(missing_base).expect_err("missing FKST_OSB_BASE_URL must fail closed");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err
        .to_string()
        .contains("FKST_OSB_BASE_URL must be a valid URL when FKST_POD_MODE=opensandbox"));
}

#[test]
fn opensandbox_mode_rejects_cluster_internal_session_endpoints() {
    // The sandbox-lockdown egress policy blocks RFC1918 / cluster DNS, so an
    // in-cluster LLM URL handed to sessions must fail closed at startup —
    // naming the var — instead of black-holing mid-session.
    let mut v = opensandbox_dispatch_vars();
    v.push((
        "FKST_LLM_BASE_URL".to_string(),
        "http://llm.internal.svc.cluster.local/v1".to_string(),
    ));
    let err = Config::from_vars(v)
        .expect_err("cluster-internal LLM URL must fail closed in opensandbox mode");
    assert!(matches!(err, AppError::Config(_)));
    let msg = err.to_string();
    assert!(msg.contains("FKST_LLM_BASE_URL"), "{msg}");
    assert!(msg.contains("sandbox"), "{msg}");
}

#[test]
fn opensandbox_mode_vets_storage_urls_when_configured() {
    // The chrono-storage URLs ride the per-session creds files, so they are
    // sandbox-facing too: a private storage base URL fails closed by name.
    let mut v = opensandbox_dispatch_vars();
    v.extend(vars(&[
        ("FKST_STORAGE_BASE_URL", "http://minio.storage.svc"),
        ("FKST_STORAGE_BUCKET", "fkst-session-logs"),
        ("FKST_NYXID_TOKEN_URL", "https://nyx.example/oauth/token"),
        ("FKST_NYXID_CLIENT_ID", "cid"),
        ("FKST_NYXID_CLIENT_SECRET", "sec"),
    ]));
    let err = Config::from_vars(v)
        .expect_err("cluster-internal storage URL must fail closed in opensandbox mode");
    assert!(err.to_string().contains("FKST_STORAGE_BASE_URL"));
}

#[test]
fn access_allowlist_parses_and_defaults_open() {
    // Unset → open (backward compatible).
    let config = Config::from_vars(vars(&[])).expect("empty config loads");
    assert!(!config.access.enforced());
    assert!(config.access.allows(1, "anyone"));
    // Set → enforced with the parsed entries.
    let config = Config::from_vars(vars(&[("FKST_ACCESS_ALLOWED_USERS", "583231, @alice")]))
        .expect("config with allowlist loads");
    assert!(config.access.enforced());
    assert!(config.access.allows(583231, "x"));
    assert!(config.access.allows(2, "Alice"));
    assert!(!config.access.allows(2, "mallory"));
}

#[test]
fn global_admin_configuration_is_wired_into_the_access_policy() {
    let config = Config::from_vars(vars(&[
        ("FKST_AUTH_MODEL", "allowlist"),
        ("FKST_ACCESS_ALLOWED_USERS", "someone-else"),
        ("FKST_GLOBAL_ADMINS", "@ChronoAI-Shining"),
    ]))
    .expect("config with global admin loads");
    assert!(config.access.enforced());
    assert!(config.access.is_global_admin(9, "chronoai-shining"));
    assert!(config.access.allows(9, "CHRONOAI-SHINING"));
    assert_eq!(config.access.global_admin_count(), 1);
}

#[test]
fn auth_model_all_overrides_a_present_allowlist() {
    // FKST_AUTH_MODEL=all opens the service even with a populated list.
    let config = Config::from_vars(vars(&[
        ("FKST_ACCESS_ALLOWED_USERS", "583231"),
        ("FKST_AUTH_MODEL", "all"),
    ]))
    .expect("config with auth model loads");
    assert!(!config.access.enforced());
    assert!(config.access.allows(999, "mallory"));
}

#[test]
fn auth_model_denylist_is_wired_into_the_access_policy() {
    // FKST_AUTH_MODEL=denylist + FKST_ACCESS_BLOCKED_USERS: everyone but
    // the blocked users is admitted, end to end through Config::from_vars.
    let config = Config::from_vars(vars(&[
        ("FKST_AUTH_MODEL", "denylist"),
        ("FKST_ACCESS_BLOCKED_USERS", "583231, @Mallory"),
    ]))
    .expect("config with denylist loads");
    assert!(config.access.enforced());
    assert!(!config.access.allows(583231, "whoever"));
    assert!(!config.access.allows(2, "mallory"));
    assert!(config.access.allows(2, "alice"));
}

#[test]
fn bad_auth_model_fails_config_closed() {
    // A non-empty unrecognized FKST_AUTH_MODEL must fail the whole config
    // load, naming the var (threaded via `?` from AccessPolicy::from_vars).
    let err = Config::from_vars(vars(&[("FKST_AUTH_MODEL", "nope")]))
        .expect_err("bad auth model must fail closed");
    assert!(err.to_string().contains("FKST_AUTH_MODEL"));
}

#[test]
fn k8s_customized_mode_does_not_vet_session_endpoints() {
    // The SAME cluster-internal LLM URL loads fine under the default
    // k8s-customized mode — the egress gate applies ONLY in opensandbox mode
    // (k8s sessions get the repo's own NetworkPolicy with different rules).
    let v = vars(&[
        ("FKST_POD_DISPATCH", "true"),
        ("FKST_POD_IMAGE", "registry/fkst-control-plane:1.0"),
        ("FKST_LLM_API_KEY", "sk-test"),
        ("FKST_GITHUB_BOT_LOGIN", "fkst-bot"),
        (
            "FKST_LLM_BASE_URL",
            "http://llm.internal.svc.cluster.local/v1",
        ),
    ]);
    Config::from_vars(v).expect("k8s-customized mode does not vet sandbox endpoints");
}

#[test]
fn pod_mode_opensandbox_with_dispatch_off_is_allowed() {
    // With dispatch off the FKST_OSB_* block is not validated, so an operator can
    // stage the mode ahead of turning dispatch on; `opensandbox` stays `None`.
    let config = Config::from_vars(vars(&[("FKST_POD_MODE", "opensandbox")]))
        .expect("opensandbox parses when dispatch is off");
    assert_eq!(config.pod.mode, PodMode::Opensandbox);
    assert!(config.opensandbox.is_none());
}

#[test]
fn no_mongodb_var_is_required_at_startup() {
    // Regression guard: with no MONGODB_URI set, the store-free control plane
    // must still load — there is no mandatory datastore config.
    Config::from_vars(vars(&[])).expect("loads without any MONGODB_* var");
}

#[test]
fn default_impl_matches_env_defaults() {
    let from_env = Config::from_vars(vars(&[])).expect("defaults should deserialize");
    let from_default = Config::default();
    assert_eq!(from_default.port, from_env.port);
    assert_eq!(from_default.bind_addr, from_env.bind_addr);
    assert_eq!(from_default.log_level, from_env.log_level);
    assert_eq!(
        from_default.request_timeout_secs,
        from_env.request_timeout_secs
    );
}

#[test]
fn port_is_overridable() {
    let config = Config::from_vars(vars(&[("FKST_HOSTED_PORT", "9090")])).unwrap();
    assert_eq!(config.port, 9090);
}

#[test]
fn bind_addr_is_overridable() {
    let config = Config::from_vars(vars(&[("FKST_HOSTED_BIND_ADDR", "127.0.0.1")])).unwrap();
    assert_eq!(config.bind_addr, "127.0.0.1");
}

#[test]
fn log_level_is_overridable() {
    let config = Config::from_vars(vars(&[("FKST_HOSTED_LOG_LEVEL", "debug")])).unwrap();
    assert_eq!(config.log_level, "debug");
}

#[test]
fn request_timeout_secs_is_overridable() {
    let config = Config::from_vars(vars(&[("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5")])).unwrap();
    assert_eq!(config.request_timeout_secs, 5);
}

#[test]
fn zero_request_timeout_is_a_config_error() {
    let err = Config::from_vars(vars(&[("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0")]))
        .expect_err("zero timeout must fail");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_HOSTED_REQUEST_TIMEOUT_SECS"));
}

#[test]
fn non_numeric_port_is_a_config_error() {
    let err = Config::from_vars(vars(&[("FKST_HOSTED_PORT", "abc")]))
        .expect_err("non-numeric port must fail");
    assert!(matches!(err, AppError::Config(_)));
}

// ---- github api base (per-user store identity) tests ----------------------

#[test]
fn github_api_base_defaults_and_overrides() {
    let default = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(default.github_api_base_url, "https://api.github.com");
    let overridden = Config::from_vars(vars(&[(
        "FKST_GITHUB_API_BASE_URL",
        "http://127.0.0.1:8080",
    )]))
    .expect("override");
    assert_eq!(overridden.github_api_base_url, "http://127.0.0.1:8080");
}

#[test]
fn blank_github_api_base_is_a_config_error_naming_the_var() {
    let err = Config::from_vars(vars(&[("FKST_GITHUB_API_BASE_URL", "   ")]))
        .expect_err("blank base must fail");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_GITHUB_API_BASE_URL"));
}

// ---- vault configuration tests --------------------------------------------

#[test]
fn vault_caps_default() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.vault_value_byte_cap, 65_536);
    assert_eq!(config.vault_entries_per_scope_cap, 100);
}

#[test]
fn vault_caps_are_overridable() {
    let config = Config::from_vars(vars(&[
        ("FKST_HOSTED_VAULT_VALUE_BYTE_CAP", "1024"),
        ("FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP", "5"),
    ]))
    .expect("overrides");
    assert_eq!(config.vault_value_byte_cap, 1024);
    assert_eq!(config.vault_entries_per_scope_cap, 5);
}

#[test]
fn zero_vault_caps_are_config_errors_naming_the_var() {
    for (var, value) in [
        ("FKST_HOSTED_VAULT_VALUE_BYTE_CAP", "0"),
        ("FKST_HOSTED_VAULT_ENTRIES_PER_SCOPE_CAP", "0"),
    ] {
        let err = Config::from_vars(vars(&[(var, value)])).expect_err("zero cap must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(var), "error must name {var}");
    }
}

// ---- named-environment (FKST_ENV_*) wiring tests ---------------------------

#[test]
fn env_config_defaults_are_wired_into_config() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.env.max_per_user, 20);
    assert_eq!(config.env.validate_max_concurrent, 4);
}

#[test]
fn env_config_zero_bound_surfaces_through_config_from_vars() {
    let err = Config::from_vars(vars(&[("FKST_ENV_MAX_PER_USER", "0")]))
        .expect_err("zero env bound must fail closed through Config");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_ENV_MAX_PER_USER"));
}

#[test]
fn durable_environment_store_must_be_outside_the_application_namespace() {
    let key = "ERERERERERERERERERERERERERERERERERERERERERE=";
    let err = Config::from_vars(vars(&[
        ("FKST_POD_NAMESPACE", "chronoai-fkst"),
        ("FKST_ENV_STORE_NAMESPACE", "chronoai-fkst"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", key),
    ]))
    .expect_err("namespace-local durable store defeats namespace recovery");
    assert!(err.to_string().contains("must differ"));
    assert!(err.to_string().contains("FKST_POD_NAMESPACE"));
    assert!(!err.to_string().contains(key));
}

// ---- Model B reconciler (FKST_*) wiring tests ------------------------------

#[test]
fn reconcile_config_defaults_are_wired_into_config() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(
        config.reconcile.substrate_trigger_label,
        "fkst-substrate-trigger"
    );
    assert_eq!(config.reconcile.reconcile_interval_secs, 30);
    assert_eq!(config.reconcile.github_bot_login, None);
}

#[test]
fn reconcile_config_override_surfaces_through_config_from_vars() {
    let config = Config::from_vars(vars(&[("FKST_RECONCILE_INTERVAL_SECS", "5")]))
        .expect("override should surface");
    assert_eq!(config.reconcile.reconcile_interval_secs, 5);
}

#[test]
fn reconcile_config_bound_violation_surfaces_through_config_from_vars() {
    let err = Config::from_vars(vars(&[("FKST_POD_TOKEN_REFRESH_SECS", "3600")]))
        .expect_err("token refresh at TTL must fail closed through Config");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_POD_TOKEN_REFRESH_SECS"));
}

// ---- static LLM provider configuration tests -------------------------------

#[test]
fn llm_defaults_apply_when_unset() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert_eq!(config.pod.llm_model, "gpt-5.6-sol");
    assert_eq!(config.pod.llm_base_url, "https://llm.aelf.dev/v1");
    // The wire_api defaults to `responses` (codex 0.139+ rejects `chat`).
    assert_eq!(config.pod.llm_wire_api, "responses");
    // The platform-default reasoning effort is the deepest tier (#3393).
    assert_eq!(config.pod.llm_reasoning_effort, "max");
    // No key configured (dispatch off) => empty, never a placeholder.
    assert_eq!(config.llm_api_key.expose_secret(), "");
}

#[test]
fn llm_vars_are_overridable() {
    let config = Config::from_vars(vars(&[
        ("FKST_LLM_MODEL", "gpt-4.1"),
        ("FKST_LLM_BASE_URL", "https://proxy.example/s/llm"),
        ("FKST_LLM_WIRE_API", "chat"),
        ("FKST_LLM_REASONING_EFFORT", " High "),
        ("FKST_LLM_API_KEY", "sk-abc"),
    ]))
    .expect("overrides");
    assert_eq!(config.pod.llm_model, "gpt-4.1");
    assert_eq!(config.pod.llm_base_url, "https://proxy.example/s/llm");
    assert_eq!(config.pod.llm_wire_api, "chat");
    // Trimmed + lowercased on the way in.
    assert_eq!(config.pod.llm_reasoning_effort, "high");
    assert_eq!(config.llm_api_key.expose_secret(), "sk-abc");
}

#[test]
fn blank_llm_vars_are_config_errors_naming_the_var() {
    for var in [
        "FKST_LLM_MODEL",
        "FKST_LLM_BASE_URL",
        "FKST_LLM_WIRE_API",
        "FKST_LLM_REASONING_EFFORT",
    ] {
        let err = Config::from_vars(vars(&[(var, "   ")])).expect_err("blank must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(var), "error must name {var}");
    }
}

#[test]
fn unknown_llm_reasoning_effort_fails_closed_naming_the_tiers() {
    let err = Config::from_vars(vars(&[("FKST_LLM_REASONING_EFFORT", "frobnicate")]))
        .expect_err("an unknown effort must fail closed");
    let msg = err.to_string();
    assert!(msg.contains("FKST_LLM_REASONING_EFFORT"), "{msg}");
    assert!(msg.contains("max"), "names the accepted tiers: {msg}");
    assert!(msg.contains("frobnicate"), "names the bad value: {msg}");
}

// ---- chrono-storage (FKST_STORAGE_* / FKST_NYXID_*) wiring tests ------------

#[test]
fn storage_config_is_none_by_default() {
    // The optional log-streaming feature is disabled unless configured.
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert!(config.storage.is_none());
}

#[test]
fn storage_config_surfaces_through_config_from_vars_when_set() {
    let config = Config::from_vars(vars(&[
        ("FKST_STORAGE_BASE_URL", "https://storage.example/proxy"),
        ("FKST_STORAGE_BUCKET", "fkst-logs"),
        ("FKST_NYXID_TOKEN_URL", "https://nyx.example/oauth/token"),
        ("FKST_NYXID_CLIENT_ID", "sa-client"),
        ("FKST_NYXID_CLIENT_SECRET", "sa-secret"),
    ]))
    .expect("full storage config should load");
    let storage = config.storage.expect("feature enabled");
    assert_eq!(storage.bucket, "fkst-logs");
}

#[test]
fn partial_storage_config_fails_closed_through_config_from_vars() {
    let err = Config::from_vars(vars(&[(
        "FKST_STORAGE_BASE_URL",
        "https://storage.example",
    )]))
    .expect_err("partial storage config must fail closed through Config");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_NYXID_CLIENT_SECRET"));
}

// ---- log-download (FKST_LOG_ADMINS / FKST_PUBLIC_BASE_URL / OAuth) wiring ---

#[test]
fn log_config_defaults_are_wired_into_config() {
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert!(config.log.admins.is_empty());
    assert_eq!(config.log.public_base_url, None);
    assert_eq!(config.log.oauth_base_url, "https://github.com");
}

#[test]
fn log_config_surfaces_through_config_from_vars_when_set() {
    let config = Config::from_vars(vars(&[
        ("FKST_LOG_ADMINS", "ops, 42"),
        ("FKST_PUBLIC_BASE_URL", "https://fkst.example"),
    ]))
    .expect("log config loads");
    assert_eq!(config.log.admins, vec!["ops", "42"]);
    assert_eq!(
        config.log.public_base_url.as_deref(),
        Some("https://fkst.example")
    );
}

#[test]
fn half_configured_oauth_pair_fails_closed_through_config_from_vars() {
    let err = Config::from_vars(vars(&[("FKST_GITHUB_OAUTH_CLIENT_ID", "Iv1.abc")]))
        .expect_err("half OAuth config must fail closed through Config");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_GITHUB_OAUTH_CLIENT_SECRET"));
}

#[test]
fn broader_oauth_is_inert_by_default_and_surfaces_when_configured() {
    // Unset → the whole broader-visibility feature is inert.
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert!(config.log.broader_oauth().is_none());
    // Both vars set → the accessor resolves the classic-OAuth pair.
    let config = Config::from_vars(vars(&[
        ("FKST_GITHUB_BROADER_OAUTH_CLIENT_ID", "classic-id"),
        ("FKST_GITHUB_BROADER_OAUTH_CLIENT_SECRET", "classic-secret"),
    ]))
    .expect("full broader pair loads");
    assert_eq!(
        config.log.broader_oauth().map(|(id, _)| id),
        Some("classic-id")
    );
}

#[test]
fn half_configured_broader_oauth_pair_fails_closed_through_config_from_vars() {
    let err = Config::from_vars(vars(&[(
        "FKST_GITHUB_BROADER_OAUTH_CLIENT_ID",
        "classic-id",
    )]))
    .expect_err("half broader OAuth config must fail closed through Config");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err
        .to_string()
        .contains("FKST_GITHUB_BROADER_OAUTH_CLIENT_SECRET"));
}

#[test]
fn audit_capture_is_off_by_default_and_wired_through_config_from_vars() {
    // Unset → the audit pipeline is inert (no worker, no network) and the
    // documented defaults apply.
    let config = Config::from_vars(vars(&[])).expect("defaults");
    assert!(!config.audit.enabled);
    assert!(config.audit.host.is_none());
    assert_eq!(config.audit.batch_size, 100);
    assert_eq!(config.audit.queue_capacity, 10_000);

    // Set → the resolved block rides on Config, normalized.
    let config = Config::from_vars(vars(&[
        ("FKST_POSTHOG_ENABLED", "true"),
        ("FKST_POSTHOG_HOST", "https://posthog.example/"),
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_token"),
        ("FKST_DEPLOYMENT_ENVIRONMENT", "production"),
    ]))
    .expect("enabled audit config loads");
    assert!(config.audit.enabled);
    assert_eq!(
        config.audit.host.as_deref(),
        Some("https://posthog.example")
    );
    assert_eq!(config.audit.environment, "production");
}

#[test]
fn an_invalid_audit_block_fails_closed_through_config_from_vars() {
    // Enabled without a host: the whole process must refuse to start rather
    // than boot with a silently disabled audit trail.
    let err = Config::from_vars(vars(&[("FKST_POSTHOG_ENABLED", "true")]))
        .expect_err("enabled audit without a host must fail closed");
    assert!(matches!(err, AppError::Config(_)));
    assert!(err.to_string().contains("FKST_POSTHOG_HOST"));

    // A nonsensical bound fails even while the feature is off, so a typo
    // surfaces at deploy time instead of at the flip.
    let err = Config::from_vars(vars(&[("FKST_POSTHOG_QUEUE_CAPACITY", "0")]))
        .expect_err("a zero queue must fail closed");
    assert!(err.to_string().contains("FKST_POSTHOG_QUEUE_CAPACITY"));
}

#[test]
fn direct_capture_and_relay_delivery_are_mutually_exclusive() {
    // Both on: two writers into one project, and a control plane that must hold
    // the capture token it is not supposed to have. Refused, naming both knobs.
    for mode in ["best_effort", "required"] {
        let err = Config::from_vars(vars(&[
            ("FKST_POSTHOG_ENABLED", "true"),
            ("FKST_POSTHOG_HOST", "https://posthog.example"),
            ("FKST_POSTHOG_PROJECT_TOKEN", "phc_token"),
            ("FKST_AUDIT_DELIVERY_MODE", mode),
            (
                "FKST_AUDIT_RELAY_URL",
                "http://fkst-audit-relay.chronoai-fkst.svc.cluster.local",
            ),
            ("FKST_AUDIT_RELAY_WRITE_TOKEN", "relay-write"),
            ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "420"),
        ]))
        .expect_err("two capture writers must fail closed");
        let message = err.to_string();
        assert!(
            message.contains("FKST_POSTHOG_ENABLED"),
            "{mode}: {message}"
        );
        assert!(
            message.contains("FKST_AUDIT_DELIVERY_MODE"),
            "{mode}: {message}"
        );
    }

    // Either one alone is the supported shape. The relay captures...
    let relayed = Config::from_vars(vars(&[
        ("FKST_AUDIT_DELIVERY_MODE", "required"),
        (
            "FKST_AUDIT_RELAY_URL",
            "http://fkst-audit-relay.chronoai-fkst.svc.cluster.local",
        ),
        ("FKST_AUDIT_RELAY_WRITE_TOKEN", "relay-write"),
        ("FKST_AUDIT_INCOMPLETE_GRACE_SECS", "420"),
    ]))
    .expect("relay-only delivery loads");
    assert!(!relayed.audit.enabled);

    // ...or the control plane does, with no relay in the picture.
    let direct = Config::from_vars(vars(&[
        ("FKST_POSTHOG_ENABLED", "true"),
        ("FKST_POSTHOG_HOST", "https://posthog.example"),
        ("FKST_POSTHOG_PROJECT_TOKEN", "phc_token"),
    ]))
    .expect("direct-capture delivery loads");
    assert!(direct.audit.enabled);
    assert!(!direct.audit_delivery.mode.uses_relay());
}
