use fkst_qa_contracts::{
    admit_json, canonical_admitted_bytes, canonical_bytes, contract_content_digest,
    contract_registry, local_fixed_json_policy, sha256_digest, validate_local_fixed_json_export,
    validate_local_fixed_json_receipt, validate_local_fixed_json_source,
    LOCAL_FIXED_JSON_MAX_BYTES, LOCAL_FIXED_JSON_POLICY_DIGEST, LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES,
};
use serde_json::Value;

fn fixture() -> Value {
    serde_json::from_str(include_str!(
        "../../fixtures/qa.local-fixed-json-export/v1/conformance.json"
    ))
    .unwrap()
}

fn rehash(mut receipt: Value) -> Vec<u8> {
    receipt.as_object_mut().unwrap().remove("content_digest");
    let admitted = admit_json(&serde_json::to_vec(&receipt).unwrap()).unwrap();
    receipt["content_digest"] = sha256_digest(&canonical_admitted_bytes(&admitted).unwrap()).into();
    serde_json::to_vec(&receipt).unwrap()
}

#[test]
fn shared_source_corpus_is_strict_bounded_and_private() {
    let fixture = fixture();
    for case in fixture["source_cases"].as_array().unwrap() {
        let raw = case["raw_utf8"].as_str().unwrap().as_bytes();
        let result = validate_local_fixed_json_source(raw);
        if case["accepted"] == true {
            let validated = result.unwrap_or_else(|error| panic!("{}: {error}", case["id"]));
            let output = canonical_bytes(&validated).unwrap();
            assert_eq!(output, case["canonical_utf8"].as_str().unwrap().as_bytes());
            assert_eq!(sha256_digest(raw), case["source_digest"]);
            assert_eq!(sha256_digest(&output), case["output_digest"]);
            validate_local_fixed_json_source(&output).unwrap();
        } else {
            let error = result.unwrap_err();
            assert_eq!(error.0.reason, "invalid_local_fixed_json_export");
            assert_eq!(error.0.path, "/");
            assert!(!format!("{error:?} {error}").contains(fixture["canary"].as_str().unwrap()));
        }
    }
    let png = fixture["png_hex"].as_str().unwrap();
    let bytes: Vec<u8> = (0..png.len())
        .step_by(2)
        .map(|offset| u8::from_str_radix(&png[offset..offset + 2], 16).unwrap())
        .collect();
    assert!(validate_local_fixed_json_source(&bytes).is_err());
}

#[test]
fn receipt_binds_source_output_policy_and_identity_in_both_languages() {
    let fixture = fixture();
    let source = fixture["source_cases"][0]["raw_utf8"]
        .as_str()
        .unwrap()
        .as_bytes();
    let output = fixture["source_cases"][0]["canonical_utf8"]
        .as_str()
        .unwrap()
        .as_bytes();
    let receipt = serde_json::to_vec(&fixture["receipt"]).unwrap();
    let validated = validate_local_fixed_json_export(&receipt, source, output).unwrap();
    assert_eq!(validated.value(), &fixture["receipt"]);
    assert_ne!(
        validated.value()["source_digest"],
        validated.value()["output_digest"]
    );
    for case in fixture["rejected_bindings"].as_array().unwrap() {
        let mut changed = fixture["receipt"].clone();
        changed
            .as_object_mut()
            .unwrap()
            .extend(case["patch"].as_object().unwrap().clone());
        // Recompute self digest so each rejection exercises the actual binding rule.
        let error = validate_local_fixed_json_export(&rehash(changed), source, output).unwrap_err();
        assert_eq!(
            error.0.reason, "invalid_local_fixed_json_export",
            "{}",
            case["id"]
        );
        assert!(!format!("{error:?} {error}").contains(fixture["canary"].as_str().unwrap()));
    }
    assert!(validate_local_fixed_json_receipt(
        fixture["duplicate_receipt_utf8"]
            .as_str()
            .unwrap()
            .as_bytes()
    )
    .is_err());
    assert!(validate_local_fixed_json_export(&receipt, output, output).is_err());
    assert!(validate_local_fixed_json_export(&receipt, source, source).is_err());
}

#[test]
fn exact_preparse_size_limits_and_no_unknown_field_stripping() {
    let fixture = fixture();
    assert_eq!(fixture["max_source_bytes"], LOCAL_FIXED_JSON_MAX_BYTES);
    assert_eq!(
        fixture["max_receipt_bytes"],
        LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES
    );
    let original = fixture["source_cases"][0]["raw_utf8"].as_str().unwrap();
    let exact = format!(
        "{{{}{}",
        " ".repeat(LOCAL_FIXED_JSON_MAX_BYTES - original.len()),
        &original[1..]
    );
    assert_eq!(exact.len(), LOCAL_FIXED_JSON_MAX_BYTES);
    validate_local_fixed_json_source(exact.as_bytes()).unwrap();
    let excessive = exact.replacen('{', "{ ", 1);
    assert!(validate_local_fixed_json_source(excessive.as_bytes()).is_err());
    let receipt = serde_json::to_string(&fixture["receipt"]).unwrap();
    let exact_receipt = format!(
        "{{{}{}",
        " ".repeat(LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES - receipt.len()),
        &receipt[1..]
    );
    validate_local_fixed_json_receipt(exact_receipt.as_bytes()).unwrap();
    assert!(
        validate_local_fixed_json_receipt(exact_receipt.replacen('{', "{ ", 1).as_bytes()).is_err()
    );
    assert!(validate_local_fixed_json_source(&vec![b'x'; LOCAL_FIXED_JSON_MAX_BYTES + 1]).is_err());
    assert!(validate_local_fixed_json_export(
        receipt.as_bytes(),
        original.as_bytes(),
        &vec![b'x'; LOCAL_FIXED_JSON_MAX_BYTES + 1]
    )
    .is_err());
}

#[test]
fn registry_policy_descriptor_is_immutable_and_explicitly_local() {
    let registry = contract_registry().unwrap();
    for name in ["LocalFixedJsonPolicy", "LocalFixedJsonReceipt"] {
        assert_eq!(
            registry["types"][name]["schema"],
            "qa.local-fixed-json-export/v1"
        );
    }
    let policy = local_fixed_json_policy().unwrap();
    assert_eq!(
        contract_content_digest(&policy).unwrap(),
        LOCAL_FIXED_JSON_POLICY_DIGEST
    );
    assert_eq!(
        policy.value()["profile"],
        "local-host-built-in-fixed-observation"
    );
    assert_eq!(policy.value()["png"], "deny");
    assert_eq!(policy.value()["raw_logs"], "deny");
}
