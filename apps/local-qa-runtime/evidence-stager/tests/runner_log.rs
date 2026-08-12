use std::fs;
use std::path::{Path, PathBuf};

use fkst_local_qa_evidence_stager::{
    EvidenceMediaType, EvidenceRole, EvidenceStager, StageRequest, StagerError, MAX_EVIDENCE_BYTES,
};
use fkst_qa_contracts::{
    canonical_bytes, contract_content_digest, validate_local_evidence_object,
    validate_local_evidence_object_ref,
};
use serde_json::json;
use tempfile::TempDir;

const RUNNER_LOG: &[u8] = b"navigation accepted\nassertion passed\n";
const RAW_DIGEST: &str = "bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e";
const CONTRACT_DIGEST: &str =
    "sha256:bdcf45ce2e53d380d8773efca6522a4d17d4b5c9aa923066ca43ed64f110ec88";
const CANONICAL_OBJECT: &str = concat!(
    r#"{"attempt":1,"byte_length":37,"media_type":"text/plain; charset=utf-8","#,
    r#""object_id":"evidence/1","ownership":"local-only:not-uploadable","#,
    r#""role":"runner-log","run_id":"run-1","schema_version":"qa.local-evidence/v1","#,
    r#""sha256":"bb9c62cc84fc533e52193a8961778b0be251cd8f19a89b3fa836e94043a0075e"}"#,
);

#[test]
fn stages_and_verifies_one_runner_log() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);

    let staged = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/1",
            role: EvidenceRole::RunnerLog,
            media_type: EvidenceMediaType::PlainTextUtf8,
            bytes: RUNNER_LOG,
        })
        .expect("stage accepted runner log");

    let expected_object = json!({
        "schema_version": "qa.local-evidence/v1",
        "run_id": "run-1",
        "attempt": 1,
        "object_id": "evidence/1",
        "role": "runner-log",
        "media_type": "text/plain; charset=utf-8",
        "byte_length": 37,
        "sha256": RAW_DIGEST,
        "ownership": "local-only:not-uploadable",
    });
    assert_eq!(staged.object().value(), &expected_object);
    validate_local_evidence_object(&serde_json::to_vec(staged.object().value()).unwrap()).unwrap();
    assert_eq!(
        canonical_bytes(staged.object()).unwrap(),
        CANONICAL_OBJECT.as_bytes()
    );
    assert_eq!(
        contract_content_digest(staged.object()).unwrap(),
        CONTRACT_DIGEST
    );

    let expected_ref = json!({
        "kind": "local-evidence-object",
        "id": "evidence/1",
        "schema_version": "qa.local-evidence/v1",
        "content_digest": CONTRACT_DIGEST,
    });
    assert_eq!(staged.object_ref().value(), &expected_ref);
    validate_local_evidence_object_ref(&serde_json::to_vec(staged.object_ref().value()).unwrap())
        .unwrap();

    let files = regular_files(temporary_parent.path());
    assert_eq!(files.len(), 1);
    assert!(files[0].starts_with(&root));
    assert_eq!(fs::read(&files[0]).unwrap(), RUNNER_LOG);
    assert!(!files[0]
        .file_name()
        .unwrap()
        .to_string_lossy()
        .ends_with(".tmp"));

    stager.verify(&staged).expect("verify unchanged runner log");
}

#[test]
fn rejects_oversized_evidence_before_creating_storage() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);
    let oversized = vec![b'x'; MAX_EVIDENCE_BYTES + 1];

    let error = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/1",
            role: EvidenceRole::RunnerLog,
            media_type: EvidenceMediaType::PlainTextUtf8,
            bytes: &oversized,
        })
        .unwrap_err();

    assert_eq!(error, StagerError::ObjectTooLarge);
    assert!(!root.exists());
}

#[test]
fn rejects_an_invalid_role_media_pair_before_creating_storage() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);

    let error = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/1",
            role: EvidenceRole::RunnerLog,
            media_type: EvidenceMediaType::Png,
            bytes: RUNNER_LOG,
        })
        .unwrap_err();

    assert_eq!(error, StagerError::InvalidObject);
    assert!(!root.exists());
}

#[test]
fn verifier_reads_the_published_file() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);
    let staged = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/1",
            role: EvidenceRole::RunnerLog,
            media_type: EvidenceMediaType::PlainTextUtf8,
            bytes: RUNNER_LOG,
        })
        .unwrap();
    let file = regular_files(&root).pop().unwrap();
    fs::write(file, b"tampered\n").unwrap();

    assert_eq!(stager.verify(&staged), Err(StagerError::VerificationFailed));
}

fn regular_files(root: &Path) -> Vec<PathBuf> {
    let mut pending = vec![root.to_path_buf()];
    let mut files = Vec::new();
    while let Some(path) = pending.pop() {
        for entry in fs::read_dir(path).unwrap() {
            let entry = entry.unwrap();
            let file_type = entry.file_type().unwrap();
            if file_type.is_dir() {
                pending.push(entry.path());
            } else if file_type.is_file() {
                files.push(entry.path());
            }
        }
    }
    files.sort();
    files
}
