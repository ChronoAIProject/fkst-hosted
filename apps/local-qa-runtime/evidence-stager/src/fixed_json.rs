//! Restricted local approval for #6146. No PNG policy, upload, or signed-policy claim.

use std::collections::BTreeMap;
use std::path::Component;
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_qa_contracts::{
    admit_json, canonical_admitted_bytes, sha256_digest, validate_local_fixed_json_export,
    validate_local_fixed_json_receipt, validate_local_fixed_json_source,
    LOCAL_FIXED_JSON_MAX_BYTES, LOCAL_FIXED_JSON_POLICY_DIGEST, LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES,
};

use super::*;

pub struct FixedJsonExportRequest<'a> {
    pub run_id: &'a str,
    pub attempt: u64,
    pub observation_id: &'a str,
    pub raw_bytes: &'a [u8],
}

/// Issued only by durable staging; never reconstructed from a caller's receipt or path.
///
/// ```compile_fail
/// use fkst_local_qa_evidence_stager::FixedJsonExportHandle;
/// let forged = FixedJsonExportHandle {};
/// ```
#[derive(Clone)]
pub struct FixedJsonExportHandle {
    root: PathBuf,
    run_id: String,
    attempt: u64,
    observation_id: String,
    receipt_digest: String,
}

impl std::fmt::Debug for FixedJsonExportHandle {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("FixedJsonExportHandle")
            .finish_non_exhaustive()
    }
}

pub struct FixedJsonExport {
    receipt: ValidatedValue,
    bytes: Vec<u8>,
}

impl FixedJsonExport {
    pub fn receipt(&self) -> &ValidatedValue {
        &self.receipt
    }

    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FixedJsonNamespace {
    Raw,
    Export,
}

impl FixedJsonNamespace {
    fn directory(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Export => "export",
        }
    }

    fn owned_directory(self) -> OwnedDirectory {
        match self {
            Self::Raw => OwnedDirectory::FixedJsonRaw,
            Self::Export => OwnedDirectory::FixedJsonExport,
        }
    }
}

/// Physical ownership status, including partial publication files; not an eligibility signal.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FixedJsonStagingStatus {
    pub files: usize,
    pub byte_length: u64,
}

impl EvidenceStager {
    /// Uses the sole built-in policy. Raw logs and PNG bytes fail the closed JSON validator.
    pub fn stage_fixed_json_export(
        &self,
        request: FixedJsonExportRequest<'_>,
    ) -> Result<FixedJsonExportHandle, StagerError> {
        if request.raw_bytes.len() > LOCAL_FIXED_JSON_MAX_BYTES {
            return Err(StagerError::ObjectTooLarge);
        }
        let scope = self.fixed_json_scope(request.run_id, request.attempt)?;
        validate_observation_id(request.observation_id)
            .map_err(|_| StagerError::InvalidIdentifier)?;
        let source = validate_local_fixed_json_source(request.raw_bytes)
            .map_err(|_| StagerError::InvalidObject)?;
        if source.value()["run_id"] != request.run_id
            || source.value()["attempt"] != request.attempt
        {
            return Err(StagerError::InvalidObject);
        }
        let output = canonical_bytes(&source).map_err(|_| StagerError::InvalidObject)?;
        validate_local_fixed_json_source(&output).map_err(|_| StagerError::InvalidObject)?;

        let _guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::Storage)?;
        check_root(&self.root)?;
        let raw = inspect_namespace(&self.root, &scope, FixedJsonNamespace::Raw)?;
        let exported = inspect_namespace(&self.root, &scope, FixedJsonNamespace::Export)?;
        // Each existing slot must already have its complete receipt-last publication.
        if exported.len() != raw.len() || raw.len() % 2 != 0 {
            return Err(StagerError::VerificationFailed);
        }
        let sources: Vec<_> = raw
            .keys()
            .filter(|name| numbered_file_name(name, ".json"))
            .collect();
        if sources.len() * 2 != raw.len() {
            return Err(StagerError::VerificationFailed);
        }
        for name in &sources {
            let number = name
                .strip_suffix(".json")
                .ok_or(StagerError::VerificationFailed)?;
            if !raw.contains_key(&format!("{number}.receipt-digest"))
                || !exported.contains_key(*name)
                || !exported.contains_key(&format!("{number}.receipt.json"))
            {
                return Err(StagerError::VerificationFailed);
            }
            self.read_fixed_json_bundle(
                request.run_id,
                request.attempt,
                &format!("observation/{number}"),
            )?;
        }
        let paths = object_paths(&scope, request.observation_id)?;
        let source_name = paths
            .raw
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or(StagerError::Storage)?;
        if raw.contains_key(source_name) {
            let existing = self.read_fixed_json_bundle(
                request.run_id,
                request.attempt,
                request.observation_id,
            )?;
            if existing.receipt.value()["source_digest"] != sha256_digest(request.raw_bytes)
                || existing.bytes != output
            {
                return Err(StagerError::DuplicateIdentity);
            }
            // A prior publisher may have stopped after linking the receipt but before directory sync.
            sync_directory(paths.receipt.parent().ok_or(StagerError::Storage)?)?;
            return self.fixed_json_handle(&request, &existing.receipt);
        }
        let total_bytes: u64 = raw.values().chain(exported.values()).sum();
        if sources.len() >= MAX_OBJECTS_PER_ATTEMPT
            || total_bytes.saturating_add(
                (request.raw_bytes.len() + output.len() + LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES + 71)
                    as u64,
            ) > MAX_ATTEMPT_BYTES
        {
            return Err(StagerError::QuotaExceeded);
        }
        let receipt = build_receipt(&request, &output)?;
        let receipt_bytes = canonical_bytes(&receipt).map_err(|_| StagerError::InvalidObject)?;
        validate_local_fixed_json_export(&receipt_bytes, request.raw_bytes, &output)
            .map_err(|_| StagerError::InvalidObject)?;

        publish_new_file(&self.root, &paths.raw, request.raw_bytes)?;
        // Anchor the original receipt (including timestamp) in the local raw namespace.
        publish_new_file(
            &self.root,
            &paths.receipt_digest,
            sha256_digest(&receipt_bytes).as_bytes(),
        )?;
        publish_new_file(&self.root, &paths.output, &output)?;
        // Sync newly-created directory ancestry before publishing the commit record.
        sync_ancestry(&self.root, paths.raw.parent().ok_or(StagerError::Storage)?)?;
        sync_ancestry(
            &self.root,
            paths.output.parent().ok_or(StagerError::Storage)?,
        )?;
        publish_new_file(&self.root, &paths.receipt, &receipt_bytes)?;
        let verified =
            self.read_fixed_json_bundle(request.run_id, request.attempt, request.observation_id)?;
        self.fixed_json_handle(&request, &verified.receipt)
    }

    /// Revalidates storage on every read. Cleanup or missing/tampered raw/output bytes revoke use.
    ///
    /// ```compile_fail
    /// use fkst_local_qa_evidence_stager::{EvidenceStager, StagedEvidence};
    /// fn raw_is_not_exportable(stager: &EvidenceStager, raw: &StagedEvidence) {
    ///     stager.read_fixed_json_export(raw).unwrap();
    /// }
    /// ```
    pub fn read_fixed_json_export(
        &self,
        handle: &FixedJsonExportHandle,
    ) -> Result<FixedJsonExport, StagerError> {
        if handle.root != self.root {
            return Err(StagerError::VerificationFailed);
        }
        let _guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::VerificationFailed)?;
        check_root(&self.root)?;
        let loaded =
            self.read_fixed_json_bundle(&handle.run_id, handle.attempt, &handle.observation_id)?;
        if contract_content_digest(&loaded.receipt).map_err(|_| StagerError::VerificationFailed)?
            != handle.receipt_digest
        {
            return Err(StagerError::VerificationFailed);
        }
        Ok(loaded)
    }

    pub fn fixed_json_status(
        &self,
        run_id: &str,
        attempt: u64,
        namespace: FixedJsonNamespace,
    ) -> Result<FixedJsonStagingStatus, StagerError> {
        let scope = self.fixed_json_scope(run_id, attempt)?;
        let _guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::Storage)?;
        check_root(&self.root)?;
        let files = inspect_namespace(&self.root, &scope, namespace)?;
        Ok(FixedJsonStagingStatus {
            files: files.len(),
            byte_length: files.values().sum(),
        })
    }

    /// Deletes only the specified owned namespace. It does not release any execution slot.
    pub fn cleanup_fixed_json(
        &self,
        run_id: &str,
        attempt: u64,
        namespace: FixedJsonNamespace,
    ) -> Result<CleanupResult, StagerError> {
        validate_cleanup_scope(run_id, attempt)?;
        let scope = self.fixed_json_scope(run_id, attempt)?;
        let path = scope.join(namespace.directory());
        let _guard = STAGING_COORDINATION
            .lock()
            .map_err(|_| StagerError::Cleanup)?;
        if check_root(&self.root).is_err() || !safe_existing_components(&self.root, &path)? {
            return Ok(residual(
                run_id,
                attempt,
                CleanupResidualReason::UnsafeEntry,
            ));
        }
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(CleanupResult {
                    complete: true,
                    residuals: Vec::new(),
                });
            }
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => {
                return Ok(residual(
                    run_id,
                    attempt,
                    CleanupResidualReason::UnsafeEntry,
                ))
            }
            Err(_) => return Err(StagerError::Cleanup),
        }
        let mut residuals = Vec::new();
        cleanup_owned_tree(
            &path,
            namespace.owned_directory(),
            &mut residuals,
            run_id,
            attempt,
        )?;
        if residuals.is_empty() {
            remove_empty_parents(&self.root, &scope)?;
        }
        Ok(CleanupResult {
            complete: residuals.is_empty(),
            residuals,
        })
    }

    fn fixed_json_scope(&self, run_id: &str, attempt: u64) -> Result<PathBuf, StagerError> {
        validate_run_id(run_id).map_err(|_| StagerError::InvalidIdentifier)?;
        validate_attempt(attempt).map_err(|_| StagerError::InvalidIdentifier)?;
        Ok(self
            .root
            .join("fixed-json")
            .join(run_id)
            .join(attempt.to_string()))
    }

    fn fixed_json_handle(
        &self,
        request: &FixedJsonExportRequest<'_>,
        receipt: &ValidatedValue,
    ) -> Result<FixedJsonExportHandle, StagerError> {
        Ok(FixedJsonExportHandle {
            root: self.root.clone(),
            run_id: request.run_id.to_owned(),
            attempt: request.attempt,
            observation_id: request.observation_id.to_owned(),
            receipt_digest: contract_content_digest(receipt)
                .map_err(|_| StagerError::VerificationFailed)?,
        })
    }

    fn read_fixed_json_bundle(
        &self,
        run_id: &str,
        attempt: u64,
        observation_id: &str,
    ) -> Result<FixedJsonExport, StagerError> {
        let scope = self.fixed_json_scope(run_id, attempt)?;
        let paths = object_paths(&scope, observation_id)?;
        let source = read_bounded(&self.root, &paths.raw, LOCAL_FIXED_JSON_MAX_BYTES)?;
        let output = read_bounded(&self.root, &paths.output, LOCAL_FIXED_JSON_MAX_BYTES)?;
        let receipt_bytes = read_bounded(
            &self.root,
            &paths.receipt,
            LOCAL_FIXED_JSON_RECEIPT_MAX_BYTES,
        )?;
        let anchor = read_bounded(&self.root, &paths.receipt_digest, 71)?;
        if anchor != sha256_digest(&receipt_bytes).as_bytes() {
            return Err(StagerError::VerificationFailed);
        }
        let receipt = validate_local_fixed_json_export(&receipt_bytes, &source, &output)
            .map_err(|_| StagerError::VerificationFailed)?;
        if receipt.value()["run_id"] != run_id
            || receipt.value()["attempt"] != attempt
            || receipt.value()["object_id"] != observation_id
            || canonical_bytes(&receipt).map_err(|_| StagerError::VerificationFailed)?
                != receipt_bytes
        {
            return Err(StagerError::VerificationFailed);
        }
        Ok(FixedJsonExport {
            receipt,
            bytes: output,
        })
    }
}

struct ObjectPaths {
    receipt_digest: PathBuf,
    raw: PathBuf,
    output: PathBuf,
    receipt: PathBuf,
}

fn object_paths(scope: &Path, id: &str) -> Result<ObjectPaths, StagerError> {
    validate_observation_id(id).map_err(|_| StagerError::InvalidIdentifier)?;
    let number = id
        .strip_prefix("observation/")
        .ok_or(StagerError::InvalidIdentifier)?;
    Ok(ObjectPaths {
        receipt_digest: scope.join("raw").join(format!("{number}.receipt-digest")),
        raw: scope.join("raw").join(format!("{number}.json")),
        output: scope.join("export").join(format!("{number}.json")),
        receipt: scope.join("export").join(format!("{number}.receipt.json")),
    })
}

fn build_receipt(
    request: &FixedJsonExportRequest<'_>,
    output: &[u8],
) -> Result<ValidatedValue, StagerError> {
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| StagerError::Storage)?
        .as_millis();
    let timestamp = u64::try_from(timestamp).map_err(|_| StagerError::Storage)?;
    let mut value = json!({
        "schema_version": "qa.local-fixed-json-export/v1",
        "kind": "completed-local-fixed-json",
        "run_id": request.run_id,
        "attempt": request.attempt,
        "object_id": request.observation_id,
        "role": "fixed-sanitized-observation",
        "source_digest": sha256_digest(request.raw_bytes),
        "source_byte_length": request.raw_bytes.len(),
        "policy_profile": "local-host-built-in-fixed-observation",
        "policy_version": 1,
        "policy_digest": LOCAL_FIXED_JSON_POLICY_DIGEST,
        "output_digest": sha256_digest(output),
        "media_type": "application/json",
        "output_byte_length": output.len(),
        "created_at_unix_ms": timestamp,
    });
    let projection = serde_json::to_vec(&value).map_err(|_| StagerError::InvalidObject)?;
    let admitted = admit_json(&projection).map_err(|_| StagerError::InvalidObject)?;
    value["content_digest"] = sha256_digest(
        &canonical_admitted_bytes(&admitted).map_err(|_| StagerError::InvalidObject)?,
    )
    .into();
    validate_local_fixed_json_receipt(
        &serde_json::to_vec(&value).map_err(|_| StagerError::InvalidObject)?,
    )
    .map_err(|_| StagerError::InvalidObject)
}

fn read_bounded(root: &Path, path: &Path, maximum: usize) -> Result<Vec<u8>, StagerError> {
    let file = checked_open_regular(root, path).map_err(|_| StagerError::VerificationFailed)?;
    let mut bytes = Vec::new();
    file.take((maximum + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| StagerError::VerificationFailed)?;
    if bytes.len() > maximum {
        return Err(StagerError::VerificationFailed);
    }
    Ok(bytes)
}

fn check_root(root: &Path) -> Result<(), StagerError> {
    if !root.is_absolute()
        || root
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::CurDir))
    {
        return Err(StagerError::FilesystemSafety);
    }
    for ancestor in root.ancestors() {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if !metadata.file_type().is_dir() => {
                return Err(StagerError::FilesystemSafety)
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => return Err(StagerError::FilesystemSafety),
        }
    }
    Ok(())
}

fn inspect_namespace(
    root: &Path,
    scope: &Path,
    namespace: FixedJsonNamespace,
) -> Result<BTreeMap<String, u64>, StagerError> {
    let directory = scope.join(namespace.directory());
    if !safe_existing_components(root, &directory)? {
        return Err(StagerError::FilesystemSafety);
    }
    let entries = match fs::read_dir(&directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(_) => return Err(StagerError::FilesystemSafety),
    };
    let mut files = BTreeMap::new();
    for entry in entries {
        let entry = entry.map_err(|_| StagerError::FilesystemSafety)?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| StagerError::FilesystemSafety)?;
        let metadata =
            fs::symlink_metadata(entry.path()).map_err(|_| StagerError::FilesystemSafety)?;
        if !fixed_json_owned_name(&name, namespace == FixedJsonNamespace::Export)
            || !metadata.file_type().is_file()
            || link_count(&metadata) != Some(1)
        {
            return Err(StagerError::FilesystemSafety);
        }
        if files.len() >= MAX_OBJECTS_PER_ATTEMPT * 3 || metadata.len() > MAX_EVIDENCE_BYTES as u64
        {
            return Err(StagerError::QuotaExceeded);
        }
        files.insert(name, metadata.len());
    }
    Ok(files)
}

pub(super) fn fixed_json_owned_name(name: &str, export: bool) -> bool {
    let logical_name = if name.starts_with('.') {
        let Some(name) = name
            .strip_prefix('.')
            .and_then(|name| name.strip_suffix(".tmp"))
        else {
            return false;
        };
        let Some((name, sequence)) = name.rsplit_once('.') else {
            return false;
        };
        let Some((name, process)) = name.rsplit_once('.') else {
            return false;
        };
        if sequence.is_empty()
            || process.is_empty()
            || !sequence
                .bytes()
                .chain(process.bytes())
                .all(|byte| byte.is_ascii_digit())
        {
            return false;
        }
        name
    } else {
        name
    };
    let number = if export {
        logical_name
            .strip_suffix(".receipt.json")
            .or_else(|| logical_name.strip_suffix(".json"))
    } else {
        logical_name
            .strip_suffix(".receipt-digest")
            .or_else(|| logical_name.strip_suffix(".json"))
    };
    number.is_some_and(|number| validate_observation_id(&format!("observation/{number}")).is_ok())
}

fn sync_ancestry(root: &Path, directory: &Path) -> Result<(), StagerError> {
    for path in directory.ancestors() {
        sync_directory(path)?;
        if Some(path) == root.parent() {
            break;
        }
    }
    Ok(())
}

#[cfg(all(test, unix))]
pub(super) mod tests {
    use std::cell::RefCell;

    use super::*;

    thread_local! {
        static SYNC_FAILURE: RefCell<Option<(PathBuf, usize)>> = const { RefCell::new(None) };
    }

    struct SyncFailure;

    impl SyncFailure {
        fn on_call(path: &Path, call: usize) -> Self {
            SYNC_FAILURE.with(|slot| {
                assert!(slot.borrow().is_none());
                *slot.borrow_mut() = Some((path.to_owned(), call));
            });
            Self
        }
    }

    impl Drop for SyncFailure {
        fn drop(&mut self) {
            SYNC_FAILURE.with(|slot| *slot.borrow_mut() = None);
        }
    }

    pub(crate) fn before_directory_sync(path: &Path) -> Result<(), StagerError> {
        SYNC_FAILURE.with(|slot| {
            let mut failure = slot.borrow_mut();
            if let Some((target, remaining)) = failure.as_mut() {
                if target == path {
                    *remaining -= 1;
                    if *remaining == 0 {
                        *failure = None;
                        return Err(StagerError::Storage);
                    }
                }
            }
            Ok(())
        })
    }

    #[test]
    fn replay_requires_directory_sync_after_interrupted_receipt_publication() {
        let temporary = tempfile::TempDir::new().unwrap();
        let root = temporary.path().canonicalize().unwrap().join("quarantine");
        let fixture: serde_json::Value = serde_json::from_str(include_str!(
            "../../../../packages/qa-contracts/fixtures/qa.local-fixed-json-export/v1/conformance.json"
        )).unwrap();
        let raw = fixture["source_cases"][0]["raw_utf8"]
            .as_str()
            .unwrap()
            .as_bytes();
        let request = || FixedJsonExportRequest {
            run_id: "run-1",
            attempt: 1,
            observation_id: "observation/0",
            raw_bytes: raw,
        };
        let export_dir = root.join("fixed-json/run-1/1/export");
        {
            // Output publication and ancestry sync precede the receipt's final directory sync.
            let _failure = SyncFailure::on_call(&export_dir, 3);
            let stager = EvidenceStager::new(&root);
            assert_eq!(
                stager.stage_fixed_json_export(request()).unwrap_err(),
                StagerError::Storage
            );
        }
        let receipt_path = export_dir.join("0.receipt.json");
        let original_receipt = fs::read(&receipt_path).unwrap();
        let original_output = fs::read(export_dir.join("0.json")).unwrap();
        assert_eq!(fs::read_dir(&export_dir).unwrap().count(), 2);
        assert_eq!(link_count(&fs::metadata(&receipt_path).unwrap()), Some(1));
        validate_local_fixed_json_export(&original_receipt, raw, &original_output).unwrap();
        {
            let _failure = SyncFailure::on_call(&export_dir, 1);
            let restarted = EvidenceStager::new(&root);
            assert_eq!(
                restarted.stage_fixed_json_export(request()).unwrap_err(),
                StagerError::Storage
            );
        }
        let restarted = EvidenceStager::new(&root);
        let handle = restarted.stage_fixed_json_export(request()).unwrap();
        let replay = restarted.read_fixed_json_export(&handle).unwrap();
        assert_eq!(canonical_bytes(replay.receipt()).unwrap(), original_receipt);
        assert_eq!(replay.bytes(), original_output);
        assert_eq!(fs::read(receipt_path).unwrap(), original_receipt);
    }
}
