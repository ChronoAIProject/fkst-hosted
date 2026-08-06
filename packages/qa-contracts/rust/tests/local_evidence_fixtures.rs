use std::fs;
use std::path::{Path, PathBuf};

use fkst_qa_contracts::{
    canonical_bytes, contract_content_digest, contract_registry, sha256_digest,
    validate_local_evidence_object, validate_local_evidence_object_ref,
    validate_local_sanitized_observation, validate_local_sanitized_observation_ref, ContractError,
    ValidatedValue,
};
use serde::Deserialize;
use serde_json::Value;

#[derive(Deserialize)]
struct LocalEvidenceFixture {
    schema_version: String,
    raw_evidence_cases: Vec<RawEvidenceCase>,
    accepted_cases: Vec<EvidenceCase>,
    rejected_binding_cases: Vec<EvidenceCase>,
    raw_digest_non_binding_cases: Vec<RawDigestNonBindingCase>,
    unknown_type_case: UnknownTypeCase,
}

#[derive(Deserialize)]
struct RawEvidenceCase {
    case_id: String,
    utf8: String,
    expected_byte_length: usize,
    expected_sha256: String,
}

#[derive(Deserialize)]
struct EvidenceCase {
    case_id: String,
    evidence_type: EvidenceType,
    source: Value,
    expected_canonical_utf8: Option<String>,
    expected_content_digest: Option<String>,
    raw_evidence_case_id: Option<String>,
    referenced_case_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
enum EvidenceType {
    #[serde(rename = "LocalSanitizedObservation")]
    Observation,
    #[serde(rename = "LocalEvidenceObject")]
    EvidenceObject,
    #[serde(rename = "LocalSanitizedObservationRef")]
    ObservationRef,
    #[serde(rename = "LocalEvidenceObjectRef")]
    EvidenceObjectRef,
}

#[derive(Deserialize)]
struct RawDigestNonBindingCase {
    case_id: String,
    raw_evidence_case_id: String,
    referenced_case_id: String,
    digest_form: DigestForm,
}

#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
enum DigestForm {
    Unprefixed,
    Prefixed,
}

#[derive(Deserialize)]
struct UnknownTypeCase {
    case_id: String,
    evidence_type: String,
    source: Value,
}

#[test]
fn local_evidence_fixture_walks_the_production_path() {
    let fixture: LocalEvidenceFixture =
        load_json(&repo_root().join("fixtures/qa/local-evidence-v1.json"));
    assert_eq!(fixture.schema_version, "qa.local-evidence-fixtures/v1");
    assert_eq!(
        fixture
            .accepted_cases
            .iter()
            .map(|fixture_case| (fixture_case.case_id.as_str(), fixture_case.evidence_type))
            .collect::<Vec<_>>(),
        vec![
            ("local-observation-accepted", EvidenceType::Observation,),
            (
                "local-runner-log-object-accepted",
                EvidenceType::EvidenceObject,
            ),
            (
                "local-observation-ref-accepted",
                EvidenceType::ObservationRef,
            ),
            (
                "local-runner-log-ref-accepted",
                EvidenceType::EvidenceObjectRef,
            ),
        ]
    );

    let registry = contract_registry().expect("load contract registry");
    for fixture_case in &fixture.accepted_cases {
        let type_name = evidence_type_name(fixture_case.evidence_type);
        let expected_pointer = format!("#/$defs/{type_name}");
        assert_eq!(
            registry
                .pointer(&format!("/types/{type_name}/schema"))
                .and_then(Value::as_str),
            Some("qa.local-evidence/v1")
        );
        assert_eq!(
            registry
                .pointer(&format!("/types/{type_name}/pointer"))
                .and_then(Value::as_str),
            Some(expected_pointer.as_str())
        );
    }

    for raw_case in &fixture.raw_evidence_cases {
        println!("case_id={}", raw_case.case_id);
        let raw_bytes = raw_case.utf8.as_bytes();
        assert_eq!(raw_bytes.len(), raw_case.expected_byte_length);
        assert_eq!(unprefixed_sha256(raw_bytes), raw_case.expected_sha256);
    }

    for fixture_case in &fixture.accepted_cases {
        println!("case_id={}", fixture_case.case_id);
        let validated = validate_fixture_case(fixture_case).expect("validate evidence fixture");
        assert_eq!(validated.value(), &fixture_case.source);
        let canonical = canonical_bytes(&validated).expect("canonical evidence bytes");

        if let Some(expected_canonical) = &fixture_case.expected_canonical_utf8 {
            assert_eq!(
                std::str::from_utf8(&canonical).expect("canonical bytes are UTF-8"),
                expected_canonical
            );
            assert_eq!(
                contract_content_digest(&validated).expect("digest evidence contract"),
                fixture_case
                    .expected_content_digest
                    .as_deref()
                    .expect("canonical vector has expected digest")
            );
            assert_eq!(
                sha256_digest(&canonical),
                fixture_case
                    .expected_content_digest
                    .as_deref()
                    .expect("canonical vector has expected digest")
            );
        }

        if let Some(raw_case_id) = &fixture_case.raw_evidence_case_id {
            let raw_case = find_raw_case(&fixture, raw_case_id);
            assert_eq!(
                fixture_case
                    .source
                    .pointer("/byte_length")
                    .and_then(Value::as_u64),
                Some(raw_case.expected_byte_length as u64)
            );
            assert_eq!(
                fixture_case
                    .source
                    .pointer("/sha256")
                    .and_then(Value::as_str),
                Some(raw_case.expected_sha256.as_str())
            );
        }

        if let Some(referenced_case_id) = &fixture_case.referenced_case_id {
            assert_reference_binds(&fixture, &validated, referenced_case_id);
        }
    }

    for fixture_case in &fixture.rejected_binding_cases {
        println!("case_id={}", fixture_case.case_id);
        let validated_reference =
            validate_fixture_case(fixture_case).expect("validate mismatched reference fixture");
        canonical_bytes(&validated_reference).expect("canonical mismatched reference bytes");
        let referenced_digest = digest_accepted_case(
            &fixture,
            fixture_case
                .referenced_case_id
                .as_deref()
                .expect("binding case target"),
        );
        assert_ne!(
            validated_reference
                .value()
                .pointer("/content_digest")
                .and_then(Value::as_str),
            Some(referenced_digest.as_str())
        );
    }

    for fixture_case in &fixture.raw_digest_non_binding_cases {
        println!("case_id={}", fixture_case.case_id);
        let raw_digest =
            &find_raw_case(&fixture, &fixture_case.raw_evidence_case_id).expected_sha256;
        let candidate = match fixture_case.digest_form {
            DigestForm::Unprefixed => raw_digest.clone(),
            DigestForm::Prefixed => format!("sha256:{raw_digest}"),
        };
        assert_ne!(
            candidate,
            digest_accepted_case(&fixture, &fixture_case.referenced_case_id)
        );
    }

    println!("case_id={}", fixture.unknown_type_case.case_id);
    assert!(fixture.unknown_type_case.source.is_object());
    serde_json::from_value::<EvidenceType>(Value::String(
        fixture.unknown_type_case.evidence_type.clone(),
    ))
    .expect_err("reject unknown evidence fixture type");
}

fn validate_fixture_case(fixture_case: &EvidenceCase) -> Result<ValidatedValue, ContractError> {
    let raw = serde_json::to_vec(&fixture_case.source).expect("serialize evidence fixture source");
    match fixture_case.evidence_type {
        EvidenceType::Observation => validate_local_sanitized_observation(&raw),
        EvidenceType::EvidenceObject => validate_local_evidence_object(&raw),
        EvidenceType::ObservationRef => validate_local_sanitized_observation_ref(&raw),
        EvidenceType::EvidenceObjectRef => validate_local_evidence_object_ref(&raw),
    }
}

fn assert_reference_binds(
    fixture: &LocalEvidenceFixture,
    reference: &ValidatedValue,
    referenced_case_id: &str,
) {
    let referenced_digest = digest_accepted_case(fixture, referenced_case_id);
    assert_eq!(
        reference
            .value()
            .pointer("/content_digest")
            .and_then(Value::as_str),
        Some(referenced_digest.as_str())
    );
}

fn digest_accepted_case(fixture: &LocalEvidenceFixture, case_id: &str) -> String {
    let target = find_accepted_case(fixture, case_id);
    let validated = validate_fixture_case(target).expect("validate referenced evidence fixture");
    contract_content_digest(&validated).expect("digest referenced evidence fixture")
}

fn find_accepted_case<'a>(fixture: &'a LocalEvidenceFixture, case_id: &str) -> &'a EvidenceCase {
    fixture
        .accepted_cases
        .iter()
        .find(|fixture_case| fixture_case.case_id == case_id)
        .unwrap_or_else(|| panic!("unknown accepted case: {case_id}"))
}

fn find_raw_case<'a>(fixture: &'a LocalEvidenceFixture, case_id: &str) -> &'a RawEvidenceCase {
    fixture
        .raw_evidence_cases
        .iter()
        .find(|raw_case| raw_case.case_id == case_id)
        .unwrap_or_else(|| panic!("unknown raw evidence case: {case_id}"))
}

fn evidence_type_name(evidence_type: EvidenceType) -> &'static str {
    match evidence_type {
        EvidenceType::Observation => "LocalSanitizedObservation",
        EvidenceType::EvidenceObject => "LocalEvidenceObject",
        EvidenceType::ObservationRef => "LocalSanitizedObservationRef",
        EvidenceType::EvidenceObjectRef => "LocalEvidenceObjectRef",
    }
}

fn unprefixed_sha256(bytes: &[u8]) -> String {
    sha256_digest(bytes)
        .strip_prefix("sha256:")
        .expect("digest prefix")
        .to_owned()
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
