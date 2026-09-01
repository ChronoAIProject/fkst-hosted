use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use fkst_qa_contracts::{
    contract_registry, validate_executor_control_report, validate_executor_control_request,
    validate_local_worker_control_frame,
};
use serde_json::Value;

#[test]
fn control_contract_registry_and_fixture_indexes_are_closed() {
    let registry = contract_registry().expect("registry validates");
    assert_eq!(
        registry["schemas"]["qa.local-worker-control/v1"],
        serde_json::json!({
            "path": "contracts/qa.local-worker-control/v1/schema.json",
            "id": "urn:chronoai:fkst:qa-contracts:qa.local-worker-control:v1",
            "major": 1
        })
    );
    assert_eq!(
        registry["schemas"]["qa.local-executor-control/v1"],
        serde_json::json!({
            "path": "contracts/qa.local-executor-control/v1/schema.json",
            "id": "urn:chronoai:fkst:qa-contracts:qa.local-executor-control:v1",
            "major": 1
        })
    );

    assert_keys(
        &fixture("qa.local-executor-control/v1/positive.json"),
        &["reports", "request"],
    );
    assert_keys(
        &fixture("qa.local-executor-control/v1/negative.json"),
        &["relation_cases", "schema_cases"],
    );
    assert_keys(
        &fixture("qa.local-worker-control/v1/positive.json"),
        &["enums", "frames"],
    );
    assert_keys(
        &fixture("qa.local-cancellation/v1/conformance.json"),
        &[
            "cleanup_outcome",
            "cleanup_receipt",
            "control_status",
            "effect_disposition",
            "independent_outcome",
            "sanitized_residual",
        ],
    );
    assert_keys(
        &fixture("qa.local-worker-control/v1/negative.json"),
        &["relation_cases", "schema_cases"],
    );
    let cancellation = fixture("qa.local-cancellation/v1/conformance.json");
    assert_eq!(
        cancellation["control_status"]["positive"],
        serde_json::json!(["accepted", "too_late", "rejected", "failed"])
    );
    assert_eq!(
        cancellation["effect_disposition"]["positive"],
        serde_json::json!(["not_started", "completed", "uncertain"])
    );
    assert_eq!(
        cancellation["independent_outcome"]["positive"],
        serde_json::json!([
            "not_started",
            "succeeded",
            "failed",
            "cancelled",
            "lost_or_inconclusive"
        ])
    );
    assert_eq!(
        cancellation["cleanup_outcome"]["positive"],
        serde_json::json!(["not_required", "completed", "blocked"])
    );
    let executor_negative = fixture("qa.local-executor-control/v1/negative.json");
    assert_eq!(executor_negative["schema_cases"].as_array().map(Vec::len), Some(16));
    assert_eq!(executor_negative["relation_cases"].as_array().map(Vec::len), Some(7));
    let worker_negative = fixture("qa.local-worker-control/v1/negative.json");
    assert_eq!(worker_negative["schema_cases"].as_array().map(Vec::len), Some(15));
    assert_eq!(worker_negative["relation_cases"].as_array().map(Vec::len), Some(3));
}

#[test]
fn validates_every_control_frame_report_and_enum_value() {
    let executor = fixture("qa.local-executor-control/v1/positive.json");
    validate_executor_control_request(&raw(&executor["request"]))
        .expect("executor control request validates");
    for report in executor["reports"]
        .as_object()
        .expect("reports is an object")
        .values()
    {
        validate_executor_control_report(&raw(report)).expect("executor control report validates");
    }

    assert_keys(
        &executor["reports"],
        &["cleanup_receipt", "residual"],
    );
    let cancellation = fixture("qa.local-cancellation/v1/conformance.json");
    assert_eq!(
        executor["reports"]["cleanup_receipt"]["cleanup_receipt"],
        cancellation["cleanup_receipt"]["positive"]
    );
    assert_eq!(
        executor["reports"]["residual"]["residual"],
        cancellation["sanitized_residual"]["positive"]
    );
    let baseline = &executor["reports"]["cleanup_receipt"];
    for (field, vocabulary) in [
        ("status", &cancellation["control_status"]["positive"]),
        (
            "effect_disposition",
            &cancellation["effect_disposition"]["positive"],
        ),
        (
            "execution_outcome",
            &cancellation["independent_outcome"]["positive"],
        ),
        (
            "evidence_outcome",
            &cancellation["independent_outcome"]["positive"],
        ),
        (
            "upload_outcome",
            &cancellation["independent_outcome"]["positive"],
        ),
        (
            "cleanup_outcome",
            &cancellation["cleanup_outcome"]["positive"],
        ),
    ] {
        for value in vocabulary.as_array().expect("enum vocabulary is an array") {
            let mut report = baseline.clone();
            report
                .as_object_mut()
                .expect("report is an object")
                .insert(field.to_owned(), value.clone());
            validate_executor_control_report(&raw(&report))
                .unwrap_or_else(|_| panic!("{field} value {value} validates"));
        }
    }

    let worker = fixture("qa.local-worker-control/v1/positive.json");
    assert_keys(
        &worker["frames"],
        &[
            "abort",
            "cancel_ack.accepted",
            "cancel_ack.too_late",
            "control_failure.control.conflict",
            "control_failure.control.deadline_elapsed",
            "control_failure.control.invalid_frame",
            "control_failure.control.invalid_invocation",
        ],
    );
    for frame in worker["frames"]
        .as_object()
        .expect("frames is an object")
        .values()
    {
        validate_local_worker_control_frame(&raw(frame)).expect("worker control frame validates");
    }
}

#[test]
fn rejects_the_shared_closed_schema_matrix() {
    let executor_positive = fixture("qa.local-executor-control/v1/positive.json");
    let executor_negative = fixture("qa.local-executor-control/v1/negative.json");
    for fixture_case in executor_negative["schema_cases"]
        .as_array()
        .expect("schema cases is an array")
    {
        let target = fixture_case["target"].as_str().expect("target is a string");
        let baseline = if target == "request" {
            &executor_positive["request"]
        } else {
            &executor_positive["reports"]["cleanup_receipt"]
        };
        let mutated = mutate(baseline, fixture_case);
        let rejected = if target == "request" {
            validate_executor_control_request(&raw(&mutated)).is_err()
        } else {
            validate_executor_control_report(&raw(&mutated)).is_err()
        };
        assert!(rejected, "{}", fixture_case["case_id"]);
    }

    let cancellation = fixture("qa.local-cancellation/v1/conformance.json");
    let report = &executor_positive["reports"]["cleanup_receipt"];
    for (field, vocabulary) in [
        ("status", &cancellation["control_status"]["negative"]),
        (
            "effect_disposition",
            &cancellation["effect_disposition"]["negative"],
        ),
        (
            "execution_outcome",
            &cancellation["independent_outcome"]["negative"],
        ),
        (
            "evidence_outcome",
            &cancellation["independent_outcome"]["negative"],
        ),
        (
            "upload_outcome",
            &cancellation["independent_outcome"]["negative"],
        ),
        (
            "cleanup_outcome",
            &cancellation["cleanup_outcome"]["negative"],
        ),
    ] {
        for value in vocabulary.as_array().expect("negative vocabulary is an array") {
            let mut mutated = report.clone();
            mutated
                .as_object_mut()
                .expect("report is an object")
                .insert(field.to_owned(), value.clone());
            assert!(validate_executor_control_report(&raw(&mutated)).is_err());
        }
    }
    for fixture_case in cancellation["cleanup_receipt"]["negative"]
        .as_array()
        .expect("cleanup receipt cases is an array")
    {
        let mut mutated = report.clone();
        mutated["cleanup_receipt"] = mutate(
            &cancellation["cleanup_receipt"]["positive"],
            fixture_case,
        );
        assert!(validate_executor_control_report(&raw(&mutated)).is_err());
    }
    for fixture_case in cancellation["sanitized_residual"]["negative"]
        .as_array()
        .expect("residual cases is an array")
    {
        let mut mutated = executor_positive["reports"]["residual"].clone();
        mutated["residual"] = mutate(
            &cancellation["sanitized_residual"]["positive"],
            fixture_case,
        );
        assert!(validate_executor_control_report(&raw(&mutated)).is_err());
    }

    let worker_positive = fixture("qa.local-worker-control/v1/positive.json");
    let worker_negative = fixture("qa.local-worker-control/v1/negative.json");
    for fixture_case in worker_negative["schema_cases"]
        .as_array()
        .expect("schema cases is an array")
    {
        let frame_name = fixture_case["frame"].as_str().expect("frame is a string");
        let mutated = mutate(&worker_positive["frames"][frame_name], fixture_case);
        assert!(
            validate_local_worker_control_frame(&raw(&mutated)).is_err(),
            "{}",
            fixture_case["case_id"]
        );
    }
}

#[test]
fn applies_the_shared_identity_relation_matrix() {
    let executor_positive = fixture("qa.local-executor-control/v1/positive.json");
    let executor_negative = fixture("qa.local-executor-control/v1/negative.json");
    for fixture_case in executor_negative["relation_cases"]
        .as_array()
        .expect("relation cases is an array")
    {
        let request = &executor_positive["request"];
        let mut report = executor_positive["reports"]["cleanup_receipt"].clone();
        if let Some(field) = fixture_case["field"].as_str() {
            report
                .as_object_mut()
                .expect("report is an object")
                .insert(field.to_owned(), fixture_case["value"].clone());
        }
        assert_eq!(
            executor_relation_valid(request, &report),
            fixture_case["valid"].as_bool().expect("valid is a boolean"),
            "{}",
            fixture_case["case_id"]
        );
    }

    let worker_positive = fixture("qa.local-worker-control/v1/positive.json");
    let worker_negative = fixture("qa.local-worker-control/v1/negative.json");
    for fixture_case in worker_negative["relation_cases"]
        .as_array()
        .expect("relation cases is an array")
    {
        let abort = &worker_positive["frames"]["abort"];
        let mut acknowledgement = worker_positive["frames"]["cancel_ack.accepted"].clone();
        if let Some(field) = fixture_case["field"].as_str() {
            acknowledgement
                .as_object_mut()
                .expect("acknowledgement is an object")
                .insert(field.to_owned(), fixture_case["value"].clone());
        }
        assert_eq!(
            worker_relation_valid(abort, &acknowledgement),
            fixture_case["valid"].as_bool().expect("valid is a boolean"),
            "{}",
            fixture_case["case_id"]
        );
    }
}

fn fixture(relative_path: &str) -> Value {
    serde_json::from_slice(
        &fs::read(fixture_path(relative_path)).expect("control fixture is readable"),
    )
    .expect("control fixture is JSON")
}

fn fixture_path(relative_path: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../fixtures")
        .join(relative_path)
}

fn raw(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("fixture value serializes")
}

fn mutate(baseline: &Value, fixture_case: &Value) -> Value {
    let mut value = baseline.clone();
    let field = fixture_case["path"][0]
        .as_str()
        .expect("mutation field is a string");
    let object = value.as_object_mut().expect("mutation target is an object");
    match fixture_case["operation"]
        .as_str()
        .expect("operation is a string")
    {
        "remove" => {
            object.remove(field);
        }
        "set" | "add" => {
            object.insert(field.to_owned(), fixture_case["value"].clone());
        }
        operation => panic!("unsupported mutation operation {operation}"),
    }
    value
}

fn executor_relation_valid(request: &Value, report: &Value) -> bool {
    ["control_id", "run_id", "executor_run_id"]
        .into_iter()
        .all(|field| request[field] == report[field])
        && request["selection"]["executor_id"] == report["executor_id"]
        && request["selection"]["executor_version"] == report["executor_version"]
        && request["selection"]["capability_digest"] == report["capability_digest"]
}

fn worker_relation_valid(abort: &Value, acknowledgement: &Value) -> bool {
    abort["control_id"] == acknowledgement["control_id"]
        && abort["invocation_id"] == acknowledgement["invocation_id"]
}

fn assert_keys(value: &Value, expected: &[&str]) {
    let actual = value
        .as_object()
        .expect("fixture root is an object")
        .keys()
        .map(String::as_str)
        .collect::<BTreeSet<_>>();
    let expected = expected.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(actual, expected);
}
