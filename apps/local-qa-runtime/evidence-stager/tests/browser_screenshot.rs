use std::fs;
use std::path::{Path, PathBuf};

use fkst_local_qa_evidence_stager::{
    EvidenceMediaType, EvidenceRole, EvidenceStager, StageRequest, StagerError,
};
use fkst_qa_contracts::{
    canonical_bytes, contract_content_digest, validate_local_evidence_object,
    validate_local_evidence_object_ref,
};
use serde_json::json;
use tempfile::TempDir;

const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x04, 0x00, 0x00, 0x00, 0xb5, 0x1c, 0x0c,
    0x02, 0x00, 0x00, 0x00, 0x0b, 0x49, 0x44, 0x41, 0x54, 0x78, 0xda, 0x63, 0x64, 0xf8, 0x0f, 0x00,
    0x01, 0x05, 0x01, 0x01, 0x27, 0x18, 0xe3, 0x66, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45, 0x4e, 0x44,
    0xae, 0x42, 0x60, 0x82,
];
const RAW_DIGEST: &str = "431ced6916a2a21a156e38701afe55bbd7f88969fbbfc56d7fe099d47f265460";
const CONTRACT_DIGEST: &str =
    "sha256:4423332d8ae5ce26e0adaeab085e85441f3686ed0cdc0e367631aef442e4ec4d";
const CANONICAL_OBJECT: &str = r#"{"attempt":1,"byte_length":68,"media_type":"image/png","object_id":"evidence/1","ownership":"local-only:not-uploadable","role":"browser-screenshot","run_id":"run-1","schema_version":"qa.local-evidence/v1","sha256":"431ced6916a2a21a156e38701afe55bbd7f88969fbbfc56d7fe099d47f265460"}"#;

#[test]
fn walks_one_png_through_staging_verification_and_cleanup() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);

    let staged = stager
        .stage(StageRequest {
            run_id: "run-1",
            attempt: 1,
            object_id: "evidence/1",
            role: EvidenceRole::BrowserScreenshot,
            media_type: EvidenceMediaType::Png,
            bytes: PNG,
        })
        .expect("stage accepted browser screenshot");

    let expected_object = json!({
        "schema_version": "qa.local-evidence/v1",
        "run_id": "run-1",
        "attempt": 1,
        "object_id": "evidence/1",
        "role": "browser-screenshot",
        "media_type": "image/png",
        "byte_length": 68,
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

    let files = regular_files(&root);
    assert_eq!(files.len(), 1);
    assert_eq!(fs::read(&files[0]).unwrap(), PNG);
    assert_no_temporary_files(&root);
    stager.verify(&staged).expect("verify unchanged screenshot");

    let cleanup = stager
        .cleanup_attempt("run-1", 1)
        .expect("clean staged attempt");
    assert!(cleanup.is_complete());
    assert!(root.exists());
    assert!(!root.join("run-1").join("1").exists());
    assert!(regular_files(&root).is_empty());
    assert_no_temporary_files(&root);
    assert!(stager
        .cleanup_attempt("run-1", 1)
        .expect("repeat completed cleanup")
        .is_complete());

    let verification_error = stager.verify(&staged).unwrap_err();
    assert_eq!(verification_error, StagerError::VerificationFailed);
    assert_sanitized_error(&verification_error, &root);
}

#[test]
fn cleanup_validates_scope_before_mutating_storage() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let owned_attempt = root.join("run-1").join("1");
    fs::create_dir_all(&owned_attempt).unwrap();
    fs::write(owned_attempt.join("sentinel"), b"preserve").unwrap();
    let stager = EvidenceStager::new(&root);

    let invalid_run_error = stager.cleanup_attempt("../run-1", 1).unwrap_err();
    assert_eq!(invalid_run_error, StagerError::InvalidCleanupScope);
    assert_eq!(
        fs::read(owned_attempt.join("sentinel")).unwrap(),
        b"preserve"
    );
    assert_sanitized_error(&invalid_run_error, &root);

    let invalid_attempt_error = stager.cleanup_attempt("run-1", 0).unwrap_err();
    assert_eq!(invalid_attempt_error, StagerError::InvalidCleanupScope);
    assert_eq!(
        fs::read(owned_attempt.join("sentinel")).unwrap(),
        b"preserve"
    );
    assert_sanitized_error(&invalid_attempt_error, &root);
}

#[test]
fn staging_does_not_overwrite_an_existing_object() {
    let temporary_parent = TempDir::new().expect("create test-owned temporary parent");
    let root = temporary_parent.path().join("quarantine");
    let stager = EvidenceStager::new(&root);
    let request = StageRequest {
        run_id: "run-1",
        attempt: 1,
        object_id: "evidence/1",
        role: EvidenceRole::BrowserScreenshot,
        media_type: EvidenceMediaType::Png,
        bytes: PNG,
    };
    stager.stage(request).expect("stage initial screenshot");
    let published = regular_files(&root).pop().unwrap();

    let error = stager.stage(request).unwrap_err();

    assert_eq!(error, StagerError::DuplicateIdentity);
    assert_eq!(fs::read(published).unwrap(), PNG);
    assert_no_temporary_files(&root);
    assert_sanitized_error(&error, &root);
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

fn assert_no_temporary_files(root: &Path) {
    assert!(regular_files(root).iter().all(|path| {
        !path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}

fn assert_sanitized_error(error: &StagerError, root: &Path) {
    let message = error.to_string();
    assert!(!message.contains(&root.display().to_string()));
    assert!(!message.contains(".tmp"));
    assert!(!message.contains("No such file"));
    assert!(!message.contains("Permission denied"));
    assert!(!message.contains("PNG"));
    assert!(!message.contains("iVBOR"));
}
