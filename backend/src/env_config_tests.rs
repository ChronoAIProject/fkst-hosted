use super::*;

fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
    pairs
        .iter()
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect()
}

#[test]
fn defaults_apply_when_nothing_is_set() {
    let config = EnvConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
    assert!(config.store_namespace.is_none());
    assert!(config.store_legacy_namespace.is_none());
    assert!(config.store_encryption_key.is_none());
    assert_eq!(config.max_per_user, 20);
    assert_eq!(config.install_max_commands, 50);
    assert_eq!(config.install_max_command_bytes, 4096);
    assert_eq!(config.install_stderr_tail_bytes, 4096);
    assert_eq!(config.validate_deadline_secs, 300);
    assert_eq!(config.validate_max_concurrent, 4);
    assert_eq!(config.validate_poll_interval_secs, 2);
}

#[test]
fn default_impl_matches_env_defaults() {
    let from_env = EnvConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
    let from_default = EnvConfig::default();
    assert_eq!(from_default.store_namespace, from_env.store_namespace);
    assert_eq!(
        from_default.store_legacy_namespace,
        from_env.store_legacy_namespace
    );
    assert!(from_default.store_encryption_key.is_none());
    assert_eq!(from_default.max_per_user, from_env.max_per_user);
    assert_eq!(
        from_default.install_max_commands,
        from_env.install_max_commands
    );
    assert_eq!(
        from_default.install_max_command_bytes,
        from_env.install_max_command_bytes
    );
    assert_eq!(
        from_default.install_stderr_tail_bytes,
        from_env.install_stderr_tail_bytes
    );
    assert_eq!(
        from_default.validate_deadline_secs,
        from_env.validate_deadline_secs
    );
    assert_eq!(
        from_default.validate_max_concurrent,
        from_env.validate_max_concurrent
    );
    assert_eq!(
        from_default.validate_poll_interval_secs,
        from_env.validate_poll_interval_secs
    );
}

#[test]
fn every_knob_is_overridable() {
    let config = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_MAX_PER_USER", "5"),
        ("FKST_ENV_INSTALL_MAX_COMMANDS", "10"),
        ("FKST_ENV_INSTALL_MAX_COMMAND_BYTES", "256"),
        ("FKST_ENV_INSTALL_STDERR_TAIL_BYTES", "512"),
        ("FKST_ENV_VALIDATE_DEADLINE_SECS", "600"),
        ("FKST_ENV_VALIDATE_MAX_CONCURRENT", "8"),
        ("FKST_ENV_VALIDATE_POLL_INTERVAL_SECS", "3"),
    ]))
    .expect("overrides should deserialize");
    assert_eq!(config.max_per_user, 5);
    assert_eq!(config.install_max_commands, 10);
    assert_eq!(config.install_max_command_bytes, 256);
    assert_eq!(config.install_stderr_tail_bytes, 512);
    assert_eq!(config.validate_deadline_secs, 600);
    assert_eq!(config.validate_max_concurrent, 8);
    assert_eq!(config.validate_poll_interval_secs, 3);
}

#[test]
fn zero_bounds_are_config_errors_naming_the_var() {
    for var in [
        "FKST_ENV_MAX_PER_USER",
        "FKST_ENV_INSTALL_MAX_COMMANDS",
        "FKST_ENV_INSTALL_MAX_COMMAND_BYTES",
        "FKST_ENV_INSTALL_STDERR_TAIL_BYTES",
        "FKST_ENV_VALIDATE_DEADLINE_SECS",
        "FKST_ENV_VALIDATE_MAX_CONCURRENT",
        "FKST_ENV_VALIDATE_POLL_INTERVAL_SECS",
    ] {
        let err = EnvConfig::from_vars(&vars(&[(var, "0")])).expect_err("zero must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(var), "error must name {var}");
    }
}

const KEY_B64: &str = "ERERERERERERERERERERERERERERERERERERERERERE=";

#[test]
fn durable_store_accepts_one_inline_key_source() {
    let config = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_LEGACY_NAMESPACE", "chronoai-fkst"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", KEY_B64),
    ]))
    .expect("valid durable store config");
    assert_eq!(config.store_namespace.as_deref(), Some("durable-envs"));
    assert_eq!(
        config.store_legacy_namespace.as_deref(),
        Some("chronoai-fkst")
    );
    assert!(config.store_encryption_key.is_some());
}

#[test]
fn durable_store_accepts_a_trimmed_key_file() {
    use secrecy::ExposeSecret;

    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("environment-store-key");
    std::fs::write(&path, format!("\n{KEY_B64}\n")).expect("write key file");
    let path = path.to_string_lossy().into_owned();
    let config = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY_FILE", &path),
    ]))
    .expect("valid file-backed key");
    assert_eq!(
        config
            .store_encryption_key
            .as_ref()
            .expect("key")
            .expose_secret(),
        KEY_B64
    );
}

#[test]
fn durable_store_rejects_conflicting_key_sources() {
    let err = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", KEY_B64),
        ("FKST_ENV_STORE_ENCRYPTION_KEY_FILE", "/tmp/not-read"),
    ]))
    .expect_err("two key sources must fail");
    assert!(err.to_string().contains("exactly one"));
}

#[test]
fn durable_store_rejects_missing_or_blank_keys() {
    for pairs in [
        vec![("FKST_ENV_STORE_NAMESPACE", "durable-envs")],
        vec![
            ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
            ("FKST_ENV_STORE_ENCRYPTION_KEY", "   "),
        ],
        vec![
            ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
            ("FKST_ENV_STORE_ENCRYPTION_KEY_FILE", "   "),
        ],
    ] {
        let err = EnvConfig::from_vars(&vars(&pairs)).expect_err("key must fail closed");
        assert!(err.to_string().contains("FKST_ENV_STORE_ENCRYPTION_KEY"));
    }
}

#[test]
fn durable_store_rejects_an_empty_key_file() {
    let dir = tempfile::tempdir().expect("tempdir");
    let path = dir.path().join("environment-store-key");
    std::fs::write(&path, " \n").expect("write empty key file");
    let path = path.to_string_lossy().into_owned();
    let err = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY_FILE", &path),
    ]))
    .expect_err("empty file must fail");
    assert!(err.to_string().contains("FILE is empty"));
}

#[test]
fn durable_store_rejects_invalid_base64_and_wrong_length() {
    for key in ["not base64", "c2hvcnQ="] {
        let err = EnvConfig::from_vars(&vars(&[
            ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
            ("FKST_ENV_STORE_ENCRYPTION_KEY", key),
        ]))
        .expect_err("invalid key must fail");
        assert!(err.to_string().contains("FKST_ENV_STORE_ENCRYPTION_KEY"));
        assert!(!err.to_string().contains(key));
    }
}

#[test]
fn durable_store_rejects_key_or_migration_without_namespace() {
    for pairs in [
        vec![("FKST_ENV_STORE_ENCRYPTION_KEY", KEY_B64)],
        vec![("FKST_ENV_STORE_LEGACY_NAMESPACE", "chronoai-fkst")],
    ] {
        let err = EnvConfig::from_vars(&vars(&pairs))
            .expect_err("dependent config without durable namespace must fail");
        assert!(err.to_string().contains("FKST_ENV_STORE_NAMESPACE"));
    }
}

#[test]
fn durable_and_legacy_namespaces_must_differ() {
    let err = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_LEGACY_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", KEY_B64),
    ]))
    .expect_err("same namespace would let migration delete durable records");
    assert!(err.to_string().contains("must differ"));
}

#[test]
fn key_material_is_redacted_from_debug_and_errors() {
    let config = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", KEY_B64),
    ]))
    .expect("valid config");
    let debug = format!("{config:?}");
    assert!(!debug.contains(KEY_B64));
    assert!(debug.contains("REDACTED"));

    let bad_key = "this-sensitive-invalid-key-must-not-echo";
    let error = EnvConfig::from_vars(&vars(&[
        ("FKST_ENV_STORE_NAMESPACE", "durable-envs"),
        ("FKST_ENV_STORE_ENCRYPTION_KEY", bad_key),
    ]))
    .expect_err("invalid key");
    assert!(!error.to_string().contains(bad_key));
}
