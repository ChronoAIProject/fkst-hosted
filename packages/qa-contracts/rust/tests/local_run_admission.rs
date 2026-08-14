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

fn fixture() -> Fixture {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/qa.local-run-admission/v1/happy-path.json");
    serde_json::from_slice(&fs::read(path).expect("read local run admission fixture"))
        .expect("parse local run admission fixture")
}

fn raw(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("serialize fixture value")
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

    let request =
        validate_local_qa_run_request(&raw(&fixture.request)).expect("validate request");
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
    let error =
        validate_local_qa_run_request(&raw(&digest_mismatch)).expect_err("digest mismatch");
    assert_eq!(error.0.code, Some("contract.digest_mismatch"));
}

#[test]
fn expires_at_equality_returns_no_acceptance() {
    let fixture = fixture();
    let request =
        validate_local_qa_run_request(&raw(&fixture.request)).expect("validate request");
    let acceptance: Result<ValidatedValue, ContractError> = build_initial_run_acceptance(
        &request,
        "2026-08-14T04:05:00Z",
        &fixture.builder_inputs.producer_version,
    );
    assert!(acceptance.is_err());
}
