#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use fkst_qa_contracts::{
    contract_content_digest, validate_local_evidence_object,
    validate_local_evidence_object_ref, ValidatedValue,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_EVIDENCE_BYTES: usize = 1_048_576;
const SCHEMA_VERSION: &str = "qa.local-evidence/v1";
const OWNERSHIP: &str = "local-only:not-uploadable";
const REFERENCE_KIND: &str = "local-evidence-object";
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceRole {
    BrowserScreenshot,
    RunnerLog,
}

impl EvidenceRole {
    const fn as_str(self) -> &'static str {
        match self {
            Self::BrowserScreenshot => "browser-screenshot",
            Self::RunnerLog => "runner-log",
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EvidenceMediaType {
    Png,
    PlainTextUtf8,
}

impl EvidenceMediaType {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::PlainTextUtf8 => "text/plain; charset=utf-8",
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct StageRequest<'a> {
    pub run_id: &'a str,
    pub attempt: u64,
    pub object_id: &'a str,
    pub role: EvidenceRole,
    pub media_type: EvidenceMediaType,
    pub bytes: &'a [u8],
}

#[derive(Clone, Debug)]
pub struct StagedEvidence {
    object: ValidatedValue,
    object_ref: ValidatedValue,
}

impl StagedEvidence {
    pub fn object(&self) -> &ValidatedValue {
        &self.object
    }

    pub fn object_ref(&self) -> &ValidatedValue {
        &self.object_ref
    }
}

#[derive(Debug, Error, Eq, PartialEq)]
pub enum StagerError {
    #[error("evidence object exceeds the per-object byte limit")]
    ObjectTooLarge,
    #[error("evidence object contract validation failed")]
    InvalidObject,
    #[error("evidence object reference contract validation failed")]
    InvalidReference,
    #[error("evidence storage operation failed")]
    Storage,
    #[error("published evidence verification failed")]
    VerificationFailed,
}

#[derive(Clone, Debug)]
pub struct EvidenceStager {
    root: PathBuf,
}

impl EvidenceStager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn stage(&self, request: StageRequest<'_>) -> Result<StagedEvidence, StagerError> {
        if request.bytes.len() > MAX_EVIDENCE_BYTES {
            return Err(StagerError::ObjectTooLarge);
        }

        let raw_digest = raw_sha256(request.bytes);
        let object_json = json!({
            "schema_version": SCHEMA_VERSION,
            "run_id": request.run_id,
            "attempt": request.attempt,
            "object_id": request.object_id,
            "role": request.role.as_str(),
            "media_type": request.media_type.as_str(),
            "byte_length": request.bytes.len(),
            "sha256": raw_digest,
            "ownership": OWNERSHIP,
        });
        let object_bytes =
            serde_json::to_vec(&object_json).map_err(|_| StagerError::InvalidObject)?;
        let object = validate_local_evidence_object(&object_bytes)
            .map_err(|_| StagerError::InvalidObject)?;
        let object_contract_digest =
            contract_content_digest(&object).map_err(|_| StagerError::InvalidObject)?;

        let object_ref_json = json!({
            "kind": REFERENCE_KIND,
            "id": request.object_id,
            "schema_version": SCHEMA_VERSION,
            "content_digest": object_contract_digest,
        });
        let object_ref_bytes =
            serde_json::to_vec(&object_ref_json).map_err(|_| StagerError::InvalidReference)?;
        let object_ref = validate_local_evidence_object_ref(&object_ref_bytes)
            .map_err(|_| StagerError::InvalidReference)?;

        let final_path = self.object_path(request.run_id, request.attempt, request.object_id)?;
        let parent = final_path.parent().ok_or(StagerError::Storage)?;
        fs::create_dir_all(parent).map_err(|_| StagerError::Storage)?;

        let temporary_path = temporary_path(parent, &final_path)?;
        let mut temporary_file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary_path)
            .map_err(|_| StagerError::Storage)?;
        let mut temporary_guard = TemporaryFileGuard::new(temporary_path.clone());
        temporary_file
            .write_all(request.bytes)
            .map_err(|_| StagerError::Storage)?;
        temporary_file.flush().map_err(|_| StagerError::Storage)?;
        temporary_file.sync_all().map_err(|_| StagerError::Storage)?;
        drop(temporary_file);

        fs::rename(&temporary_path, &final_path).map_err(|_| StagerError::Storage)?;
        temporary_guard.disarm();
        sync_directory(parent)?;

        Ok(StagedEvidence { object, object_ref })
    }

    pub fn verify(&self, staged: &StagedEvidence) -> Result<(), StagerError> {
        let object = staged.object.value();
        let run_id = object
            .get("run_id")
            .and_then(serde_json::Value::as_str)
            .ok_or(StagerError::VerificationFailed)?;
        let attempt = object
            .get("attempt")
            .and_then(serde_json::Value::as_u64)
            .ok_or(StagerError::VerificationFailed)?;
        let object_id = object
            .get("object_id")
            .and_then(serde_json::Value::as_str)
            .ok_or(StagerError::VerificationFailed)?;
        let expected_length = object
            .get("byte_length")
            .and_then(serde_json::Value::as_u64)
            .ok_or(StagerError::VerificationFailed)?;
        let expected_digest = object
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .ok_or(StagerError::VerificationFailed)?;

        let final_path = self.object_path(run_id, attempt, object_id)?;
        let file = File::open(final_path).map_err(|_| StagerError::VerificationFailed)?;
        if !file
            .metadata()
            .map_err(|_| StagerError::VerificationFailed)?
            .is_file()
        {
            return Err(StagerError::VerificationFailed);
        }

        let mut stored_bytes = Vec::new();
        file.take((MAX_EVIDENCE_BYTES + 1) as u64)
            .read_to_end(&mut stored_bytes)
            .map_err(|_| StagerError::VerificationFailed)?;
        if stored_bytes.len() as u64 != expected_length
            || stored_bytes.len() > MAX_EVIDENCE_BYTES
            || raw_sha256(&stored_bytes) != expected_digest
        {
            return Err(StagerError::VerificationFailed);
        }

        Ok(())
    }

    fn object_path(
        &self,
        run_id: &str,
        attempt: u64,
        object_id: &str,
    ) -> Result<PathBuf, StagerError> {
        let object_number = object_id
            .strip_prefix("evidence/")
            .filter(|value| !value.is_empty() && value.bytes().all(|byte| byte.is_ascii_digit()))
            .ok_or(StagerError::InvalidObject)?;
        Ok(self
            .root
            .join(run_id)
            .join(attempt.to_string())
            .join("evidence")
            .join(format!("{object_number}.bin")))
    }
}

fn raw_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut output = String::with_capacity(64);
    for byte in digest {
        use std::fmt::Write as _;
        write!(&mut output, "{byte:02x}").expect("writing to a string cannot fail");
    }
    output
}

fn temporary_path(parent: &Path, final_path: &Path) -> Result<PathBuf, StagerError> {
    let final_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(StagerError::Storage)?;
    let sequence = TEMPORARY_FILE_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    Ok(parent.join(format!(
        ".{final_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    )))
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> Result<(), StagerError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| StagerError::Storage)
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), StagerError> {
    Ok(())
}

struct TemporaryFileGuard {
    path: PathBuf,
    armed: bool,
}

impl TemporaryFileGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for TemporaryFileGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}
