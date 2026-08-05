use fkst_qa_contracts::{
    contract_registry, validate_local_sanitized_observation, ContractError, Rejection,
};
use serde_json::{json, Value};

const ACCEPTED_JSON: &[u8] = br#"{"schema_version":"qa.local-evidence/v1","run_id":"run_1","attempt":1,"fixture_url":"http://127.0.0.1:3210/fixed-page.html","final_url":"http://127.0.0.1:3210/fixed-page.html","selector":"[data-local-qa=\"status\"]","expected_text":"READY","observed_text":"READY"}"#;
const DUPLICATE_RUN_ID_JSON: &[u8] = br#"{"schema_version":"qa.local-evidence/v1","run_id":"run_1","run_id":"run_2","attempt":1,"fixture_url":"http://127.0.0.1:3210/fixed-page.html","final_url":"http://127.0.0.1:3210/fixed-page.html","selector":"[data-local-qa=\"status\"]","expected_text":"READY","observed_text":"READY"}"#;

fn accepted_observation() -> Value {
    json!({
        "schema_version": "qa.local-evidence/v1",
        "run_id": "run_1",
        "attempt": 1,
        "fixture_url": "http://127.0.0.1:3210/fixed-page.html",
        "final_url": "http://127.0.0.1:3210/fixed-page.html",
        "selector": "[data-local-qa=\"status\"]",
        "expected_text": "READY",
        "observed_text": "READY"
    })
}

#[test]
fn fixed_passing_mvp_observation_walks_registry_and_rust_validator() {
    let registry = contract_registry().expect("load contract registry");
    assert_eq!(
        registry.pointer("/schemas/qa.local-evidence~1v1"),
        Some(&json!({
            "path": "contracts/qa.local-evidence/v1/schema.json",
            "id": "urn:chronoai:fkst:qa-contracts:qa.local-evidence:v1",
            "major": 1
        }))
    );
    assert_eq!(
        registry.pointer("/types/LocalSanitizedObservation"),
        Some(&json!({
            "schema": "qa.local-evidence/v1",
            "pointer": "#/$defs/LocalSanitizedObservation"
        }))
    );

    let validated = validate_local_sanitized_observation(ACCEPTED_JSON)
        .expect("validate fixed passing MVP observation");
    assert_eq!(validated.value(), &accepted_observation());
}

#[test]
fn local_sanitized_observation_rejects_unequal_urls_and_unknown_fields() {
    assert_rejection(
        validate_local_sanitized_observation(&raw_with(json!({
            "final_url": "http://127.0.0.1:3211/fixed-page.html"
        })))
        .expect_err("reject unequal URLs"),
        "contract",
        "contract.invalid_relation",
        "fixture_url_mismatch",
        "/final_url",
    );
    validate_local_sanitized_observation(&raw_with(json!({ "uploadable": true })))
        .expect_err("reject unknown field");
}

#[test]
fn local_sanitized_observation_retains_strict_duplicate_key_admission() {
    assert_rejection(
        validate_local_sanitized_observation(DUPLICATE_RUN_ID_JSON)
            .expect_err("reject duplicate run_id"),
        "canonicalization",
        "canonicalization.duplicate_member",
        "duplicate_member",
        "/",
    );
}

#[test]
fn local_sanitized_observation_rejects_malformed_identifiers_attempts_and_literals() {
    for replacement in [
        json!({ "run_id": "-run" }),
        json!({ "run_id": "a".repeat(65) }),
        json!({ "attempt": 1.5 }),
        json!({ "attempt": 0 }),
        json!({ "attempt": 9_007_199_254_740_992_u64 }),
        json!({ "schema_version": "qa.local-evidence/v2" }),
        json!({ "selector": "[data-local-qa=\"other\"]" }),
        json!({ "expected_text": "WAIT" }),
        json!({ "observed_text": "WAIT" }),
    ] {
        validate_local_sanitized_observation(&raw_with(replacement))
            .expect_err("reject malformed observation");
    }
}

#[test]
fn local_sanitized_observation_rejects_every_prohibited_url_form() {
    for url in [
        "http://localhost:3210/fixed-page.html",
        "http://127.0.0.1:0/fixed-page.html",
        "http://127.0.0.1:65536/fixed-page.html",
        "http://user@127.0.0.1:3210/fixed-page.html",
        "http://[::1]:3210/fixed-page.html",
        "http://127.0.0.1:3210/fixed-page.html?x=1",
        "http://127.0.0.1:3210/fixed-page.html#status",
        "http://127.0.0.1:3210/%66ixed-page.html",
        "http://127.0.0.1:3210/fixed-page.html/extra",
    ] {
        validate_local_sanitized_observation(&raw_with(json!({ "fixture_url": url })))
            .expect_err("reject prohibited fixture_url");
        validate_local_sanitized_observation(&raw_with(json!({ "final_url": url })))
            .expect_err("reject prohibited final_url");
    }
}

fn raw_with(replacement: Value) -> Vec<u8> {
    let mut observation = accepted_observation();
    let object = observation.as_object_mut().expect("observation object");
    for (key, value) in replacement.as_object().expect("replacement object") {
        object.insert(key.clone(), value.clone());
    }
    serde_json::to_vec(&observation).expect("serialize observation")
}

fn assert_rejection(
    error: ContractError,
    category: &'static str,
    code: &'static str,
    reason: &str,
    path: &str,
) {
    assert_eq!(
        error.0,
        Rejection {
            category,
            code: Some(code),
            reason: reason.into(),
            path: path.into(),
        }
    );
}
