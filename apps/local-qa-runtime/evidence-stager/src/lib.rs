#![forbid(unsafe_code)]

use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use fkst_qa_contracts::{
    contract_content_digest, validate_local_evidence_object, validate_local_evidence_object_ref,
    ValidatedValue,
};
use serde_json::json;
use sha2::{Digest, Sha256};
use thiserror::Error;

pub const MAX_EVIDENCE_BYTES: usize = 1_048_576;
const MAX_ATTEMPT_BYTES: u64 = 2_097_152;
const MAX_OBJECTS_PER_ATTEMPT: usize = 2;
const MAX_SAFE_ATTEMPT: u64 = 9_007_199_254_740_991;
const SCHEMA_VERSION: &str = "qa.local-evidence/v1";
const OWNERSHIP: &str = "local-only:not-uploadable";
const REFERENCE_KIND: &str = "local-evidence-object";
static TEMPORARY_FILE_SEQUENCE: AtomicU64 = AtomicU64::new(1);
static STAGING_COORDINATION: Mutex<()> = Mutex::new(());

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CleanupResidual {
    run_id: String,
    attempt: u64,
    reason: CleanupResidualReason,
}

impl CleanupResidual {
    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub const fn attempt(&self) -> u64 {
        self.attempt
    }

    pub const fn reason(&self) -> CleanupResidualReason {
        self.reason
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CleanupResidualReason {
    UnrelatedEntry,
    UnsafeEntry,
    RemovalFailed,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[must_use]
pub struct CleanupResult {
    complete: bool,
    residuals: Vec<CleanupResidual>,
}

impl CleanupResult {
    pub const fn is_complete(&self) -> bool {
        self.complete
    }

    pub fn residuals(&self) -> &[CleanupResidual] {
        &self.residuals
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
    #[error("evidence identifier validation failed")]
    InvalidIdentifier,
    #[error("evidence attempt quota exceeded")]
    QuotaExceeded,
    #[error("evidence object identity already exists")]
    DuplicateIdentity,
    #[error("evidence filesystem safety check failed")]
    FilesystemSafety,
    #[error("evidence storage operation failed")]
    Storage,
    #[error("published evidence verification failed")]
    VerificationFailed,
    #[error("evidence cleanup scope validation failed")]
    InvalidCleanupScope,
    #[error("evidence cleanup operation failed")]
    Cleanup,
}

#[derive(Clone, Debug)]
pub struct EvidenceStager {
    root: PathBuf,
    coordination: Arc<Mutex<()>>,
}

impl EvidenceStager {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            coordination: Arc::new(Mutex::new(())),
        }
    }

    pub fn stage(&self, request: StageRequest<'_>) -> Result<StagedEvidence, StagerError> {
        validate_request(&request)?;
        let object = build_object(&request)?;
        let object_contract_digest =
            contract_content_digest(&object).map_err(|_| StagerError::InvalidObject)?;
        let object_ref = build_reference(request.object_id, object_contract_digest)?;

        let _global_guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::Storage)?;
        let _guard = self.coordination.lock().map_err(|_| StagerError::Storage)?;
        let final_path = self.object_path(request.run_id, request.attempt, request.object_id)?;
        let parent = final_path.parent().ok_or(StagerError::Storage)?;
        ensure_directory_path(&self.root, parent)?;
        let quota = inspect_attempt(parent)?;
        match fs::symlink_metadata(&final_path) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() {
                    return Err(StagerError::FilesystemSafety);
                }
                if metadata.file_type().is_file() {
                    return Err(StagerError::DuplicateIdentity);
                }
                return Err(StagerError::FilesystemSafety);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StagerError::Storage),
        }
        if quota.count >= MAX_OBJECTS_PER_ATTEMPT
            || quota.bytes.saturating_add(request.bytes.len() as u64) > MAX_ATTEMPT_BYTES
        {
            return Err(StagerError::QuotaExceeded);
        }

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
        temporary_file
            .sync_all()
            .map_err(|_| StagerError::Storage)?;
        drop(temporary_file);
        fs::hard_link(&temporary_path, &final_path).map_err(|error| {
            if error.kind() == std::io::ErrorKind::AlreadyExists {
                StagerError::DuplicateIdentity
            } else {
                StagerError::Storage
            }
        })?;
        fs::remove_file(&temporary_path).map_err(|_| StagerError::Storage)?;
        temporary_guard.disarm();
        sync_directory(parent).map_err(|_| StagerError::Storage)?;
        Ok(StagedEvidence { object, object_ref })
    }

    pub fn verify(&self, staged: &StagedEvidence) -> Result<(), StagerError> {
        let _global_guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::VerificationFailed)?;
        let _guard = self
            .coordination
            .lock()
            .map_err(|_| StagerError::VerificationFailed)?;
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
        let final_path = self
            .object_path(run_id, attempt, object_id)
            .map_err(|_| StagerError::VerificationFailed)?;
        let file = checked_open_regular(&self.root, &final_path)
            .map_err(|_| StagerError::VerificationFailed)?;
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

    pub fn cleanup_attempt(
        &self,
        run_id: &str,
        attempt: u64,
    ) -> Result<CleanupResult, StagerError> {
        validate_cleanup_scope(run_id, attempt)?;
        let _global_guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::Cleanup)?;
        let _guard = self.coordination.lock().map_err(|_| StagerError::Cleanup)?;
        let attempt_path = self.root.join(run_id).join(attempt.to_string());
        if !safe_existing_components(&self.root, &attempt_path)? {
            return Ok(residual(
                run_id,
                attempt,
                CleanupResidualReason::UnsafeEntry,
            ));
        }
        let metadata = match fs::symlink_metadata(&attempt_path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CleanupResult {
                    complete: true,
                    residuals: Vec::new(),
                })
            }
            Err(_) => return Err(StagerError::Cleanup),
        };
        if !metadata.file_type().is_dir() {
            return Ok(residual(
                run_id,
                attempt,
                CleanupResidualReason::UnsafeEntry,
            ));
        }
        let mut residuals = Vec::new();
        cleanup_owned_tree(&attempt_path, &mut residuals, run_id, attempt)?;
        if residuals.is_empty() {
            remove_empty_parents(&self.root, &attempt_path)?;
        }
        Ok(CleanupResult {
            complete: residuals.is_empty(),
            residuals,
        })
    }

    fn object_path(
        &self,
        run_id: &str,
        attempt: u64,
        object_id: &str,
    ) -> Result<PathBuf, StagerError> {
        validate_run_id(run_id).map_err(|_| StagerError::InvalidObject)?;
        validate_attempt(attempt).map_err(|_| StagerError::InvalidObject)?;
        validate_object_id(object_id).map_err(|_| StagerError::InvalidObject)?;
        let object_number = object_id
            .strip_prefix("evidence/")
            .ok_or(StagerError::InvalidObject)?;
        Ok(self
            .root
            .join(run_id)
            .join(attempt.to_string())
            .join("evidence")
            .join(format!("{object_number}.bin")))
    }
}

#[derive(Clone, Copy)]
struct Quota {
    count: usize,
    bytes: u64,
}

fn validate_request(request: &StageRequest<'_>) -> Result<(), StagerError> {
    validate_run_id(request.run_id).map_err(|_| StagerError::InvalidObject)?;
    validate_attempt(request.attempt).map_err(|_| StagerError::InvalidObject)?;
    validate_object_id(request.object_id).map_err(|_| StagerError::InvalidObject)?;
    if request.bytes.len() > MAX_EVIDENCE_BYTES {
        return Err(StagerError::ObjectTooLarge);
    }
    if !matches!(
        (request.role, request.media_type),
        (EvidenceRole::BrowserScreenshot, EvidenceMediaType::Png)
            | (EvidenceRole::RunnerLog, EvidenceMediaType::PlainTextUtf8)
    ) {
        return Err(StagerError::InvalidObject);
    }
    Ok(())
}

fn validate_run_id(value: &str) -> Result<(), ()> {
    if value.is_empty()
        || value.len() > 64
        || !value.is_ascii()
        || !value.bytes().next().unwrap().is_ascii_alphanumeric()
    {
        return Err(());
    }
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
    {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_attempt(value: u64) -> Result<(), ()> {
    if (1..=MAX_SAFE_ATTEMPT).contains(&value) {
        Ok(())
    } else {
        Err(())
    }
}

fn validate_object_id(value: &str) -> Result<(), ()> {
    let number = value.strip_prefix("evidence/").ok_or(())?;
    if value.is_empty()
        || value.len() > 64
        || number.is_empty()
        || !number.bytes().all(|byte| byte.is_ascii_digit())
    {
        return Err(());
    }
    Ok(())
}

fn build_object(request: &StageRequest<'_>) -> Result<ValidatedValue, StagerError> {
    let object_json = json!({
        "schema_version": SCHEMA_VERSION,
        "run_id": request.run_id,
        "attempt": request.attempt,
        "object_id": request.object_id,
        "role": request.role.as_str(),
        "media_type": request.media_type.as_str(),
        "byte_length": request.bytes.len(),
        "sha256": raw_sha256(request.bytes),
        "ownership": OWNERSHIP,
    });
    let bytes = serde_json::to_vec(&object_json).map_err(|_| StagerError::InvalidObject)?;
    validate_local_evidence_object(&bytes).map_err(|_| StagerError::InvalidObject)
}

fn build_reference(object_id: &str, digest: String) -> Result<ValidatedValue, StagerError> {
    let object_ref_json = json!({ "kind": REFERENCE_KIND, "id": object_id, "schema_version": SCHEMA_VERSION, "content_digest": digest });
    let bytes = serde_json::to_vec(&object_ref_json).map_err(|_| StagerError::InvalidReference)?;
    validate_local_evidence_object_ref(&bytes).map_err(|_| StagerError::InvalidReference)
}

fn ensure_directory_path(root: &Path, target: &Path) -> Result<(), StagerError> {
    ensure_root_directory(root)?;
    let relative = target
        .strip_prefix(root)
        .map_err(|_| StagerError::FilesystemSafety)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() => {
                return Err(StagerError::FilesystemSafety)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir(&current).map_err(|_| StagerError::Storage)?
            }
            Err(_) => return Err(StagerError::Storage),
        }
    }
    Ok(())
}

fn ensure_root_directory(root: &Path) -> Result<(), StagerError> {
    let mut missing = Vec::new();
    let mut current = root.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_dir() {
                    return Err(StagerError::FilesystemSafety);
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.clone());
                if !current.pop() {
                    return Err(StagerError::Storage);
                }
            }
            Err(_) => return Err(StagerError::Storage),
        }
    }
    for directory in missing.into_iter().rev() {
        fs::create_dir(&directory).map_err(|_| StagerError::Storage)?;
    }
    Ok(())
}

fn safe_existing_components(root: &Path, target: &Path) -> Result<bool, StagerError> {
    match fs::symlink_metadata(root) {
        Ok(metadata) if metadata.file_type().is_symlink() => return Ok(false),
        Ok(metadata) if !metadata.file_type().is_dir() => {
            return Err(StagerError::FilesystemSafety)
        }
        Ok(_) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
        Err(_) => return Err(StagerError::Cleanup),
    }
    let relative = target
        .strip_prefix(root)
        .map_err(|_| StagerError::FilesystemSafety)?;
    let mut current = root.to_path_buf();
    for component in relative.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => return Ok(false),
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(true),
            Err(_) => return Err(StagerError::Cleanup),
        }
    }
    Ok(true)
}

fn inspect_attempt(parent: &Path) -> Result<Quota, StagerError> {
    let mut quota = Quota { count: 0, bytes: 0 };
    for entry in fs::read_dir(parent).map_err(|_| StagerError::Storage)? {
        let entry = entry.map_err(|_| StagerError::FilesystemSafety)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| StagerError::FilesystemSafety)?;
        if !is_published_name(&name)
            || !metadata.file_type().is_file()
            || link_count(&metadata) != Some(1)
        {
            return Err(StagerError::FilesystemSafety);
        }
        quota.count += 1;
        quota.bytes = quota
            .bytes
            .checked_add(metadata.len())
            .ok_or(StagerError::QuotaExceeded)?;
    }
    Ok(quota)
}

fn is_published_name(name: &str) -> bool {
    let number = name.strip_suffix(".bin").unwrap_or("");
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn checked_open_regular(root: &Path, path: &Path) -> Result<File, ()> {
    if !safe_existing_components(root, path).map_err(|_| ())? {
        return Err(());
    }
    let before = fs::symlink_metadata(path).map_err(|_| ())?;
    if !before.file_type().is_file() || link_count(&before) != Some(1) {
        return Err(());
    }
    let file = File::open(path).map_err(|_| ())?;
    let opened = file.metadata().map_err(|_| ())?;
    let after = fs::symlink_metadata(path).map_err(|_| ())?;
    if !after.file_type().is_file() || link_count(&after) != Some(1) || !same_file(&opened, &after)
    {
        return Err(());
    }
    Ok(file)
}

#[cfg(unix)]
fn link_count(metadata: &fs::Metadata) -> Option<u64> {
    Some(std::os::unix::fs::MetadataExt::nlink(metadata))
}
#[cfg(not(unix))]
fn link_count(_metadata: &fs::Metadata) -> Option<u64> {
    None
}

#[cfg(unix)]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    left.dev() == right.dev() && left.ino() == right.ino()
}
#[cfg(not(unix))]
fn same_file(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    left.len() == right.len()
}

fn cleanup_owned_tree(
    path: &Path,
    residuals: &mut Vec<CleanupResidual>,
    run_id: &str,
    attempt: u64,
) -> Result<(), StagerError> {
    for entry in fs::read_dir(path).map_err(|_| StagerError::Cleanup)? {
        let entry = entry.map_err(|_| StagerError::Cleanup)?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let metadata = fs::symlink_metadata(entry.path()).map_err(|_| StagerError::Cleanup)?;
        if metadata.file_type().is_symlink() {
            residuals.push(residual_item(
                run_id,
                attempt,
                CleanupResidualReason::UnsafeEntry,
            ));
        } else if metadata.file_type().is_dir() {
            if name == "evidence" {
                cleanup_owned_tree(&entry.path(), residuals, run_id, attempt)?;
            } else {
                residuals.push(residual_item(
                    run_id,
                    attempt,
                    CleanupResidualReason::UnrelatedEntry,
                ));
            }
        } else if is_published_name(&name) || is_temporary_name(&name) {
            if !metadata.file_type().is_file() {
                residuals.push(residual_item(
                    run_id,
                    attempt,
                    CleanupResidualReason::UnsafeEntry,
                ));
            } else if fs::remove_file(entry.path()).is_err() {
                residuals.push(residual_item(
                    run_id,
                    attempt,
                    CleanupResidualReason::RemovalFailed,
                ));
            }
        } else {
            residuals.push(residual_item(
                run_id,
                attempt,
                CleanupResidualReason::UnrelatedEntry,
            ));
        }
    }
    if residuals.is_empty() {
        let _ = fs::remove_dir(path);
    }
    Ok(())
}

fn is_temporary_name(name: &str) -> bool {
    name.starts_with('.') && name.ends_with(".tmp")
}
fn residual_item(run_id: &str, attempt: u64, reason: CleanupResidualReason) -> CleanupResidual {
    CleanupResidual {
        run_id: run_id.to_owned(),
        attempt,
        reason,
    }
}
fn residual(run_id: &str, attempt: u64, reason: CleanupResidualReason) -> CleanupResult {
    CleanupResult {
        complete: false,
        residuals: vec![residual_item(run_id, attempt, reason)],
    }
}

fn remove_empty_parents(root: &Path, attempt_path: &Path) -> Result<(), StagerError> {
    let mut current = attempt_path.to_path_buf();
    while current != root {
        match fs::remove_dir(&current) {
            Ok(()) => {
                let _ = current.pop();
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound
                    || error.kind() == std::io::ErrorKind::DirectoryNotEmpty =>
            {
                break
            }
            Err(_) => return Err(StagerError::Cleanup),
        }
    }
    Ok(())
}

fn validate_cleanup_scope(run_id: &str, attempt: u64) -> Result<(), StagerError> {
    validate_run_id(run_id).map_err(|_| StagerError::InvalidCleanupScope)?;
    validate_attempt(attempt).map_err(|_| StagerError::InvalidCleanupScope)
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
