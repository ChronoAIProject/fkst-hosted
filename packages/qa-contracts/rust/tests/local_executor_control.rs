use std::fs;
use std::path::PathBuf;

use fkst_qa_contracts::{
    contract_registry, validate_executor_control_report, validate_executor_control_request,
    validate_local_worker_control_frame,
};
use serde_json::Value;

#[test]
fn executor_control_contract_is_registered_and_fixture_backed() {
    let registry = contract_registry().expect("registry validates");
    assert_eq!(
        registry["schemas"]["qa.local-executor-control/v1"]["major"],
        1
    );
    let fixture: Value = serde_json::from_slice(
        &fs::read(fixture_path()).expect("executor control fixture is readable"),
    )
    .expect("executor control fixture is JSON");
    validate_executor_control_request(
        &serde_json::to_vec(&fixture["request"]).expect("request serializes"),
    )
    .expect("request validates");
    validate_executor_control_report(
        &serde_json::to_vec(&fixture["report"]).expect("report serializes"),
    )
    .expect("report validates");
}

#[test]
fn worker_control_contract_is_registered_and_fixture_backed() {
    let registry = contract_registry().expect("registry validates");
    assert_eq!(
        registry["schemas"]["qa.local-worker-control/v1"]["major"],
        1
    );
    let fixture: Value = serde_json::from_slice(
        &fs::read(worker_fixture_path()).expect("worker control fixture is readable"),
    )
    .expect("worker control fixture is JSON");
    for frame in ["abort", "cancel_ack", "control_failure"] {
        validate_local_worker_control_frame(
            &serde_json::to_vec(&fixture[frame]).expect("frame serializes"),
        )
        .expect("worker control frame validates");
    }
}

#[test]
fn executor_control_report_requires_exact_cleanup_evidence() {
    let fixture: Value = serde_json::from_slice(
        &fs::read(fixture_path()).expect("executor control fixture is readable"),
    )
    .expect("executor control fixture is JSON");
    let mut report = fixture["report"].clone();
    report
        .as_object_mut()
        .expect("report is an object")
        .remove("cleanup_receipt");
    assert!(validate_executor_control_report(
        &serde_json::to_vec(&report).expect("report serializes")
    )
    .is_err());
}

fn fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/qa.local-executor-control/v1/positive.json")
}

fn worker_fixture_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures/qa.local-worker-control/v1/positive.json")
}
