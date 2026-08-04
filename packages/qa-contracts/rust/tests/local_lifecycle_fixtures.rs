use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use fkst_qa_contracts::{
    canonical_bytes, contract_registry, sha256_digest, validate_execution_outcome,
    validate_local_state, ContractError, Rejection, ValidatedValue,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct LifecycleFixture {
    schema_version: String,
    valid_cases: Vec<LifecycleValidCase>,
    invalid_cases: Vec<LifecycleInvalidCase>,
}

#[derive(Deserialize)]
struct LifecycleValidCase {
    case_id: String,
    lifecycle_type: LifecycleType,
    source: Value,
    expected_canonical_utf8_hex: String,
    expected_canonical_utf8_base64: String,
    expected_sha256: String,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum LifecycleType {
    LocalState,
    ExecutionOutcome,
}

#[derive(Deserialize)]
struct LifecycleInvalidCase {
    case_id: String,
    source: Value,
    expected: ExpectedRejection,
}

#[derive(Debug, Deserialize)]
struct ExpectedRejection {
    category: String,
    code: Option<String>,
    reason: String,
    path: String,
}

#[test]
fn local_lifecycle_fixture_walks_the_production_path() {
    let fixture: LifecycleFixture =
        load_json(&repo_root().join("fixtures/qa/local-lifecycle-v1.json"));
    assert_eq!(fixture.schema_version, "qa.local-lifecycle-fixtures/v1");
    assert_eq!(
        fixture
            .valid_cases
            .iter()
            .map(|fixture_case| (fixture_case.lifecycle_type, fixture_case.source.as_str()))
            .collect::<Vec<_>>(),
        vec![
            (LifecycleType::LocalState, Some("accepted")),
            (LifecycleType::LocalState, Some("preparing")),
            (LifecycleType::LocalState, Some("ready")),
            (LifecycleType::LocalState, Some("executing")),
            (LifecycleType::LocalState, Some("staging_evidence")),
            (LifecycleType::LocalState, Some("cleaning_up_execution")),
            (LifecycleType::LocalState, Some("uploading")),
            (LifecycleType::LocalState, Some("finalizing_local")),
            (LifecycleType::LocalState, Some("terminal")),
            (LifecycleType::ExecutionOutcome, Some("passed")),
        ]
    );
    let registry = contract_registry().expect("load contract registry");
    assert_eq!(
        registry
            .pointer("/types/ExecutionOutcome/schema")
            .and_then(Value::as_str),
        Some("qa.local-lifecycle/v1")
    );
    assert_eq!(
        registry
            .pointer("/types/ExecutionOutcome/pointer")
            .and_then(Value::as_str),
        Some("#/$defs/ExecutionOutcome")
    );

    for fixture_case in &fixture.valid_cases {
        println!("case_id={}", fixture_case.case_id);
        let raw = serde_json::to_vec(&fixture_case.source).expect("serialize fixture source");
        let validated = validate_lifecycle_case(fixture_case.lifecycle_type, &raw);
        if fixture_case.case_id == "execution-outcome-passed" {
            assert_eq!(fixture_case.lifecycle_type, LifecycleType::ExecutionOutcome);
            assert_eq!(raw, br#""passed""#);
            assert_eq!(validated.value().as_str(), Some("passed"));
        }
        assert_eq!(validated.value(), &fixture_case.source);
        let canonical = canonical_bytes(&validated).expect("canonical lifecycle bytes");
        assert_eq!(hex(&canonical), fixture_case.expected_canonical_utf8_hex);
        assert_eq!(
            base64::engine::general_purpose::STANDARD.encode(&canonical),
            fixture_case.expected_canonical_utf8_base64
        );
        assert_eq!(sha256_digest(&canonical), fixture_case.expected_sha256);
    }

    for fixture_case in &fixture.invalid_cases {
        println!("case_id={}", fixture_case.case_id);
        let raw = serde_json::to_vec(&fixture_case.source).expect("serialize fixture source");
        let error = validate_local_state(&raw).expect_err("reject invalid LocalState");
        assert_rejection(&fixture_case.case_id, &error, &fixture_case.expected);
    }

    for (case_id, raw, expected) in [
        (
            "local-state-malformed-json",
            &[0x22],
            ExpectedRejection {
                category: "validation".into(),
                code: None,
                reason: "invalid_json".into(),
                path: "/".into(),
            },
        ),
        (
            "local-state-invalid-utf8",
            &[0xff],
            ExpectedRejection {
                category: "canonicalization".into(),
                code: Some("canonicalization.invalid_utf8".into()),
                reason: "invalid_utf8".into(),
                path: "/".into(),
            },
        ),
    ] {
        println!("case_id={case_id}");
        let error = validate_local_state(raw).expect_err("reject invalid LocalState bytes");
        assert_rejection(case_id, &error, &expected);
    }
}

fn validate_lifecycle_case(lifecycle_type: LifecycleType, raw: &[u8]) -> ValidatedValue {
    match lifecycle_type {
        LifecycleType::LocalState => validate_local_state(raw).expect("validate LocalState"),
        LifecycleType::ExecutionOutcome => {
            validate_execution_outcome(raw).expect("validate ExecutionOutcome")
        }
    }
}

fn assert_rejection(case_id: &str, actual: &ContractError, expected: &ExpectedRejection) {
    let Rejection {
        category,
        code,
        reason,
        path,
    } = &actual.0;
    assert_eq!(*category, expected.category.as_str(), "{case_id}: category");
    assert_eq!(*code, expected.code.as_deref(), "{case_id}: code");
    assert_eq!(reason, &expected.reason, "{case_id}: reason");
    assert_eq!(path, &expected.path, "{case_id}: path");
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../..")
        .canonicalize()
        .expect("repository root")
}

fn load_json<T: for<'de> Deserialize<'de>>(path: &Path) -> T {
    let bytes = fs::read(path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()));
    serde_json::from_slice(&bytes)
        .unwrap_or_else(|error| panic!("parse {}: {error}", path.display()))
}

fn hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}
