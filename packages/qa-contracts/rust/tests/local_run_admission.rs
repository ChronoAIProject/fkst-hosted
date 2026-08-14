use std::fs;
use std::path::PathBuf;

use fkst_qa_contracts::{
    build_initial_run_acceptance, canonical_bytes, contract_content_digest,
    contract_content_projection, contract_registry, validate_local_qa_run_request,
    validate_run_acceptance, verify_contract_content_digest, ContractError, ValidatedValue,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct Fixture {
    schema_version: String,
    request: Value,
    builder_inputs: BuilderInputs,
    expected_request_utf8: String,
    expected_request_projection_utf8: String,
    expected_request_digest: String,
    expected_acceptance_utf8: String,
    expected_acceptance_projection_utf8: String,
    expected_acceptance_digest: String,
}

#[derive(Deserialize)]
struct BuilderInputs {
    accepted_at: String,
    producer_version: String,
}

fn conformance() -> Value {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/qa.local-run-admission/v1/conformance.json");
    serde_json::from_slice(&fs::read(path).expect("read local run admission conformance"))
        .expect("parse local run admission conformance")
}

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/qa.local-run-admission/v1/happy-path.json");
    serde_json::from_slice(&fs::read(path).expect("read local run admission fixture"))
        .expect("parse local run admission fixture")
}

fn raw(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("serialize fixture value")
}

fn set_pointer(value: &mut Value, pointer: &str, replacement: Value) {
    if let Some(slot) = value.pointer_mut(pointer) {
        *slot = replacement;
        return;
    }

    let root_member = pointer
        .strip_prefix('/')
        .filter(|member| !member.is_empty() && !member.contains('/'))
        .expect("missing fixture pointer names one root member");
    value
        .as_object_mut()
        .expect("fixture root is an object")
        .insert(root_member.to_owned(), replacement);
}

fn assert_rejection(error: ContractError, expected: &Value) {
    assert_eq!(error.0.category, expected["category"].as_str().unwrap());
    assert_eq!(error.0.code, expected.get("code").and_then(Value::as_str));
    assert_eq!(error.0.reason, expected["reason"].as_str().unwrap());
    assert_eq!(error.0.path, expected["path"].as_str().unwrap());
}

fn digest_mutation(path: &str) -> Value {
    if path.ends_with("/content_digest") {
        Value::String(format!("sha256:{}", "8".repeat(64)))
    } else if path.ends_with("/schema_version") {
        Value::String("qa.changed/v2".into())
    } else if path == "/expires_at" {
        Value::String("2026-08-14T04:06:00Z".into())
    } else if path == "/nonce" {
        Value::String("bm9uY2UtMDAwMDAy".into())
    } else if path == "/idempotency_key" {
        Value::String("idem_0002".into())
    } else {
        Value::String("changed-2".into())
    }
}

#[test]
fn walks_shared_request_fixture_into_exact_acceptance_bytes() {
    let fixture = fixture();
    assert_eq!(fixture.schema_version, "qa.local-run-admission-fixture/v1");
    let registry = contract_registry().expect("load registry");
    assert_eq!(
        registry.pointer("/types/LocalQARunRequest/schema"),
        Some(&Value::String("qa.local-run-admission/v1".into()))
    );
    assert_eq!(
        registry.pointer("/types/RunAcceptance/pointer"),
        Some(&Value::String("#/$defs/RunAcceptance".into()))
    );

    let request = validate_local_qa_run_request(&raw(&fixture.request)).expect("validate request");
    assert_eq!(
        String::from_utf8(canonical_bytes(&request).expect("canonicalize request"))
            .expect("request is UTF-8"),
        fixture.expected_request_utf8
    );
    assert_eq!(
        String::from_utf8(contract_content_projection(&request).expect("project request"))
            .expect("request projection is UTF-8"),
        fixture.expected_request_projection_utf8
    );
    assert_eq!(
        contract_content_digest(&request).expect("digest request"),
        fixture.expected_request_digest
    );
    verify_contract_content_digest(&request).expect("verify request digest");

    let acceptance = build_initial_run_acceptance(
        &request,
        &fixture.builder_inputs.accepted_at,
        &fixture.builder_inputs.producer_version,
    )
    .expect("build acceptance");
    let acceptance_bytes = canonical_bytes(&acceptance).expect("canonicalize acceptance");
    assert_eq!(
        String::from_utf8(acceptance_bytes.clone()).expect("acceptance is UTF-8"),
        fixture.expected_acceptance_utf8
    );
    assert_eq!(
        String::from_utf8(contract_content_projection(&acceptance).expect("project acceptance"))
            .expect("acceptance projection is UTF-8"),
        fixture.expected_acceptance_projection_utf8
    );
    assert_eq!(
        contract_content_digest(&acceptance).expect("digest acceptance"),
        fixture.expected_acceptance_digest
    );
    let validated_acceptance =
        validate_run_acceptance(&acceptance_bytes).expect("validate acceptance");
    verify_contract_content_digest(&validated_acceptance).expect("verify acceptance digest");
}

#[test]
fn rejects_profile_unknown_member_and_digest_mismatch() {
    let fixture = fixture();
    let mut host_profile = fixture.request.clone();
    host_profile["profile"] = Value::String("local_qa_host_mvp".into());
    assert!(validate_local_qa_run_request(&raw(&host_profile)).is_err());

    let mut unknown_member = fixture.request.clone();
    unknown_member["unknown"] = Value::Bool(true);
    assert!(validate_local_qa_run_request(&raw(&unknown_member)).is_err());

    let mut digest_mismatch = fixture.request;
    digest_mismatch["producer_version"] = Value::String("changed/1".into());
    let error = validate_local_qa_run_request(&raw(&digest_mismatch)).expect_err("digest mismatch");
    assert_eq!(error.0.code, Some("contract.digest_mismatch"));
}

#[test]
fn expires_at_equality_returns_no_acceptance() {
    let fixture = fixture();
    let request = validate_local_qa_run_request(&raw(&fixture.request)).expect("validate request");
    let acceptance: Result<ValidatedValue, ContractError> = build_initial_run_acceptance(
        &request,
        "2026-08-14T04:05:00Z",
        &fixture.builder_inputs.producer_version,
    );
    assert!(acceptance.is_err());
}

#[test]
fn applies_shared_request_rejection_corpus() {
    let fixture = fixture();
    let conformance = conformance();
    assert_eq!(
        conformance["schema_version"],
        "qa.local-run-admission-conformance/v1"
    );
    for member in conformance["missing_request_members"].as_array().unwrap() {
        let mut request = fixture.request.clone();
        request
            .as_object_mut()
            .unwrap()
            .remove(member.as_str().unwrap());
        let error = validate_local_qa_run_request(&raw(&request)).expect_err("missing member");
        assert_eq!(error.0.category, "validation");
        assert_eq!(error.0.reason, "schema_violation");
        assert_eq!(error.0.path, "/");
    }
    for entry in conformance["request_cases"].as_array().unwrap() {
        let mut request = fixture.request.clone();
        set_pointer(
            &mut request,
            entry["path"].as_str().unwrap(),
            entry["value"].clone(),
        );
        let error = validate_local_qa_run_request(&raw(&request)).expect_err("request case");
        assert_rejection(error, &entry["expected"]);
    }
    for member in conformance["nested_unknown_members"].as_array().unwrap() {
        let mut request = fixture.request.clone();
        request["source"].as_object_mut().unwrap().insert(
            member.as_str().unwrap().into(),
            if member.as_str() == Some("bytes") {
                serde_json::json!([1])
            } else {
                Value::String("secret".into())
            },
        );
        let error = validate_local_qa_run_request(&raw(&request)).expect_err("nested member");
        assert_eq!(
            (
                error.0.category,
                error.0.reason.as_str(),
                error.0.path.as_str()
            ),
            ("validation", "schema_violation", "/source")
        );
    }
    for (identity, _) in conformance["identity_kinds"].as_object().unwrap() {
        let mut request = fixture.request.clone();
        set_pointer(
            &mut request,
            &format!("/{identity}/kind"),
            Value::String("wrong-kind".into()),
        );
        let error = validate_local_qa_run_request(&raw(&request)).expect_err("wrong kind");
        assert_eq!(
            (error.0.category, error.0.reason.as_str(), error.0.path),
            (
                "validation",
                "schema_violation",
                format!("/{identity}/kind")
            )
        );
    }
}

#[test]
fn binds_every_request_projection_class_and_nested_digest() {
    let fixture = fixture();
    let conformance = conformance();
    for path in conformance["request_digest_bound_paths"]
        .as_array()
        .unwrap()
    {
        let path = path.as_str().unwrap();
        let mut request = fixture.request.clone();
        set_pointer(&mut request, path, digest_mutation(path));
        let error = validate_local_qa_run_request(&raw(&request)).expect_err("digest mismatch");
        assert_eq!(error.0.category, "contract");
        assert_eq!(error.0.code, Some("contract.digest_mismatch"));
        assert_eq!(error.0.reason, "digest_mismatch");
        assert_eq!(error.0.path, "/content_digest");
    }
}

#[test]
fn applies_builder_boundaries_without_partial_acceptance() {
    let fixture = fixture();
    let conformance = conformance();
    let request = validate_local_qa_run_request(&raw(&fixture.request)).expect("valid request");
    for entry in conformance["builder_cases"].as_array().unwrap() {
        let result = build_initial_run_acceptance(
            &request,
            entry["accepted_at"].as_str().unwrap(),
            &fixture.builder_inputs.producer_version,
        );
        if entry["accepted"].as_bool().unwrap() {
            assert_eq!(result.unwrap().value()["accepted_at"], entry["accepted_at"]);
        } else {
            let error = result.expect_err("builder rejection");
            assert_eq!(error.0.code, Some("contract.invalid_relation"));
            assert_eq!(error.0.reason, "accepted_at_out_of_window");
            assert_eq!(error.0.path, "/accepted_at");
        }
    }
}

#[test]
fn rejects_acceptance_mutations_and_created_at_mismatch() {
    let fixture = fixture();
    let conformance = conformance();
    let acceptance: Value = serde_json::from_str(&fixture.expected_acceptance_utf8).unwrap();
    for member in conformance["missing_acceptance_members"]
        .as_array()
        .unwrap()
    {
        let mut changed = acceptance.clone();
        changed
            .as_object_mut()
            .unwrap()
            .remove(member.as_str().unwrap());
        let error = validate_run_acceptance(&raw(&changed)).expect_err("missing acceptance member");
        assert_eq!(
            (
                error.0.category,
                error.0.reason.as_str(),
                error.0.path.as_str()
            ),
            ("validation", "schema_violation", "/")
        );
    }
    let mut unknown = acceptance.clone();
    unknown["unknown"] = Value::Bool(true);
    let error = validate_run_acceptance(&raw(&unknown)).expect_err("unknown acceptance member");
    assert_eq!(
        (
            error.0.category,
            error.0.reason.as_str(),
            error.0.path.as_str()
        ),
        ("validation", "schema_violation", "/")
    );
    for path in conformance["acceptance_digest_bound_paths"]
        .as_array()
        .unwrap()
    {
        let path = path.as_str().unwrap();
        let mut changed = acceptance.clone();
        let replacement = if path == "/state" {
            Value::String("running".into())
        } else if path.ends_with("_at") {
            Value::String("2026-08-14T04:00:02Z".into())
        } else if path.contains("digest") {
            Value::String(format!("sha256:{}", "8".repeat(64)))
        } else {
            Value::String("changed-2".into())
        };
        set_pointer(&mut changed, path, replacement);
        assert!(validate_run_acceptance(&raw(&changed)).is_err());
    }
    let mut mismatch = acceptance;
    mismatch["created_at"] = Value::String("2026-08-14T04:00:02Z".into());
    let error = validate_run_acceptance(&raw(&mismatch)).expect_err("timestamp mismatch");
    assert_eq!(error.0.code, Some("contract.invalid_relation"));
    assert_eq!(error.0.reason, "accepted_at_mismatch");
    assert_eq!(error.0.path, "/created_at");
}

#[test]
fn applies_strict_raw_admission_and_canonicalizes_formatting() {
    let fixture = fixture();
    let conformance = conformance();
    for entry in conformance["raw_cases"].as_array().unwrap() {
        let mut text = if entry["target"] == "request" {
            fixture.expected_request_utf8.clone()
        } else {
            fixture.expected_acceptance_utf8.clone()
        };
        if let Some(replacement) = entry.get("replace").and_then(Value::as_array) {
            text = text.replacen(
                replacement[0].as_str().unwrap(),
                replacement[1].as_str().unwrap(),
                1,
            );
        }
        if let Some(depth) = entry.get("wrap_depth").and_then(Value::as_u64) {
            text = format!(
                "{}{}{}",
                "[".repeat(depth as usize),
                text,
                "]".repeat(depth as usize)
            );
        }
        text = format!(
            "{}{}{}",
            entry.get("prefix").and_then(Value::as_str).unwrap_or(""),
            text,
            entry.get("suffix").and_then(Value::as_str).unwrap_or("")
        );
        let mut bytes: Vec<u8> = entry
            .get("hex_prefix")
            .and_then(Value::as_str)
            .map(|hex| {
                hex.as_bytes()
                    .chunks(2)
                    .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap())
                    .collect()
            })
            .unwrap_or_default();
        bytes.extend_from_slice(text.as_bytes());
        let error = if entry["target"] == "request" {
            validate_local_qa_run_request(&bytes)
        } else {
            validate_run_acceptance(&bytes)
        }
        .expect_err("raw rejection");
        assert_rejection(error, &entry["expected"]);
    }
    let spaced = fixture
        .expected_request_utf8
        .replace(",\"", ", \"")
        .replace("\":", "\": ");
    let validated = validate_local_qa_run_request(spaced.as_bytes()).expect("formatted request");
    assert_eq!(
        String::from_utf8(canonical_bytes(&validated).unwrap()).unwrap(),
        fixture.expected_request_utf8
    );
}
