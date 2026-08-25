use fkst_qa_contracts::{
    build_initial_run_acceptance_v2, canonical_bytes, contract_content_digest,
    validate_local_qa_run_request_v2, validate_run_acceptance_v2,
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
    let request = validate_local_qa_run_request_v2(fixture.expected_request_utf8.as_bytes())
        .expect("valid request");
    assert_eq!(
        contract_content_digest(&request).unwrap(),
        fixture.request["content_digest"]
    );
    assert_eq!(canonical_bytes(&request).unwrap(), fixture.expected_request_utf8.as_bytes());

    let acceptance = build_initial_run_acceptance_v2(
        &request,
        &fixture.accepted_at,
        "fkst-local-qa-host/0.1.0",
    )
    .expect("valid acceptance");
    assert_eq!(canonical_bytes(&acceptance).unwrap(), fixture.expected_acceptance_utf8.as_bytes());
    assert_eq!(
        contract_content_digest(&acceptance).unwrap(),
        "sha256:c590e3ffd6ca7d36e1a62e4ebb8f5799f7f879d0abff82422497c1bcba0f399d"
    );
    validate_run_acceptance_v2(fixture.expected_acceptance_utf8.as_bytes())
        .expect("valid acceptance fixture");
}
