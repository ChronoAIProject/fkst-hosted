use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

use base64::Engine as _;
use fkst_qa_contracts::{
    admit_json, canonical_admitted_bytes, canonical_bytes, contract_content_digest,
    contract_content_projection, contract_registry, sha256_digest, validate_foundation,
    validate_scalar, verify_contract_content_digest, ContractError, FoundationType, Rejection,
};
use serde::Deserialize;
use serde_json::Value;

const GATE_ID: &str = "P0-02-CONTRACT-FOUNDATION";

#[derive(Deserialize)]
struct RfcFixture {
    schema_version: String,
    gate_id: String,
    valid_cases: Vec<RfcValidCase>,
    invalid_cases: Vec<RfcInvalidCase>,
}

#[derive(Deserialize)]
struct RfcValidCase {
    case_id: String,
    source_utf8_base64: String,
    expected_canonical_utf8_base64: String,
    expected_sha256: String,
}

#[derive(Deserialize)]
struct RfcInvalidCase {
    case_id: String,
    source_utf8_base64: String,
    expected: ExpectedRejection,
}

#[derive(Deserialize)]
struct FoundationFixture {
    schema_version: String,
    gate_id: String,
    valid_cases: Vec<FoundationValidCase>,
    invalid_cases: Vec<FoundationInvalidCase>,
    projection_cases: Vec<ProjectionCase>,
    digest_mismatch_cases: Vec<DigestMismatchCase>,
}

#[derive(Deserialize)]
struct FoundationValidCase {
    case_id: String,
    foundation_type: String,
    source: Value,
    expected_canonical_utf8_base64: String,
    expected_sha256: String,
}

#[derive(Deserialize)]
struct FoundationInvalidCase {
    case_id: String,
    foundation_type: String,
    source: Value,
    expected: ExpectedRejection,
}

#[derive(Deserialize)]
struct ProjectionCase {
    case_id: String,
    foundation_type: String,
    source: Value,
    expected_projection_utf8_base64: String,
    expected_sha256: String,
}

#[derive(Deserialize)]
struct DigestMismatchCase {
    case_id: String,
    foundation_type: String,
    source: Value,
    expected_projection_sha256: String,
    expected: ExpectedRejection,
}

#[derive(Deserialize)]
struct ExpectedRejection {
    category: String,
    code: Option<String>,
    reason: String,
    path: String,
}

#[test]
fn contract_registry_and_fixture_metadata() {
    let rfc_fixture = rfc_fixture();
    let foundation_fixture = foundation_fixture();
    assert_eq!(rfc_fixture.schema_version, "qa.rfc8785-fixtures/v1");
    assert_eq!(
        foundation_fixture.schema_version,
        "qa.contract-foundation-fixtures/v1"
    );
    assert_eq!(rfc_fixture.gate_id, GATE_ID);
    assert_eq!(foundation_fixture.gate_id, GATE_ID);

    let case_ids = fixture_case_ids(&rfc_fixture, &foundation_fixture);
    assert_eq!(case_ids.len(), 49, "shared fixture case count");
    assert_eq!(
        case_ids.iter().collect::<BTreeSet<_>>().len(),
        case_ids.len(),
        "fixture case IDs are globally unique"
    );

    let registry = contract_registry().expect("embedded registry is valid");
    assert_eq!(registry["registry_version"], "qa.contract-registry/v1");
    assert_eq!(registry["profile"], "local_qa_host_mvp");

    let schemas = registry["schemas"]
        .as_object()
        .expect("registry schemas object");
    assert_eq!(
        schemas.keys().map(String::as_str).collect::<Vec<_>>(),
        [
            "qa.contract-foundation/v1",
            "qa.local-lifecycle/v1",
            "qa.local-evidence/v1",
            "qa.local-worker-protocol/v1",
            "qa.local-run-admission/v1",
            "qa.local-run-admission/v2",
            "qa.local-executor/v1",
            "qa.local-cancellation/v1",
            "qa.local-worker-control/v1",
            "qa.local-executor-control/v1",
        ]
    );
    let schema = &schemas["qa.contract-foundation/v1"];
    assert_eq!(
        schema["path"],
        "contracts/qa.contract-foundation/v1/schema.json"
    );
    assert_eq!(
        schema["id"],
        "urn:chronoai:fkst:qa-contracts:qa.contract-foundation:v1"
    );
    assert_eq!(schema["major"], 1);

    let types = registry["types"]
        .as_object()
        .expect("registry types object");
    let expected_foundation_types: BTreeSet<_> = FoundationType::ALL
        .into_iter()
        .map(|foundation_type| foundation_type.definition().to_owned())
        .collect();
    let mut expected_types = expected_foundation_types.clone();
    expected_types.insert("CancelDisposition".to_owned());
    expected_types.insert("CleanupOutcome".to_owned());
    expected_types.insert("CleanupReceipt".to_owned());
    expected_types.insert("ControlStatus".to_owned());
    expected_types.insert("EffectDisposition".to_owned());
    expected_types.insert("EventCursor".to_owned());
    expected_types.insert("EventSequence".to_owned());
    expected_types.insert("ExecutionOutcome".to_owned());
    expected_types.insert("ExecutorControlReport".to_owned());
    expected_types.insert("ExecutorControlRequest".to_owned());
    expected_types.insert("ExecutorDescriptor".to_owned());
    expected_types.insert("ExecutorRequest".to_owned());
    expected_types.insert("ExecutorResult".to_owned());
    expected_types.insert("ExecutorSelection".to_owned());
    expected_types.insert("IndependentOutcome".to_owned());
    expected_types.insert("LocalEvidenceObject".to_owned());
    expected_types.insert("LocalEvidenceObjectRef".to_owned());
    expected_types.insert("LocalSanitizedObservation".to_owned());
    expected_types.insert("LocalSanitizedObservationRef".to_owned());
    expected_types.insert("LocalQARunRequest".to_owned());
    expected_types.insert("LocalQARunRequestV2".to_owned());
    expected_types.insert("LocalState".to_owned());
    expected_types.insert("LocalWorkerAbort".to_owned());
    expected_types.insert("LocalWorkerCancelAck".to_owned());
    expected_types.insert("LocalWorkerCapabilityRequest".to_owned());
    expected_types.insert("LocalWorkerCapabilityResult".to_owned());
    expected_types.insert("LocalWorkerControlFailure".to_owned());
    expected_types.insert("LocalWorkerControlFrame".to_owned());
    expected_types.insert("LocalWorkerFrame".to_owned());
    expected_types.insert("LocalWorkerInvocation".to_owned());
    expected_types.insert("LocalWorkerProtocolFailure".to_owned());
    expected_types.insert("LocalWorkerTerminalResult".to_owned());
    expected_types.insert("RunAcceptance".to_owned());
    expected_types.insert("RunAcceptanceV2".to_owned());
    expected_types.insert("SanitizedResidual".to_owned());
    assert_eq!(
        types.keys().cloned().collect::<BTreeSet<_>>(),
        expected_types
    );

    for foundation_type in FoundationType::ALL {
        let definition = foundation_type.definition();
        let entry = &types[definition];
        assert_eq!(
            entry["schema"], "qa.contract-foundation/v1",
            "{definition}: schema"
        );
        assert_eq!(
            entry["pointer"],
            format!("#/$defs/{definition}"),
            "{definition}: pointer"
        );
        let fixture_only = matches!(
            foundation_type,
            FoundationType::ProjectionSpecimen | FoundationType::StrictUnionSpecimen
        );
        assert_eq!(
            entry
                .get("fixture_only")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            fixture_only,
            "{definition}: fixture-only marker"
        );
    }

    let local_state = &types["LocalState"];
    assert_eq!(local_state["schema"], "qa.local-lifecycle/v1");
    assert_eq!(local_state["pointer"], "#/$defs/LocalState");
    assert!(local_state.get("fixture_only").is_none());

    let exercised_types = foundation_fixture_types(&foundation_fixture);
    assert_eq!(
        exercised_types, expected_foundation_types,
        "foundation fixture coverage matches foundation registry types"
    );

    let mut mutable_registry = registry;
    mutable_registry["profile"] = Value::String("mutated".into());
    assert_eq!(
        contract_registry().expect("embedded registry remains valid")["profile"],
        "local_qa_host_mvp"
    );
}

#[test]
fn rfc8785_shared_corpus() {
    let fixture = rfc_fixture();

    for case in fixture.valid_cases {
        report_case(&case.case_id);
        let raw = decode(&case.source_utf8_base64, &case.case_id);
        let admitted = admit_json(&raw).unwrap_or_else(|error| {
            panic!("{}: unexpected rejection: {:?}", case.case_id, error.0)
        });
        let canonical = canonical_admitted_bytes(&admitted).unwrap_or_else(|error| {
            panic!("{}: canonicalization failed: {:?}", case.case_id, error.0)
        });
        assert_eq!(
            encode(&canonical),
            case.expected_canonical_utf8_base64,
            "{}: canonical bytes",
            case.case_id
        );
        assert_eq!(
            sha256_digest(&canonical),
            case.expected_sha256,
            "{}: digest",
            case.case_id
        );
    }

    for case in fixture.invalid_cases {
        report_case(&case.case_id);
        let raw = decode(&case.source_utf8_base64, &case.case_id);
        let error = match admit_json(&raw) {
            Ok(_) => panic!("{}: expected rejection", case.case_id),
            Err(error) => error,
        };
        assert_rejection(&case.case_id, &error, &case.expected);
    }
}

#[test]
fn foundation_shared_corpus() {
    let fixture = foundation_fixture();

    for case in fixture.valid_cases {
        report_case(&case.case_id);
        let raw = serde_json::to_vec(&case.source).expect("fixture source serializes");
        let validated = validate_foundation(&raw, foundation_type(&case.foundation_type))
            .unwrap_or_else(|error| {
                panic!("{}: unexpected rejection: {:?}", case.case_id, error.0)
            });
        let canonical = canonical_bytes(&validated).unwrap_or_else(|error| {
            panic!("{}: canonicalization failed: {:?}", case.case_id, error.0)
        });
        assert_eq!(
            encode(&canonical),
            case.expected_canonical_utf8_base64,
            "{}: canonical bytes",
            case.case_id
        );
        assert_eq!(
            sha256_digest(&canonical),
            case.expected_sha256,
            "{}: digest",
            case.case_id
        );
    }

    for case in fixture.invalid_cases {
        report_case(&case.case_id);
        let raw = serde_json::to_vec(&case.source).expect("fixture source serializes");
        let error = match validate_foundation(&raw, foundation_type(&case.foundation_type)) {
            Ok(_) => panic!("{}: expected rejection", case.case_id),
            Err(error) => error,
        };
        assert_rejection(&case.case_id, &error, &case.expected);
    }

    for case in fixture.projection_cases {
        report_case(&case.case_id);
        let raw = serde_json::to_vec(&case.source).expect("fixture source serializes");
        let validated = validate_foundation(&raw, foundation_type(&case.foundation_type))
            .unwrap_or_else(|error| {
                panic!("{}: unexpected rejection: {:?}", case.case_id, error.0)
            });
        let projection = contract_content_projection(&validated)
            .unwrap_or_else(|error| panic!("{}: projection failed: {:?}", case.case_id, error.0));
        assert_eq!(
            encode(&projection),
            case.expected_projection_utf8_base64,
            "{}: projection bytes",
            case.case_id
        );
        assert_eq!(
            contract_content_digest(&validated).expect("projection digest"),
            case.expected_sha256,
            "{}: projection digest",
            case.case_id
        );
        verify_contract_content_digest(&validated).unwrap_or_else(|error| {
            panic!(
                "{}: digest verification failed: {:?}",
                case.case_id, error.0
            )
        });
    }

    for case in fixture.digest_mismatch_cases {
        report_case(&case.case_id);
        let raw = serde_json::to_vec(&case.source).expect("fixture source serializes");
        let validated = validate_foundation(&raw, foundation_type(&case.foundation_type))
            .unwrap_or_else(|error| {
                panic!("{}: unexpected rejection: {:?}", case.case_id, error.0)
            });
        assert_eq!(
            contract_content_digest(&validated).expect("projection digest"),
            case.expected_projection_sha256,
            "{}: projection digest",
            case.case_id
        );
        let error = match verify_contract_content_digest(&validated) {
            Ok(()) => panic!("{}: expected digest mismatch", case.case_id),
            Err(error) => error,
        };
        assert_rejection(&case.case_id, &error, &case.expected);
    }
}

#[test]
fn timestamp_boundaries_match_typescript() {
    for valid in [
        "2026-07-31T12:34:56Z",
        "2024-02-29T23:59:59.1Z",
        "0000-02-29T00:00:00Z",
    ] {
        validate_scalar("ISO8601", valid)
            .unwrap_or_else(|error| panic!("{valid}: expected valid timestamp, got {:?}", error.0));
    }

    for invalid in [
        "2026-02-31T12:34:56Z",
        "2023-02-29T00:00:00Z",
        "2026-07-31T23:59:60Z",
        "2026-07-31T24:00:00Z",
        "2026-07-31T12:34:56.0Z",
        "2026-07-31T12:34:56+00:00",
        "2026-07-31t12:34:56z",
    ] {
        assert!(
            validate_scalar("ISO8601", invalid).is_err(),
            "{invalid}: expected invalid timestamp"
        );
    }
}

#[test]
fn admission_precedence_and_order_match_typescript() {
    let tiny = admit_json(b"1e-9223372036854775808").expect("extreme negative exponent");
    assert_eq!(
        canonical_admitted_bytes(&tiny).expect("canonical tiny number"),
        b"0"
    );
    let escaped_pair = ["\"", "\\", "uD83D", "\\", "uDE00", "\""].concat();
    admit_json(escaped_pair.as_bytes()).expect("valid escaped surrogate pair");

    for (case_id, raw, expected) in [
        (
            "malformed-low-surrogate-is-invalid-json",
            br#""\uD800\uZZZZ""#.as_slice(),
            ExpectedRejection {
                category: "validation".into(),
                code: None,
                reason: "invalid_json".into(),
                path: "/".into(),
            },
        ),
        (
            "trailing-comma-precedes-duplicate-member",
            br#"{"a":1,"a":2,}"#.as_slice(),
            ExpectedRejection {
                category: "validation".into(),
                code: None,
                reason: "invalid_json".into(),
                path: "/".into(),
            },
        ),
        (
            "invalid-number-precedes-lone-surrogate",
            br#"{"x":"\uD800","n":NaN}"#.as_slice(),
            ExpectedRejection {
                category: "canonicalization".into(),
                code: Some("canonicalization.invalid_json_number".into()),
                reason: "invalid_json_number".into(),
                path: "/".into(),
            },
        ),
        (
            "duplicate-precedes-later-lone-surrogate",
            br#"{"a":1,"a":2,"x":"\uD800"}"#.as_slice(),
            ExpectedRejection {
                category: "canonicalization".into(),
                code: Some("canonicalization.duplicate_member".into()),
                reason: "duplicate_member".into(),
                path: "/".into(),
            },
        ),
        (
            "duplicate-key-precedes-invalid-second-value",
            br#"{"a":1,"a":"\uD800"}"#.as_slice(),
            ExpectedRejection {
                category: "canonicalization".into(),
                code: Some("canonicalization.duplicate_member".into()),
                reason: "duplicate_member".into(),
                path: "/".into(),
            },
        ),
        (
            "lone-surrogate-precedes-later-duplicate",
            br#"{"x":"\uD800","a":1,"a":2}"#.as_slice(),
            ExpectedRejection {
                category: "canonicalization".into(),
                code: Some("canonicalization.invalid_unicode_scalar".into()),
                reason: "invalid_unicode_scalar".into(),
                path: "/".into(),
            },
        ),
    ] {
        let error = match admit_json(raw) {
            Ok(_) => panic!("{case_id}: expected rejection"),
            Err(error) => error,
        };
        assert_rejection(case_id, &error, &expected);
    }

    let error = match validate_foundation(br#"{"z":1,"a":2}"#, FoundationType::ContractMeta) {
        Ok(_) => panic!("unknown fields should be rejected"),
        Err(error) => error,
    };
    assert_rejection(
        "unknown-field-source-order",
        &error,
        &ExpectedRejection {
            category: "contract".into(),
            code: Some("contract.forbidden_field".into()),
            reason: "unknown_field".into(),
            path: "/z".into(),
        },
    );
}

#[test]
fn admitted_and_validated_values_are_opaque_snapshots() {
    let admitted = admit_json(br#"{"nested":{"value":1}}"#).expect("admitted fixture");
    let admitted_canonical = canonical_admitted_bytes(&admitted).expect("canonical admitted value");
    let mut admitted_clone = admitted.value().clone();
    admitted_clone["nested"]["value"] = Value::from(2);
    assert_eq!(
        canonical_admitted_bytes(&admitted).expect("canonical admitted value"),
        admitted_canonical
    );

    let projection_case = foundation_fixture()
        .projection_cases
        .into_iter()
        .next()
        .expect("projection fixture");
    let raw = serde_json::to_vec(&projection_case.source).expect("fixture source serializes");
    let validated = validate_foundation(&raw, foundation_type(&projection_case.foundation_type))
        .expect("validated fixture");
    let digest = contract_content_digest(&validated).expect("projection digest");
    let mut validated_clone = validated.value().clone();
    validated_clone["payload"]["a"] = Value::from(99);
    assert_eq!(
        contract_content_digest(&validated).expect("projection digest"),
        digest
    );
}

fn rfc_fixture() -> RfcFixture {
    load_json(&repo_root().join("fixtures/rfc8785-v1.json"))
}

fn foundation_fixture() -> FoundationFixture {
    load_json(&repo_root().join("fixtures/qa/contract-foundation-v1.json"))
}

fn fixture_case_ids<'a>(rfc: &'a RfcFixture, foundation: &'a FoundationFixture) -> Vec<&'a str> {
    rfc.valid_cases
        .iter()
        .map(|case| case.case_id.as_str())
        .chain(rfc.invalid_cases.iter().map(|case| case.case_id.as_str()))
        .chain(
            foundation
                .valid_cases
                .iter()
                .map(|case| case.case_id.as_str()),
        )
        .chain(
            foundation
                .invalid_cases
                .iter()
                .map(|case| case.case_id.as_str()),
        )
        .chain(
            foundation
                .projection_cases
                .iter()
                .map(|case| case.case_id.as_str()),
        )
        .chain(
            foundation
                .digest_mismatch_cases
                .iter()
                .map(|case| case.case_id.as_str()),
        )
        .collect()
}

fn foundation_fixture_types(fixture: &FoundationFixture) -> BTreeSet<String> {
    fixture
        .valid_cases
        .iter()
        .map(|case| case.foundation_type.clone())
        .chain(
            fixture
                .invalid_cases
                .iter()
                .map(|case| case.foundation_type.clone()),
        )
        .chain(
            fixture
                .projection_cases
                .iter()
                .map(|case| case.foundation_type.clone()),
        )
        .chain(
            fixture
                .digest_mismatch_cases
                .iter()
                .map(|case| case.foundation_type.clone()),
        )
        .collect()
}

fn report_case(case_id: &str) {
    println!("case_id={case_id}");
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

fn decode(encoded: &str, case_id: &str) -> Vec<u8> {
    base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .unwrap_or_else(|error| panic!("{case_id}: invalid fixture base64: {error}"))
}

fn encode(bytes: &[u8]) -> String {
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

fn foundation_type(name: &str) -> FoundationType {
    match name {
        "ContractMeta" => FoundationType::ContractMeta,
        "HostScopedMeta" => FoundationType::HostScopedMeta,
        "ResourceRef" => FoundationType::ResourceRef,
        "ActorRef" => FoundationType::ActorRef,
        "DigestBoundRef" => FoundationType::DigestBoundRef,
        "SignatureBlock" => FoundationType::SignatureBlock,
        "ProjectionSpecimen" => FoundationType::ProjectionSpecimen,
        "StrictUnionSpecimen" => FoundationType::StrictUnionSpecimen,
        other => panic!("unknown foundation fixture type: {other}"),
    }
}

fn assert_rejection(case_id: &str, actual: &ContractError, expected: &ExpectedRejection) {
    let Rejection {
        category,
        code,
        reason,
        path,
    } = &actual.0;
    assert_eq!(*category, expected.category, "{case_id}: category");
    assert_eq!(code.map(str::to_owned), expected.code, "{case_id}: code");
    assert_eq!(*reason, expected.reason, "{case_id}: reason");
    assert_eq!(*path, expected.path, "{case_id}: path");
}
