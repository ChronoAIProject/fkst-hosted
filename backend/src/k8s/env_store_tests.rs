//! Tests for [`super`] (the named-environment metadata + projection helpers).
//! Split into a sibling file to keep `env_store_meta.rs` under the 500-line
//! limit; included via `#[cfg(test)] #[path = "env_store_tests.rs"] mod tests;`.

use super::*;
// `ByteString` is a Secret-construction detail used only by these tests (the
// meta module itself never names the type), so import it directly here.
use k8s_openapi::ByteString;

#[test]
fn env_object_name_composes_id_and_name() {
    assert_eq!(env_object_name(42, "prod"), "fkst-env-42-prod");
    assert_eq!(env_object_name(583231, "ci"), "fkst-env-583231-ci");
}

#[test]
fn env_object_name_stays_within_the_dns1123_label_budget() {
    // A realistic large GitHub id (well beyond current account counts) with a
    // 40-char env name must still fit the 63-char DNS-1123 label limit. The route
    // layer enforces the PRECISE per-part budget; this guards the composed shape.
    let big_id: i64 = 999_999_999; // 9 digits
    let name = "a".repeat(40);
    let object = env_object_name(big_id, &name);
    assert!(
        object.len() <= 63,
        "object name {object:?} is {} chars, over the 63-char limit",
        object.len()
    );
}

#[test]
fn env_labels_carry_component_id_and_login() {
    let labels = env_labels(42, "octocat");
    assert_eq!(labels[PART_OF_LABEL], "fkst-hosted");
    assert_eq!(labels[COMPONENT_LABEL], "user-env");
    assert_eq!(labels[USER_ID_LABEL], "42");
    assert_eq!(labels[LOGIN_LABEL], "octocat");
}

#[test]
fn owner_selector_filters_by_component_and_id() {
    assert_eq!(
        owner_selector(7),
        "app.kubernetes.io/component=user-env,fkst.chrono-ai.fun/github-user-id=7"
    );
}

#[test]
fn env_annotations_carry_status_and_provenance() {
    let ann = env_annotations("Prod-Env", "2026-06-30T00:00:00Z", "abc123", "img:1");
    // The RAW (un-sanitized) name is preserved in the annotation.
    assert_eq!(ann[ENV_NAME_ANNOTATION], "Prod-Env");
    assert_eq!(ann[STATUS_ANNOTATION], "ready");
    assert_eq!(ann[VALIDATED_AT_ANNOTATION], "2026-06-30T00:00:00Z");
    assert_eq!(ann[CONTENT_HASH_ANNOTATION], "abc123");
    assert_eq!(ann[VALIDATION_IMAGE_ANNOTATION], "img:1");
}

#[test]
fn sanitize_label_value_coerces_to_a_valid_label() {
    assert_eq!(sanitize_label_value("octo-cat_1.2"), "octo-cat_1.2");
    assert_eq!(sanitize_label_value("a/b c"), "a-b-c");
    assert_eq!(sanitize_label_value("-weird-"), "weird");
    assert_eq!(sanitize_label_value(&"a".repeat(100)).len(), 63);
}

// ---- content_hash ---------------------------------------------------------

fn vars(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
    pairs
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

#[test]
fn content_hash_is_stable_and_hex() {
    let install = vec!["apt-get install jq".to_string()];
    let variables = vars(&[("FOO", "bar")]);
    let keys = vec!["TOKEN".to_string()];
    let a = content_hash(&install, &variables, &keys);
    let b = content_hash(&install, &variables, &keys);
    assert_eq!(a, b, "same input hashes identically");
    assert_eq!(a.len(), 64, "sha256 hex is 64 chars");
    assert!(a.chars().all(|c| c.is_ascii_hexdigit()));
}

#[test]
fn content_hash_changes_when_any_public_input_changes() {
    let base = content_hash(
        &["a".to_string()],
        &vars(&[("FOO", "bar")]),
        &["TOKEN".to_string()],
    );
    // install differs
    assert_ne!(
        base,
        content_hash(
            &["b".to_string()],
            &vars(&[("FOO", "bar")]),
            &["TOKEN".to_string()]
        )
    );
    // variable value differs
    assert_ne!(
        base,
        content_hash(
            &["a".to_string()],
            &vars(&[("FOO", "baz")]),
            &["TOKEN".to_string()]
        )
    );
    // secret key name differs
    assert_ne!(
        base,
        content_hash(
            &["a".to_string()],
            &vars(&[("FOO", "bar")]),
            &["OTHER".to_string()]
        )
    );
}

#[test]
fn content_hash_is_independent_of_secret_key_order() {
    let install = vec!["a".to_string()];
    let variables = vars(&[("FOO", "bar")]);
    let ordered = content_hash(&install, &variables, &["A".to_string(), "B".to_string()]);
    let reversed = content_hash(&install, &variables, &["B".to_string(), "A".to_string()]);
    assert_eq!(
        ordered, reversed,
        "secret key order must not affect the hash"
    );
}

#[test]
fn content_hash_is_independent_of_variable_insertion_order() {
    // BTreeMap always sorts, so two different insertion orders hash identically.
    let mut first = BTreeMap::new();
    first.insert("B".to_string(), "2".to_string());
    first.insert("A".to_string(), "1".to_string());
    let mut second = BTreeMap::new();
    second.insert("A".to_string(), "1".to_string());
    second.insert("B".to_string(), "2".to_string());
    let install = vec!["x".to_string()];
    assert_eq!(
        content_hash(&install, &first, &[]),
        content_hash(&install, &second, &[])
    );
}

// ---- projection helpers ---------------------------------------------------

#[test]
fn secret_key_names_returns_sorted_names_never_values() {
    let mut data = BTreeMap::new();
    data.insert("ZED".to_string(), ByteString(b"super-secret".to_vec()));
    data.insert(
        "API_KEY".to_string(),
        ByteString(b"another-secret".to_vec()),
    );
    let secret = Secret {
        data: Some(data),
        ..Default::default()
    };
    let names = secret_key_names(&secret);
    assert_eq!(names, vec!["API_KEY".to_string(), "ZED".to_string()]);
    assert!(!names.iter().any(|n| n.contains("secret")));
}

#[test]
fn decode_secret_values_decodes_data_and_drops_non_utf8() {
    let mut data = BTreeMap::new();
    data.insert("GOOD".to_string(), ByteString(b"ok".to_vec()));
    data.insert("BAD".to_string(), ByteString(vec![0xff, 0xfe]));
    let secret = Secret {
        data: Some(data),
        ..Default::default()
    };
    let values = decode_secret_values(&secret);
    assert_eq!(values["GOOD"], "ok");
    assert!(!values.contains_key("BAD"), "invalid utf-8 is dropped");
}

#[test]
fn parse_install_and_variables_round_trip_reserved_keys() {
    let install = vec!["one".to_string(), "two".to_string()];
    let variables = vars(&[("FOO", "bar"), ("BAZ", "qux")]);
    let mut data = BTreeMap::new();
    data.insert(
        INSTALL_KEY.to_string(),
        serde_json::to_string(&install).unwrap(),
    );
    data.insert(
        VARIABLES_KEY.to_string(),
        serde_json::to_string(&variables).unwrap(),
    );
    assert_eq!(parse_install(&data), install);
    assert_eq!(parse_variables(&data), variables);
}

#[test]
fn parse_install_and_variables_fail_soft_on_missing_or_malformed() {
    let empty = BTreeMap::new();
    assert!(parse_install(&empty).is_empty());
    assert!(parse_variables(&empty).is_empty());

    let mut bad = BTreeMap::new();
    bad.insert(INSTALL_KEY.to_string(), "not json".to_string());
    bad.insert(VARIABLES_KEY.to_string(), "{".to_string());
    assert!(parse_install(&bad).is_empty());
    assert!(parse_variables(&bad).is_empty());
}

#[test]
fn annotation_reads_present_and_absent_keys() {
    let meta = ObjectMeta {
        annotations: Some(BTreeMap::from([(
            STATUS_ANNOTATION.to_string(),
            "ready".to_string(),
        )])),
        ..Default::default()
    };
    assert_eq!(annotation(&meta, STATUS_ANNOTATION), "ready");
    assert_eq!(annotation(&meta, CONTENT_HASH_ANNOTATION), "");
    assert_eq!(annotation(&ObjectMeta::default(), STATUS_ANNOTATION), "");
}

// ---- record secret-hiding shape lock --------------------------------------

#[test]
fn env_record_serializes_key_names_never_values() {
    // The record exposes secret KEY NAMES only; there is no field that could ever
    // hold a secret value. Lock the serialized shape to guard that invariant.
    let record = EnvRecord {
        name: "prod".to_string(),
        status: "ready".to_string(),
        validated_at: "2026-06-30T00:00:00Z".to_string(),
        install: vec!["apt-get install jq".to_string()],
        variables: vars(&[("FOO", "bar")]),
        secret_keys: vec!["TOKEN".to_string()],
    };
    let json = serde_json::to_value(&record).expect("serializes");
    let obj = json.as_object().expect("object");
    // Exactly the six declared fields — no `secrets`/value leak field.
    assert_eq!(obj.len(), 6);
    assert!(obj.get("secrets").is_none());
    assert_eq!(json["secret_keys"], serde_json::json!(["TOKEN"]));
    assert_eq!(json["variables"]["FOO"], "bar");
}

#[test]
fn env_summary_serializes_counts_only() {
    let summary = EnvSummary {
        name: "prod".to_string(),
        status: "ready".to_string(),
        validated_at: "2026-06-30T00:00:00Z".to_string(),
        install_command_count: 2,
        variable_count: 3,
        secret_count: 1,
    };
    let json = serde_json::to_value(&summary).expect("serializes");
    let obj = json.as_object().expect("object");
    assert_eq!(obj.len(), 6);
    assert_eq!(json["secret_count"], 1);
    // Only a COUNT is exposed — no key names, no values.
    assert!(obj.get("secret_keys").is_none());
    assert!(obj.get("secrets").is_none());
}
