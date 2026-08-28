use base64::Engine;
use fkst_qa_contracts::{
    build_initial_run_acceptance_v2, canonical_bytes, contract_content_digest,
    validate_local_qa_run_request_v2, validate_run_acceptance_v2, ContractError,
};
use serde::Deserialize;

#[derive(Deserialize)]
struct Fixture {
    request: serde_json::Value,
    expected_request_utf8: String,
    accepted_at: String,
    expected_acceptance_utf8: String,
}

fn fixture() -> Fixture {
    serde_json::from_str(include_str!(
        "../../fixtures/qa.local-run-admission/v2/happy-path.json"
    ))
    .expect("v2 fixture")
}

#[test]
fn validates_the_v2_admission_walking_skeleton_vectors() {
    let fixture = fixture();
    assert_eq!(
        fixture.request["attempt_binding"]["fence_token"],
        "dGVzdC1mZW5jZS0wMDAwMDAwMg"
    );
    assert_eq!(
        base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode("dGVzdC1mZW5jZS0wMDAwMDAwMg")
            .unwrap(),
        b"test-fence-00000002"
    );
    let request = validate_local_qa_run_request_v2(fixture.expected_request_utf8.as_bytes())
        .expect("valid request");
    assert_eq!(
        contract_content_digest(&request).unwrap(),
        fixture.request["content_digest"]
    );
    assert_eq!(
        canonical_bytes(&request).unwrap(),
        fixture.expected_request_utf8.as_bytes()
    );

    let acceptance =
        build_initial_run_acceptance_v2(&request, &fixture.accepted_at, "fkst-local-qa-host/0.1.0")
            .expect("valid acceptance");
    assert_eq!(
        canonical_bytes(&acceptance).unwrap(),
        fixture.expected_acceptance_utf8.as_bytes()
    );
    assert_eq!(
        contract_content_digest(&acceptance).unwrap(),
        "sha256:193ec4c65b3c5a16334c2ae2688c827e6246170a51e5f1987305fe28ce7b5ef5"
    );
    validate_run_acceptance_v2(fixture.expected_acceptance_utf8.as_bytes())
        .expect("valid acceptance fixture");
}

fn assert_rejection(raw: String, category: &str, code: Option<&str>, path: &str) {
    let ContractError(rejection) =
        validate_local_qa_run_request_v2(raw.as_bytes()).expect_err("request must reject");
    assert_eq!(rejection.category, category);
    assert_eq!(rejection.code, code);
    assert_eq!(rejection.path, path);
}

#[test]
fn rejects_non_canonical_v2_encoded_identities() {
    let fixture = fixture();
    assert_rejection(
        fixture.expected_request_utf8.replace(
            "dGVzdC1mZW5jZS0wMDAwMDAwMg",
            "test-fence-00000002",
        ),
        "contract",
        Some("contract.invalid_encoding"),
        "/attempt_binding/fence_token",
    );
    assert_rejection(
        fixture
            .expected_request_utf8
            .replace("bm9uY2UtMDAwMDAwMDAy", "test-fence-00000002"),
        "contract",
        Some("contract.invalid_encoding"),
        "/nonce",
    );
}

#[test]
fn enforces_the_v2_producer_version_utf8_byte_limit() {
    let fixture = fixture();
    assert_rejection(
        fixture
            .expected_request_utf8
            .replace("fkst-local-qa-host/0.1.0", &"é".repeat(65)),
        "validation",
        None,
        "/producer_version",
    );
}

#[test]
fn enforces_the_v2_acceptance_producer_version_utf8_byte_limit() {
    let fixture = fixture();
    let ContractError(rejection) = validate_run_acceptance_v2(
        fixture
            .expected_acceptance_utf8
            .replace("fkst-local-qa-host/0.1.0", &"é".repeat(65))
            .as_bytes(),
    )
    .expect_err("acceptance must reject");
    assert_eq!(rejection.category, "validation");
    assert_eq!(rejection.code, None);
    assert_eq!(rejection.path, "/producer_version");
}
