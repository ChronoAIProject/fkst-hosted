use fkst_qa_contracts::{
    contract_registry, validate_local_evidence_object, validate_local_evidence_object_ref,
    validate_local_sanitized_observation_ref, ContractError, ValidatedValue,
};
use serde_json::{json, Value};

type Validator = fn(&[u8]) -> Result<ValidatedValue, ContractError>;

const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";
const PATTERNED_DIGEST: &str = "abcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcdefabcd";
const DUPLICATE_EVIDENCE_OBJECT: &[u8] = br#"{"schema_version":"qa.local-evidence/v1","run_id":"run_1","attempt":1,"object_id":"evidence/1","object_id":"evidence/2","role":"browser-screenshot","media_type":"image/png","byte_length":0,"sha256":"0000000000000000000000000000000000000000000000000000000000000000","ownership":"local-only:not-uploadable"}"#;
const DUPLICATE_OBSERVATION_REF: &[u8] = br#"{"kind":"local-sanitized-observation","id":"observation/1","id":"observation/2","schema_version":"qa.local-evidence/v1","content_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#;
const DUPLICATE_EVIDENCE_REF: &[u8] = br#"{"kind":"local-evidence-object","id":"evidence/1","id":"evidence/2","schema_version":"qa.local-evidence/v1","content_digest":"sha256:0000000000000000000000000000000000000000000000000000000000000000"}"#;

fn screenshot_object() -> Value {
    json!({
        "schema_version": "qa.local-evidence/v1",
        "run_id": "run_1",
        "attempt": 1,
        "object_id": "evidence/1",
        "role": "browser-screenshot",
        "media_type": "image/png",
        "byte_length": 0,
        "sha256": ZERO_DIGEST,
        "ownership": "local-only:not-uploadable"
    })
}

fn runner_log_object() -> Value {
    json!({
        "schema_version": "qa.local-evidence/v1",
        "run_id": "run_1",
        "attempt": 1,
        "object_id": "evidence/2",
        "role": "runner-log",
        "media_type": "text/plain; charset=utf-8",
        "byte_length": 1_048_576,
        "sha256": PATTERNED_DIGEST,
        "ownership": "local-only:not-uploadable"
    })
}

fn observation_ref() -> Value {
    json!({
        "kind": "local-sanitized-observation",
        "id": "observation/1",
        "schema_version": "qa.local-evidence/v1",
        "content_digest": format!("sha256:{ZERO_DIGEST}")
    })
}

fn evidence_ref() -> Value {
    json!({
        "kind": "local-evidence-object",
        "id": "evidence/1",
        "schema_version": "qa.local-evidence/v1",
        "content_digest": format!("sha256:{ZERO_DIGEST}")
    })
}

#[test]
fn local_evidence_registry_exposes_exact_public_definitions() {
    let registry = contract_registry().expect("load contract registry");
    for type_name in [
        "LocalSanitizedObservation",
        "LocalEvidenceObject",
        "LocalSanitizedObservationRef",
        "LocalEvidenceObjectRef",
    ] {
        assert_eq!(
            registry.pointer(&format!("/types/{type_name}")),
            Some(&json!({
                "schema": "qa.local-evidence/v1",
                "pointer": format!("#/$defs/{type_name}")
            }))
        );
    }
    for forbidden_alias in [
        "SanitizedObservation",
        "ArtifactPointer",
        "qa.sanitized-observation~1v1",
        "qa.artifact-pointer~1v1",
    ] {
        assert!(registry
            .pointer(&format!("/types/{forbidden_alias}"))
            .is_none());
    }
}

#[test]
fn exact_local_evidence_fixtures_validate_and_values_remain_owned() {
    let versioned_observation_ref = with(
        &observation_ref(),
        &["id", "content_digest", "version"],
        &[
            json!("observation/2"),
            json!(format!("sha256:{PATTERNED_DIGEST}")),
            json!("v1"),
        ],
    );
    let versioned_evidence_ref = with(
        &evidence_ref(),
        &["id", "content_digest", "version"],
        &[
            json!("evidence/2"),
            json!(format!("sha256:{PATTERNED_DIGEST}")),
            json!("v1"),
        ],
    );
    for (fixture, validate) in [
        (
            screenshot_object(),
            validate_local_evidence_object as Validator,
        ),
        (
            runner_log_object(),
            validate_local_evidence_object as Validator,
        ),
        (
            observation_ref(),
            validate_local_sanitized_observation_ref as Validator,
        ),
        (
            versioned_observation_ref,
            validate_local_sanitized_observation_ref as Validator,
        ),
        (
            evidence_ref(),
            validate_local_evidence_object_ref as Validator,
        ),
        (
            versioned_evidence_ref,
            validate_local_evidence_object_ref as Validator,
        ),
    ] {
        let validated = validate(&raw(&fixture)).expect("validate exact fixture");
        assert_eq!(validated.value(), &fixture);
        let mut detached = validated.value().clone();
        detached["schema_version"] = json!("changed");
        assert_eq!(validated.value()["schema_version"], "qa.local-evidence/v1");
    }
}

#[test]
fn new_validators_reject_duplicate_keys_unknown_fields_and_missing_members() {
    validate_local_evidence_object(DUPLICATE_EVIDENCE_OBJECT)
        .expect_err("reject duplicate object_id");
    validate_local_sanitized_observation_ref(DUPLICATE_OBSERVATION_REF)
        .expect_err("reject duplicate observation id");
    validate_local_evidence_object_ref(DUPLICATE_EVIDENCE_REF)
        .expect_err("reject duplicate evidence id");

    for (fixture, validate) in [
        (
            screenshot_object(),
            validate_local_evidence_object as Validator,
        ),
        (
            observation_ref(),
            validate_local_sanitized_observation_ref as Validator,
        ),
        (
            evidence_ref(),
            validate_local_evidence_object_ref as Validator,
        ),
    ] {
        validate(&raw(&set(&fixture, "uploadable", json!(true))))
            .expect_err("reject unknown field");
        validate(&raw(&remove(&fixture, "schema_version")))
            .expect_err("reject missing schema version");
        validate(&raw(&set(
            &fixture,
            "schema_version",
            json!("qa.local-evidence/v2"),
        )))
        .expect_err("reject wrong schema version");
    }
    for field in [
        "run_id",
        "attempt",
        "object_id",
        "role",
        "media_type",
        "byte_length",
        "sha256",
        "ownership",
    ] {
        validate_local_evidence_object(&raw(&remove(&screenshot_object(), field)))
            .expect_err("reject missing evidence member");
    }
    for (fixture, validate) in [
        (
            observation_ref(),
            validate_local_sanitized_observation_ref as Validator,
        ),
        (
            evidence_ref(),
            validate_local_evidence_object_ref as Validator,
        ),
    ] {
        for field in ["kind", "id", "content_digest"] {
            validate(&raw(&remove(&fixture, field))).expect_err("reject missing ref member");
        }
    }
}

#[test]
fn local_evidence_object_enforces_boundaries_and_literals() {
    for (field, value) in [
        ("run_id", json!("A")),
        ("run_id", json!("A".repeat(64))),
        ("attempt", json!(1)),
        ("attempt", json!(9_007_199_254_740_991_u64)),
        ("object_id", json!("evidence/0")),
        ("object_id", json!(format!("evidence/{}", "1".repeat(55)))),
        ("byte_length", json!(0)),
        ("byte_length", json!(1_048_576)),
    ] {
        validate_local_evidence_object(&raw(&set(&screenshot_object(), field, value)))
            .expect("accept boundary");
    }
    for (field, value) in [
        ("run_id", json!("")),
        ("run_id", json!("-run")),
        ("run_id", json!("A".repeat(65))),
        ("attempt", json!(0)),
        ("attempt", json!(9_007_199_254_740_992_u64)),
        ("attempt", json!(1.5)),
        ("object_id", json!("evidence/")),
        ("object_id", json!("evidence/a")),
        ("object_id", json!("observation/1")),
        ("object_id", json!(format!("evidence/{}", "1".repeat(56)))),
        ("byte_length", json!(-1)),
        ("byte_length", json!(1_048_577)),
        ("byte_length", json!(1.5)),
        ("sha256", json!("A".repeat(64))),
        ("sha256", json!("g".repeat(64))),
        ("sha256", json!("0".repeat(63))),
        ("sha256", json!("0".repeat(65))),
        ("sha256", json!(format!("sha256:{ZERO_DIGEST}"))),
        ("ownership", json!("uploadable")),
        ("role", json!("browser-video")),
        ("media_type", json!("application/octet-stream")),
    ] {
        validate_local_evidence_object(&raw(&set(&screenshot_object(), field, value)))
            .expect_err("reject invalid boundary or literal");
    }
}

#[test]
fn local_evidence_object_schema_and_public_validator_own_pairing() {
    for fixture in [screenshot_object(), runner_log_object()] {
        validate_local_evidence_object(&raw(&fixture)).expect("accept valid pair");
        assert!(direct_evidence_schema_valid(&fixture));
    }
    for fixture in [
        set(
            &screenshot_object(),
            "media_type",
            json!("text/plain; charset=utf-8"),
        ),
        set(&runner_log_object(), "media_type", json!("image/png")),
    ] {
        validate_local_evidence_object(&raw(&fixture)).expect_err("reject cross-pair");
        assert!(!direct_evidence_schema_valid(&fixture));
    }
}

#[test]
fn reference_validators_enforce_ids_kinds_digests_and_versions() {
    let cases = [
        (
            observation_ref(),
            validate_local_sanitized_observation_ref as Validator,
            "observation/",
            "local-evidence-object",
            "evidence/1",
            vec![
                "/observation/1",
                "http://127.0.0.1/observation/1",
                "observation\\1",
                "observation/../1",
                "observation/%2F1",
                "observation/%5C1",
            ],
        ),
        (
            evidence_ref(),
            validate_local_evidence_object_ref as Validator,
            "evidence/",
            "local-sanitized-observation",
            "observation/1",
            vec![
                "/evidence/1",
                "https://example.test/evidence/1",
                "evidence\\1",
                "evidence/../1",
                "evidence/%2F1",
                "evidence/%5C1",
            ],
        ),
    ];
    for (fixture, validate, prefix, other_kind, wrong_prefix, prohibited) in cases {
        for (field, value) in [
            ("id", json!(format!("{prefix}0"))),
            (
                "id",
                json!(format!("{prefix}{}", "1".repeat(64 - prefix.len()))),
            ),
            ("version", json!("v")),
            ("version", json!("v".repeat(64))),
        ] {
            validate(&raw(&set(&fixture, field, value))).expect("accept reference boundary");
        }
        let mut invalid = vec![
            ("id", json!(prefix)),
            ("id", json!(format!("{prefix}x"))),
            ("id", json!(wrong_prefix)),
            (
                "id",
                json!(format!("{prefix}{}", "1".repeat(65 - prefix.len()))),
            ),
            ("kind", json!(other_kind)),
            ("kind", json!("unknown")),
            ("content_digest", json!(ZERO_DIGEST)),
            (
                "content_digest",
                json!(format!("sha256:{}", "A".repeat(64))),
            ),
            (
                "content_digest",
                json!(format!("sha256:{}", "g".repeat(64))),
            ),
            (
                "content_digest",
                json!(format!("sha256:{}", "0".repeat(63))),
            ),
            (
                "content_digest",
                json!(format!("sha256:{}", "0".repeat(65))),
            ),
            ("version", json!("")),
            ("version", json!("v".repeat(65))),
        ];
        invalid.extend(prohibited.into_iter().map(|id| ("id", json!(id))));
        for (field, value) in invalid {
            validate(&raw(&set(&fixture, field, value))).expect_err("reject invalid reference");
        }
    }
}

fn direct_evidence_schema_valid(instance: &Value) -> bool {
    let registry: Value = serde_json::from_str(include_str!("../../contracts/registry.json"))
        .expect("parse registry");
    let mut schema: Value = serde_json::from_str(include_str!(
        "../../contracts/qa.local-evidence/v1/schema.json"
    ))
    .expect("parse evidence schema");
    schema.as_object_mut().expect("schema object").remove("$id");
    schema["$ref"] = registry["types"]["LocalEvidenceObject"]["pointer"].clone();
    jsonschema::draft202012::new(&schema)
        .expect("compile registered evidence schema")
        .is_valid(instance)
}

fn raw(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("serialize fixture")
}

fn set(value: &Value, field: &str, replacement: Value) -> Value {
    let mut updated = value.clone();
    updated
        .as_object_mut()
        .expect("fixture object")
        .insert(field.into(), replacement);
    updated
}

fn with(value: &Value, fields: &[&str], replacements: &[Value]) -> Value {
    let mut updated = value.clone();
    let object = updated.as_object_mut().expect("fixture object");
    for (field, replacement) in fields.iter().zip(replacements) {
        object.insert((*field).into(), replacement.clone());
    }
    updated
}

fn remove(value: &Value, field: &str) -> Value {
    let mut updated = value.clone();
    updated
        .as_object_mut()
        .expect("fixture object")
        .remove(field);
    updated
}
