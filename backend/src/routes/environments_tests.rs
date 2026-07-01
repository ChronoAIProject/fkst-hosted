//! Tests for [`super`] (the named-environment REST layer). Split into a sibling
//! file to keep `environments.rs` under the 500-line limit; included via
//! `#[cfg(test)] #[path = "environments_tests.rs"] mod tests;`.

use super::*;

fn config() -> Config {
    Config::default()
}

// ---- env key validation ---------------------------------------------------

#[test]
fn valid_env_key_accepts_env_var_names() {
    for key in ["FOO", "_x", "API_KEY", "a1", "MY_VAR_2", "_"] {
        assert!(valid_env_key(key), "must accept {key:?}");
    }
}

#[test]
fn valid_env_key_rejects_non_env_var_names() {
    for key in ["", "1FOO", "a-b", "a.b", "a b", "a/b", "MY-VAR", "key=val"] {
        assert!(!valid_env_key(key), "must reject {key:?}");
    }
}

#[test]
fn validate_key_maps_invalid_to_422() {
    assert!(validate_key("GOOD_KEY").is_ok());
    let err = validate_key("bad-key").expect_err("must reject");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_key_rejects_reserved_keys() {
    // The whole `FKST_*` family, git-credential vars, allow-listed host vars, and
    // the engine's LLM credential slot are all reserved.
    for key in [
        "FKST_ANYTHING",
        "GITHUB_TOKEN",
        "GH_TOKEN",
        "GIT_CONFIG_KEY_0",
        "PATH",
        "HOME",
        "LLM_API_KEY",
    ] {
        let err = validate_key(key).expect_err("reserved key must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "for {key:?}");
    }
}

// ---- entries validation ---------------------------------------------------

#[test]
fn validate_entries_accepts_within_caps() {
    let mut m = BTreeMap::new();
    m.insert("FOO".to_string(), "bar".to_string());
    assert!(validate_entries(&m, &config()).is_ok());
}

#[test]
fn validate_entries_rejects_a_bad_key() {
    let mut m = BTreeMap::new();
    m.insert("not ok".to_string(), "bar".to_string());
    let err = validate_entries(&m, &config()).expect_err("bad key must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_entries_rejects_a_reserved_key() {
    let mut m = BTreeMap::new();
    m.insert("LLM_API_KEY".to_string(), "x".to_string());
    let err = validate_entries(&m, &config()).expect_err("reserved key must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_entries_rejects_oversize_value() {
    let mut cfg = config();
    cfg.vault_value_byte_cap = 4;
    let mut m = BTreeMap::new();
    m.insert("FOO".to_string(), "toolong".to_string());
    let err = validate_entries(&m, &cfg).expect_err("oversize value must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_entries_rejects_too_many_entries() {
    let mut cfg = config();
    cfg.vault_entries_per_scope_cap = 1;
    let mut m = BTreeMap::new();
    m.insert("A".to_string(), "1".to_string());
    m.insert("B".to_string(), "2".to_string());
    let err = validate_entries(&m, &cfg).expect_err("over cap must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

// ---- name validation ------------------------------------------------------

#[test]
fn validate_name_accepts_valid_dns_names() {
    for name in ["prod", "ci-2", "a", "web-app-1", &"n".repeat(40)] {
        assert!(validate_name(name, 42).is_ok(), "must accept {name:?}");
    }
}

#[test]
fn validate_name_rejects_bad_shapes() {
    for name in ["", "Prod", "-x", "x-", "a_b", "a.b", "UP", "x y"] {
        let err = validate_name(name, 42).expect_err("bad name must fail");
        assert!(matches!(err, AppError::Unprocessable(_)), "for {name:?}");
    }
}

#[test]
fn validate_name_rejects_over_length() {
    let too_long = "a".repeat(41);
    let err = validate_name(&too_long, 42).expect_err("over 40 chars must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_name_rejects_object_name_over_dns_budget() {
    // A 40-char name is legal on its own, but with a large id the composed
    // `fkst-env-<id>-<name>` exceeds the 63-char DNS-1123 label limit.
    let big_id: i64 = 9_999_999_999_999_999; // 16 digits
    let name = "a".repeat(40);
    let err = validate_name(&name, big_id).expect_err("object name over budget must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
    // The same 40-char name fits a small id.
    assert!(validate_name(&name, 1).is_ok());
}

// ---- install validation ---------------------------------------------------

#[test]
fn validate_install_accepts_a_reasonable_list() {
    let install = vec!["npm ci".to_string(), "npm run build".to_string()];
    assert!(validate_install(&install, &config()).is_ok());
}

#[test]
fn validate_install_rejects_an_empty_list() {
    let err = validate_install(&[], &config()).expect_err("empty list must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_install_rejects_too_many_commands() {
    let mut cfg = config();
    cfg.env.install_max_commands = 2;
    let install = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    let err = validate_install(&install, &cfg).expect_err("over cap must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_install_rejects_a_blank_command() {
    let install = vec!["ok".to_string(), "   ".to_string()];
    let err = validate_install(&install, &config()).expect_err("blank command must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

#[test]
fn validate_install_rejects_an_oversize_command() {
    let mut cfg = config();
    cfg.env.install_max_command_bytes = 4;
    let install = vec!["toolong".to_string()];
    let err = validate_install(&install, &cfg).expect_err("oversize command must fail");
    assert!(matches!(err, AppError::Unprocessable(_)));
}

// ---- DTO deserialization + projection -------------------------------------

#[test]
fn environment_spec_deserializes_all_fields() {
    let spec: EnvironmentSpec = serde_json::from_value(serde_json::json!({
        "install": ["npm ci"],
        "variables": { "NODE_ENV": "production" },
        "secrets": { "TOKEN": "s3cr3t" }
    }))
    .expect("deserializes");
    assert_eq!(spec.install, vec!["npm ci".to_string()]);
    assert_eq!(spec.variables["NODE_ENV"], "production");
    assert_eq!(spec.secrets["TOKEN"], "s3cr3t");
}

#[test]
fn environment_spec_fields_are_optional() {
    let empty: EnvironmentSpec =
        serde_json::from_value(serde_json::json!({})).expect("deserializes");
    assert!(empty.install.is_empty() && empty.variables.is_empty() && empty.secrets.is_empty());
}

#[test]
fn summary_from_record_maps_counts() {
    let summary = EnvSummary {
        name: "prod".to_string(),
        status: "ready".to_string(),
        validated_at: "2026-01-01T00:00:00+00:00".to_string(),
        install_command_count: 3,
        variable_count: 2,
        secret_count: 1,
    };
    let out = summary_from_record(summary);
    assert_eq!(out.name, "prod");
    assert_eq!(out.install_command_count, 3);
    assert_eq!(out.variable_count, 2);
    assert_eq!(out.secret_count, 1);
}

// ---- secret-hiding shape (the value never crosses the API boundary) --------

#[test]
fn environment_view_never_carries_secret_values() {
    // The view is install + variables + secret KEY NAMES; there is no field that
    // could ever hold a secret value. Assert the serialized shape to lock it in.
    let mut variables = BTreeMap::new();
    variables.insert("FOO".to_string(), "bar".to_string());
    let view = EnvironmentView {
        name: "prod".to_string(),
        status: "ready".to_string(),
        validated_at: "2026-01-01T00:00:00+00:00".to_string(),
        install: vec!["echo hi".to_string()],
        variables,
        secret_keys: vec!["TOKEN".to_string()],
    };
    let json = serde_json::to_value(&view).expect("serializes");
    assert_eq!(json["variables"]["FOO"], "bar");
    assert_eq!(json["secret_keys"], serde_json::json!(["TOKEN"]));
    // EXACTLY these six top-level fields — no `secrets`/value leak.
    let obj = json.as_object().expect("object");
    let mut keys: Vec<&str> = obj.keys().map(String::as_str).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "install",
            "name",
            "secret_keys",
            "status",
            "validated_at",
            "variables"
        ]
    );
    assert!(obj.get("secrets").is_none(), "no `secrets` field may exist");
}
