use std::fs;

use fkst_local_qa_evidence_stager::{
    EvidenceMediaType, EvidenceRole, EvidenceStager, StageRequest,
    StageSanitizedObservationRequest, StagerError,
};
use fkst_qa_contracts::{
    contract_content_digest, validate_local_sanitized_observation,
    validate_local_sanitized_observation_ref,
};
use tempfile::TempDir;

const RUNNER_LOG: &[u8] = b"navigation accepted\nassertion passed\n";
const FIXTURE_URL: &str = "http://127.0.0.1:49152/fixed-page.html";

fn request<'a>() -> StageSanitizedObservationRequest<'a> {
    StageSanitizedObservationRequest {
        run_id: "run-1",
        attempt: 1,
        observation_id: "observation/0",
        fixture_url: FIXTURE_URL,
        final_url: FIXTURE_URL,
        selector: r#"[data-local-qa="status"]"#,
        expected_text: "READY",
        observed_text: "READY",
    }
}

#[test]
fn stages_reloads_verifies_and_cleans_canonical_observation() {
    let temporary = TempDir::new().expect("temporary staging parent");
    let root = temporary.path().join("quarantine");
    let stager = EvidenceStager::new(&root);

    let staged = stager
        .stage_sanitized_observation(request())
        .expect("observation stages");
    validate_local_sanitized_observation(staged.canonical_bytes())
        .expect("stored bytes validate");
    validate_local_sanitized_observation_ref(
        &serde_json::to_vec(staged.observation_ref().value()).expect("reference encodes"),
    )
    .expect("reference validates");
    assert_eq!(
        staged
            .observation_ref()
            .value()
            .get("content_digest")
            .and_then(serde_json::Value::as_str),
        Some(
            contract_content_digest(staged.observation())
                .expect("observation digest")
                .as_str()
        )
    );
    stager
        .verify_sanitized_observation(&staged)
        .expect("staged observation verifies");

    let reopened = EvidenceStager::new(&root)
        .load_sanitized_observation("run-1", 1, "observation/0")
        .expect("observation reloads through a new owner");
    assert_eq!(reopened.canonical_bytes(), staged.canonical_bytes());
    assert_eq!(
        reopened.observation_ref().value(),
        staged.observation_ref().value()
    );

    let path = root.join("run-1/1/observation/0.json");
    assert_eq!(fs::read(&path).expect("observation bytes read"), staged.canonical_bytes());
    assert_eq!(
        stager.stage_sanitized_observation(request()).unwrap_err(),
        StagerError::DuplicateIdentity
    );

    assert!(stager
        .cleanup_attempt("run-1", 1)
        .expect("attempt cleanup")
        .is_complete());
    assert!(!root.join("run-1/1").exists());
}

#[test]
fn observation_is_separate_from_the_two_evidence_object_quota() {
    let temporary = TempDir::new().expect("temporary staging parent");
    let root = temporary.path().join("quarantine");
    let stager = EvidenceStager::new(&root);
    stager
        .stage_sanitized_observation(request())
        .expect("observation stages");

    for object_id in ["evidence/0", "evidence/1"] {
        stager
            .stage(StageRequest {
                run_id: "run-1",
                attempt: 1,
                object_id,
                role: EvidenceRole::RunnerLog,
                media_type: EvidenceMediaType::PlainTextUtf8,
                bytes: RUNNER_LOG,
            })
            .expect("two Evidence objects remain available");
    }
    assert_eq!(
        stager
            .stage(StageRequest {
                run_id: "run-1",
                attempt: 1,
                object_id: "evidence/2",
                role: EvidenceRole::RunnerLog,
                media_type: EvidenceMediaType::PlainTextUtf8,
                bytes: RUNNER_LOG,
            })
            .unwrap_err(),
        StagerError::QuotaExceeded
    );
}

#[test]
fn tampered_or_unsafe_observation_fails_closed() {
    let temporary = TempDir::new().expect("temporary staging parent");
    let root = temporary.path().join("quarantine");
    let stager = EvidenceStager::new(&root);
    let staged = stager
        .stage_sanitized_observation(request())
        .expect("observation stages");
    fs::write(root.join("run-1/1/observation/0.json"), b"{}")
        .expect("tamper observation");
    assert_eq!(
        stager.verify_sanitized_observation(&staged),
        Err(StagerError::VerificationFailed)
    );

    let unsafe_root = temporary.path().join("unsafe");
    let outside = temporary.path().join("outside");
    fs::create_dir_all(&outside).expect("outside directory");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&outside, &unsafe_root).expect("symlink staging root");
        assert_eq!(
            EvidenceStager::new(&unsafe_root)
                .stage_sanitized_observation(request())
                .unwrap_err(),
            StagerError::FilesystemSafety
        );
    }
}
