use std::cell::{Cell, RefCell};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[path = "source_workspace_fs.rs"]
mod filesystem;
use filesystem::{Directory, PinnedFile};

use fkst_qa_contracts::{
    compare_iso8601_timestamps, sha256_digest, validate_scalar, DigestBoundReferenceV2,
};
use serde::{Deserialize, Serialize};

pub use crate::journal::workspace::{OwnedWorkspace, WorkspaceIntent, WorkspaceState};
use crate::journal::Journal;
use crate::ownership::Clock;
use crate::RunError;

const WORKSPACE_MARKER: &str = ".fkst-workspace.json";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ImmutableRevision {
    GitCommit(String),
    ObjectDigest(String),
}

impl ImmutableRevision {
    fn validate(&self) -> Result<(), RunError> {
        match self {
            Self::GitCommit(commit) => {
                if commit.len() != 40
                    || !commit
                        .bytes()
                        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                {
                    return Err(RunError::Lifecycle(
                        "source revision must be an exact lowercase Git commit",
                    ));
                }
            }
            Self::ObjectDigest(digest) => validate_scalar("Sha256", digest)
                .map_err(|_| RunError::Lifecycle("source object revision must be a SHA-256"))?,
        }
        Ok(())
    }

    fn marker_value(&self) -> String {
        match self {
            Self::GitCommit(commit) => format!("git:{commit}"),
            Self::ObjectDigest(digest) => format!("object:{digest}"),
        }
    }
}

/// Expectations supplied by the trusted local embedding, independently of the
/// incoming reference. This is not a production lease or authenticated authority.
/// Revision and provider labels are declared facts; only raw bytes are hashed here.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrustedLocalSourceBinding {
    pub reference: DigestBoundReferenceV2,
    pub source_object_id: String,
    pub expected_raw_digest: String,
    pub expected_revision: ImmutableRevision,
    pub expected_provider_scope: String,
    pub expected_provider_identity: String,
}

/// Local acquisition envelope; no signed SourceObjectLease contract is implemented.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectLease {
    pub lease_id: String,
    pub binding: TrustedLocalSourceBinding,
    pub run_id: String,
    pub generation: i64,
    pub deadline_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredSource {
    pub source_object_id: String,
    pub immutable_revision: ImmutableRevision,
    pub provider_scope: String,
    pub provider_identity: String,
    pub bytes: Vec<u8>,
}

pub trait SourceProvider {
    fn acquire(&mut self, lease: &SourceObjectLease) -> Result<AcquiredSource, RunError>;
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceResource {
    pub intent: WorkspaceIntent,
    pub provider_identity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMaterialization {
    pub resource: WorkspaceResource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceProviderStatus {
    Active,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderStatusReceipt {
    pub resource: WorkspaceResource,
    pub status: WorkspaceProviderStatus,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkspaceDiscovery {
    Absent,
    Found(Box<WorkspaceProviderStatusReceipt>),
    Unknown,
    Conflict,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderStopReceipt {
    pub resource: WorkspaceResource,
    pub stopped: bool,
}

/// A trusted local adapter must report observed ownership, not echo requested labels.
/// Stable keys must name one resource within this adapter's immutable scope.
pub trait WorkspaceProvider {
    fn scope(&self) -> &str;
    fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError>;
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        verified_source_blob: &Path,
        workspace_root: &Path,
        immutable_revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError>;
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, RunError>;
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, RunError>;
}

/// Supplied by the trusted local embedding. All additional adapter-writable trees
/// must be declared; this is a storage contract, not OS isolation from the same UID.
pub struct WorkspaceProviderScope {
    pub identity: String,
    pub writable_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRequest {
    pub run_id: String,
    pub generation: i64,
    pub deadline_utc: String,
}

struct VerifiedSource {
    source_object_id: String,
    content_digest: String,
    immutable_revision: ImmutableRevision,
    provider_identity: String,
    cache_path: PathBuf,
    blob: PinnedFile,
    marker: PinnedFile,
    binding_receipt: PinnedFile,
    binding: TrustedLocalSourceBinding,
}

impl VerifiedSource {
    fn validate(&self) -> Result<(), RunError> {
        verify_file_digest(&self.blob, &self.binding.expected_raw_digest)?;
        let marker = read_cache_marker(&self.marker)?;
        if marker.content_digest != self.binding.expected_raw_digest {
            return Err(RunError::Lifecycle("source cache byte metadata mismatch"));
        }
        validate_binding_receipt(&self.binding_receipt, &self.binding)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHandle {
    source_binding: Option<TrustedLocalSourceBinding>,
    run_id: String,
    generation: i64,
    source_object_id: String,
    source_digest: String,
    immutable_revision: ImmutableRevision,
    source_provider_identity: String,
    workspace_provider_identity: String,
    root: PathBuf,
}

impl WorkspaceHandle {
    pub fn root(&self) -> &Path {
        &self.root
    }
    pub fn workspace_provider_identity(&self) -> &str {
        &self.workspace_provider_identity
    }
    pub fn run_id(&self) -> &str {
        &self.run_id
    }
    pub fn generation(&self) -> i64 {
        self.generation
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceStatus {
    Active,
    Stopped,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceStopReceipt {
    pub run_id: String,
    pub generation: i64,
    pub workspace_provider_identity: String,
    pub already_stopped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleAuthorityBlocker {
    MissingSourceObjectLeaseBinding,
    MissingEnvironmentProviderProjection,
    MissingReadinessReceiptMapping,
}

pub fn lifecycle_authority_blockers() -> [LifecycleAuthorityBlocker; 3] {
    [
        LifecycleAuthorityBlocker::MissingSourceObjectLeaseBinding,
        LifecycleAuthorityBlocker::MissingEnvironmentProviderProjection,
        LifecycleAuthorityBlocker::MissingReadinessReceiptMapping,
    ]
}

struct JournalLocation {
    database: PinnedFile,
    sidecars: Vec<PinnedFile>,
    parent: Arc<Directory>,
    name: std::ffi::OsString,
}

impl JournalLocation {
    fn new(journal: &Journal, exposed: &[Arc<Directory>]) -> Result<Self, RunError> {
        let path = journal
            .connection
            .path()
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .ok_or(RunError::Lifecycle(
                "workspace journal must be a persistent database",
            ))?;
        let parent_path = path
            .parent()
            .ok_or(RunError::Lifecycle("workspace journal location is unknown"))?;
        let parent = Directory::prepare(parent_path)?;
        for root in exposed {
            root.ensure_attached()?;
            if parent.overlaps(root)? {
                return Err(RunError::Lifecycle(
                    "workspace journal must be outside provider-writable trees",
                ));
            }
        }
        let name = file_name(&path)?.to_owned();
        let database = parent
            .open_file(&name)?
            .ok_or(RunError::Lifecycle("workspace journal is missing"))?;
        let mut sidecars = Vec::new();
        for suffix in ["-wal", "-shm"] {
            let mut sidecar = name.clone();
            sidecar.push(suffix);
            if let Some(file) = parent.open_file(&sidecar)? {
                sidecars.push(file);
            }
        }
        let location = Self {
            database,
            sidecars,
            parent,
            name,
        };
        location.ensure_attached()?;
        // Storage provisioning must remain stable while Journal::open opens SQLite
        // and captures the named file identity. This is not an atomic VFS binding.
        // Reject stale provisioning before executing any manager SQL: shared WAL
        // visibility alone cannot identify the connection's main database file.
        if journal.opened_database_identity != Some(location.database.identity_chain()?) {
            return Err(RunError::Lifecycle(
                "workspace journal connection location changed",
            ));
        }
        Ok(location)
    }

    fn ensure_attached(&self) -> Result<(), RunError> {
        self.database.ensure_attached()?;
        for file in &self.sidecars {
            file.ensure_attached()?;
        }
        for suffix in ["-wal", "-shm"] {
            let mut name = self.name.clone();
            name.push(suffix);
            if let Some(file) = self.parent.open_file(&name)? {
                file.ensure_attached()?;
            }
        }
        Ok(())
    }
}

pub struct SourceWorkspaceManager {
    cache_root: Arc<Directory>,
    workspace_root: Arc<Directory>,
    journal: RefCell<Journal>,
    journal_location: JournalLocation,
    provider_scope: String,
    exposed_roots: Vec<Arc<Directory>>,
    busy: Cell<bool>,
}

struct Operation<'a>(&'a Cell<bool>);
impl Drop for Operation<'_> {
    fn drop(&mut self) {
        self.0.set(false);
    }
}

impl SourceWorkspaceManager {
    pub fn new(
        cache_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
        journal: Journal,
        provider_scope: WorkspaceProviderScope,
    ) -> Result<Self, RunError> {
        let cache_root = cache_root.as_ref();
        let workspace_root = workspace_root.as_ref();
        if cache_root == workspace_root
            || cache_root.starts_with(workspace_root)
            || workspace_root.starts_with(cache_root)
        {
            return Err(RunError::Lifecycle(
                "source cache and workspace roots must be disjoint",
            ));
        }
        if provider_scope.identity.is_empty() {
            return Err(RunError::Lifecycle(
                "workspace provider scope must not be empty",
            ));
        }
        let cache_root = Directory::prepare(cache_root)?;
        let workspace_root = Directory::prepare(workspace_root)?;
        if cache_root.overlaps(&workspace_root)? {
            return Err(RunError::Lifecycle(
                "source cache and workspace roots overlap",
            ));
        }
        let mut exposed_roots = vec![cache_root.clone(), workspace_root.clone()];
        for path in provider_scope.writable_roots {
            exposed_roots.push(Directory::prepare(&path)?);
        }
        let journal_location = JournalLocation::new(&journal, &exposed_roots)?;
        Ok(Self {
            cache_root,
            workspace_root,
            journal: RefCell::new(journal),
            journal_location,
            provider_scope: provider_scope.identity,
            exposed_roots,
            busy: Cell::new(false),
        })
    }

    fn enter(&self) -> Result<Operation<'_>, RunError> {
        if self.busy.replace(true) {
            return Err(RunError::Lifecycle(
                "workspace operation is already in progress",
            ));
        }
        Ok(Operation(&self.busy))
    }

    fn with_journal<T>(
        &self,
        action: impl FnOnce(&mut Journal) -> Result<T, RunError>,
    ) -> Result<T, RunError> {
        self.journal_location.ensure_attached()?;
        for root in &self.exposed_roots {
            root.ensure_attached()?;
        }
        let result = {
            let mut journal = self
                .journal
                .try_borrow_mut()
                .map_err(|_| RunError::Lifecycle("workspace journal is busy"))?;
            action(&mut journal)?
        };
        self.journal_location.ensure_attached()?;
        Ok(result)
    }

    fn check_provider(&self, provider: &impl WorkspaceProvider) -> Result<(), RunError> {
        if provider.scope() != self.provider_scope {
            return Err(RunError::Lifecycle("workspace provider scope mismatch"));
        }
        Ok(())
    }

    fn transition(
        &self,
        record: &OwnedWorkspace,
        state: WorkspaceState,
    ) -> Result<OwnedWorkspace, RunError> {
        let mut after = record.clone();
        after.state = state;
        after.blocker = None;
        self.with_journal(|journal| journal.update_workspace(record, &after))
    }

    fn block<T>(&self, record: &OwnedWorkspace, reason: &'static str) -> Result<T, RunError> {
        // The directory claim itself preserves uncertainty; a competing observer
        // must not invalidate its creator's pending identity publication.
        if record.state == WorkspaceState::DirectoryAttempted {
            return Err(RunError::Lifecycle(reason));
        }
        let mut after = record.clone();
        after.blocker = Some(reason.to_owned());
        self.with_journal(|journal| journal.update_workspace(record, &after))?;
        Err(RunError::Lifecycle(reason))
    }

    pub fn prepare<S: SourceProvider, W: WorkspaceProvider>(
        &self,
        source_provider: &mut S,
        workspace_provider: &mut W,
        source_reference: &DigestBoundReferenceV2,
        lease: &SourceObjectLease,
        request: &WorkspaceRequest,
        clock: &impl Clock,
    ) -> Result<WorkspaceHandle, RunError> {
        let _operation = self.enter()?;
        self.check_provider(workspace_provider)?;
        self.with_journal(|_| Ok(()))?;
        validate_binding(source_reference, lease, request, clock)?;
        let existing_record = self.with_journal(|journal| {
            journal.workspace(&stable_workspace_key(&request.run_id, request.generation))
        })?;
        if let Some(record) = &existing_record {
            if record.intent.source_binding.as_ref() != Some(&lease.binding)
                || record.intent.source_lease_id != lease.lease_id
                || record.intent.deadline_utc != request.deadline_utc
            {
                return Err(RunError::Lifecycle(
                    "missing or conflicting durable source binding",
                ));
            }
        }
        self.workspace_directory(&request.run_id, request.generation, false)?;
        let verified = match self.load_cached_source(&lease.binding)? {
            Some(verified) => verified,
            None if existing_record.is_some() => {
                return Err(RunError::Lifecycle(
                    "existing workspace source receipt is unavailable",
                ));
            }
            None => {
                let acquired = source_provider.acquire(lease)?;
                validate_acquired(&lease.binding, &acquired)?;
                ensure_before_deadline(clock, &request.deadline_utc)?;
                self.cache_verified_source(&lease.binding, acquired)?
            }
        };
        self.materialize_workspace(workspace_provider, lease, request, &verified, clock)
    }

    /// Reconcile an existing ownership obligation without admitting a new create.
    /// The returned handle identifies ownership; call status to establish usability.
    /// Expiration does not prevent discovering or stopping an already attempted resource.
    pub fn recover<W: WorkspaceProvider>(
        &self,
        provider: &mut W,
        stable_key: &str,
    ) -> Result<WorkspaceHandle, RunError> {
        let _operation = self.enter()?;
        self.check_provider(provider)?;
        let mut record = self
            .with_journal(|journal| journal.workspace(stable_key))?
            .ok_or(RunError::Lifecycle("workspace ownership record is missing"))?;
        if record.intent.provider_scope != self.provider_scope
            || record.intent.roots_identity != self.roots_identity()?
        {
            return Err(RunError::Lifecycle("workspace recovery scope mismatch"));
        }
        if record.resource.is_none() {
            if record.state != WorkspaceState::CreateAttempted {
                return self.block(&record, "workspace has no recoverable provider attempt");
            }
            let directory = self
                .owned_directory(&record)?
                .ok_or(RunError::Lifecycle("workspace directory is missing"))?;
            directory.tree()?.ensure_attached()?;
            let receipt = match provider.discover(&record.intent) {
                Ok(WorkspaceDiscovery::Found(receipt)) => receipt,
                _ => return self.block(&record, "workspace discovery remains unresolved"),
            };
            directory.ensure_attached()?;
            if receipt.resource.intent != record.intent
                || receipt.resource.provider_identity.is_empty()
                || receipt.status == WorkspaceProviderStatus::Unknown
            {
                return self.block(&record, "workspace discovery ownership or state is unknown");
            }
            record = self.bind_resource(&record, receipt.resource)?;
            if receipt.status == WorkspaceProviderStatus::Stopped {
                record = self.transition(&record, WorkspaceState::Stopped)?;
            } else {
                self.publish_marker(&directory, &self.handle_from_record(&record)?)?;
            }
        }
        let handle = self.handle_from_record(&record)?;
        // Interrupted publication may be finished only from the exact durable bind.
        if record.state == WorkspaceState::Bound {
            return self.replay_workspace(provider, &record);
        }
        self.validated_record_directory(&record, &handle)?;
        Ok(handle)
    }

    pub fn status<W: WorkspaceProvider>(
        &self,
        workspace_provider: &mut W,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceStatus, RunError> {
        let _operation = self.enter()?;
        self.check_provider(workspace_provider)?;
        let record = self.record_for_handle(handle)?;
        let directory = self.validated_record_directory(&record, handle)?;
        let resource = record
            .resource
            .as_ref()
            .ok_or(RunError::Lifecycle("workspace is unbound"))?;
        let receipt = workspace_provider.status(resource)?;
        self.revalidate_record_directory(&record, handle, directory.as_ref())?;
        validate_receipt(resource, &receipt.resource)?;
        match receipt.status {
            WorkspaceProviderStatus::Active
                if record.state == WorkspaceState::Bound && directory.is_some() =>
            {
                Ok(WorkspaceStatus::Active)
            }
            WorkspaceProviderStatus::Stopped => {
                if record.state != WorkspaceState::Stopped {
                    self.transition(&record, WorkspaceState::Stopped)?;
                }
                Ok(WorkspaceStatus::Stopped)
            }
            _ => self.block(
                &record,
                "workspace provider state is unknown or inconsistent",
            ),
        }
    }

    pub fn stop<W: WorkspaceProvider>(
        &self,
        workspace_provider: &mut W,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceStopReceipt, RunError> {
        let _operation = self.enter()?;
        self.check_provider(workspace_provider)?;
        let mut record = self.record_for_handle(handle)?;
        let directory = self.validated_record_directory(&record, handle)?;
        let resource = record
            .resource
            .clone()
            .ok_or(RunError::Lifecycle("workspace is unbound"))?;
        let mut already_stopped = record.state == WorkspaceState::Stopped;
        if !already_stopped {
            let receipt = workspace_provider.status(&resource)?;
            self.revalidate_record_directory(&record, handle, directory.as_ref())?;
            validate_receipt(&resource, &receipt.resource)?;
            match receipt.status {
                WorkspaceProviderStatus::Stopped => {
                    already_stopped = true;
                }
                WorkspaceProviderStatus::Active
                    if record.state == WorkspaceState::Bound && directory.is_some() =>
                {
                    record = self.transition(&record, WorkspaceState::StopAttempted)?;
                    let receipt = match workspace_provider.stop(&resource) {
                        Ok(receipt) => receipt,
                        Err(_) => return self.block(&record, "workspace stop outcome is unknown"),
                    };
                    if validate_receipt(&resource, &receipt.resource).is_err() || !receipt.stopped {
                        return self.block(
                            &record,
                            "workspace stop receipt does not match owned identity",
                        );
                    }
                }
                _ => {
                    return self.block(
                        &record,
                        "workspace stop requires definitive ownership reconciliation",
                    )
                }
            }
            // Provider stop is durable even if subsequent filesystem checks fail.
            record = self.transition(&record, WorkspaceState::Stopped)?;
        }
        self.revalidate_record_directory(&record, handle, directory.as_ref())?;
        if let Some(directory) = directory {
            if directory.open_file(WORKSPACE_MARKER.as_ref())?.is_some() {
                directory.tree()?.remove(WORKSPACE_MARKER.as_ref())?;
            } else {
                directory.remove_empty()?;
            }
        }
        Ok(WorkspaceStopReceipt {
            run_id: handle.run_id.clone(),
            generation: handle.generation,
            workspace_provider_identity: resource.provider_identity,
            already_stopped,
        })
    }

    fn cache_verified_source(
        &self,
        binding: &TrustedLocalSourceBinding,
        acquired: AcquiredSource,
    ) -> Result<VerifiedSource, RunError> {
        // Recheck after the provider callback. Existing partial/corrupt storage is
        // never deleted or completed by this acquisition path.
        if let Some(source) = self.load_cached_source(binding)? {
            return Ok(source);
        }
        let cache_path = self.cache_path(&binding.expected_raw_digest)?;
        let new_blob = self
            .cache_root
            .open_file(file_name(&cache_path)?)?
            .is_none();
        if new_blob {
            self.cache_root
                .write_new(file_name(&cache_path)?, &acquired.bytes, true)?;
        }
        let receipt = SourceBindingReceipt {
            schema_version: "fkst.local-qa-source-binding/v1".to_owned(),
            binding: binding.clone(),
            source_object_id: acquired.source_object_id,
            raw_digest: sha256_digest(&acquired.bytes),
            immutable_revision: acquired.immutable_revision,
            provider_scope: acquired.provider_scope,
            provider_identity: acquired.provider_identity,
        };
        self.cache_root.write_new(
            file_name(&self.binding_receipt_path(binding)?)?,
            &encode_cache(&receipt)?,
            false,
        )?;
        if new_blob {
            // Initial completion follows the receipt: interrupted creation remains
            // partial, rather than appearing to be reusable byte-only storage.
            let marker = SourceCacheMarker {
                schema_version: "fkst.local-qa-source-cache/v2".to_owned(),
                content_digest: binding.expected_raw_digest.clone(),
            };
            self.cache_root.write_new(
                file_name(&cache_marker_path(&cache_path))?,
                &encode_cache(&marker)?,
                false,
            )?;
        }
        self.load_cached_source(binding)?.ok_or(RunError::Lifecycle(
            "source binding receipt publication is incomplete",
        ))
    }

    fn binding_receipt_path(
        &self,
        binding: &TrustedLocalSourceBinding,
    ) -> Result<PathBuf, RunError> {
        let digest = sha256_digest(&encode_cache(binding)?);
        Ok(self.cache_root.path().join(format!(
            "{}.binding.json",
            digest.strip_prefix("sha256:").expect("SHA-256 prefix")
        )))
    }

    fn load_cached_source(
        &self,
        binding: &TrustedLocalSourceBinding,
    ) -> Result<Option<VerifiedSource>, RunError> {
        let cache_path = self.cache_path(&binding.expected_raw_digest)?;
        let marker_path = cache_marker_path(&cache_path);
        let binding_receipt = self
            .cache_root
            .open_file(file_name(&self.binding_receipt_path(binding)?)?)?;
        match (
            self.cache_root.open_file(file_name(&cache_path)?)?,
            self.cache_root.open_file(file_name(&marker_path)?)?,
        ) {
            (None, None) if binding_receipt.is_none() => Ok(None),
            (Some(blob), Some(marker_file)) => {
                verify_file_digest(&blob, &binding.expected_raw_digest)?;
                let marker = read_cache_marker(&marker_file)?;
                if marker.content_digest != binding.expected_raw_digest {
                    return Err(RunError::Lifecycle("source cache byte metadata mismatch"));
                }
                let Some(binding_receipt) = binding_receipt else {
                    // Verified byte storage is reusable, but is not a source binding.
                    // Each distinct binding must freshly acquire and check its facts.
                    return Ok(None);
                };
                validate_binding_receipt(&binding_receipt, binding)?;
                Ok(Some(VerifiedSource {
                    source_object_id: binding.source_object_id.clone(),
                    content_digest: binding.expected_raw_digest.clone(),
                    immutable_revision: binding.expected_revision.clone(),
                    provider_identity: binding.expected_provider_identity.clone(),
                    cache_path,
                    blob,
                    marker: marker_file,
                    binding_receipt,
                    binding: binding.clone(),
                }))
            }
            _ => Err(RunError::Lifecycle("verified source cache is incomplete")),
        }
    }

    fn materialize_workspace<W: WorkspaceProvider>(
        &self,
        workspace_provider: &mut W,
        lease: &SourceObjectLease,
        request: &WorkspaceRequest,
        source: &VerifiedSource,
        clock: &impl Clock,
    ) -> Result<WorkspaceHandle, RunError> {
        ensure_before_deadline(clock, &request.deadline_utc)?;
        source.validate()?;
        let intent = WorkspaceIntent {
            source_binding: Some(lease.binding.clone()),
            stable_key: stable_workspace_key(&request.run_id, request.generation),
            run_id: request.run_id.clone(),
            generation: request.generation,
            source_lease_id: lease.lease_id.clone(),
            source_object_id: source.source_object_id.clone(),
            source_digest: source.content_digest.clone(),
            immutable_revision: source.immutable_revision.clone(),
            source_provider_identity: source.provider_identity.clone(),
            deadline_utc: request.deadline_utc.clone(),
            relative_location: format!("{}/generation-{}", request.run_id, request.generation),
            roots_identity: self.roots_identity()?,
            provider_scope: self.provider_scope.clone(),
        };
        let mut record = self.with_journal(|journal| journal.prepare_workspace(&intent))?;
        if record.resource.is_some() {
            let handle = self.replay_workspace(workspace_provider, &record)?;
            let directory = self.validated_record_directory(&record, &handle)?;
            ensure_before_deadline(clock, &intent.deadline_utc)?;
            source.validate()?;
            self.revalidate_record_directory(&record, &handle, directory.as_ref())?;
            return Ok(handle);
        }
        let root = self.workspace_path(&request.run_id, request.generation)?;
        if record.state == WorkspaceState::Prepared {
            if self
                .workspace_directory(&request.run_id, request.generation, false)?
                .is_some()
            {
                return self.block(&record, "unowned workspace directory already exists");
            }
            record = self.transition(&record, WorkspaceState::DirectoryAttempted)?;
            let run_root = self
                .workspace_root
                .child(request.run_id.as_ref(), true)?
                .ok_or(RunError::Lifecycle("workspace Run directory is missing"))?;
            let directory = run_root.create_child(file_name(&root)?)?;
            let mut after = record.clone();
            after.directory_identity = Some(directory.identity()?);
            after.state = WorkspaceState::DirectoryReady;
            record = self.with_journal(|journal| journal.update_workspace(&record, &after))?;
        }
        if record.state == WorkspaceState::DirectoryAttempted {
            return self.block(&record, "workspace directory creation outcome is unknown");
        }
        let directory = self
            .owned_directory(&record)?
            .ok_or(RunError::Lifecycle("owned workspace directory is missing"))?;
        directory.tree()?.ensure_attached()?;
        let discovery = match workspace_provider.discover(&intent) {
            Ok(discovery) => discovery,
            Err(_) => return self.block(&record, "workspace discovery failed"),
        };
        directory.ensure_attached()?;
        match discovery {
            WorkspaceDiscovery::Found(receipt)
                if record.state == WorkspaceState::CreateAttempted =>
            {
                if receipt.resource.intent != intent
                    || receipt.resource.provider_identity.is_empty()
                {
                    return self.block(&record, "workspace discovery ownership conflict");
                }
                record = self.bind_resource(&record, receipt.resource)?;
                if receipt.status == WorkspaceProviderStatus::Stopped {
                    self.transition(&record, WorkspaceState::Stopped)?;
                    return Err(RunError::Lifecycle(
                        "recovered workspace provider is stopped",
                    ));
                }
                if receipt.status != WorkspaceProviderStatus::Active {
                    return self.block(&record, "recovered workspace provider state is unknown");
                }
            }
            WorkspaceDiscovery::Absent
                if record.state == WorkspaceState::DirectoryReady && record.blocker.is_none() =>
            {
                ensure_before_deadline(clock, &intent.deadline_utc)?;
                source.validate()?;
                directory.ensure_attached()?;
                record = self.transition(&record, WorkspaceState::CreateAttempted)?;
                // Providers receive paths and must protect their own accesses. No
                // Journal borrow or transaction survives this external boundary.
                let receipt = match workspace_provider.materialize(
                    &intent,
                    &source.cache_path,
                    &root,
                    &source.immutable_revision,
                ) {
                    Ok(receipt) => receipt,
                    Err(_) => return self.block(&record, "workspace create outcome is unknown"),
                };
                if receipt.resource.intent != intent
                    || receipt.resource.provider_identity.is_empty()
                {
                    return self.block(&record, "workspace create receipt ownership conflict");
                }
                record = self.bind_resource(&record, receipt.resource)?;
            }
            _ => return self.block(&record, "workspace discovery does not authorize creation"),
        }
        directory.ensure_attached()?;
        source.validate()?;
        let handle = self.handle_from_record(&record)?;
        self.publish_marker(&directory, &handle)?;
        // Bind and publish obtained ownership before observing the Host clock.
        // A late return (or clock failure) must retain recover/status/stop authority.
        ensure_before_deadline(clock, &intent.deadline_utc)?;
        source.validate()?;
        self.revalidate_record_directory(&record, &handle, Some(&directory))?;
        Ok(handle)
    }

    fn bind_resource(
        &self,
        record: &OwnedWorkspace,
        resource: WorkspaceResource,
    ) -> Result<OwnedWorkspace, RunError> {
        let mut after = record.clone();
        after.resource = Some(resource);
        after.state = WorkspaceState::Bound;
        after.blocker = None;
        self.with_journal(|journal| journal.update_workspace(record, &after))
    }

    fn replay_workspace(
        &self,
        provider: &mut impl WorkspaceProvider,
        record: &OwnedWorkspace,
    ) -> Result<WorkspaceHandle, RunError> {
        if record.state != WorkspaceState::Bound {
            return Err(RunError::Lifecycle("workspace is not usable"));
        }
        let handle = self.handle_from_record(record)?;
        let directory = self
            .owned_directory(record)?
            .ok_or(RunError::Lifecycle("workspace directory is missing"))?;
        directory.tree()?.ensure_attached()?;
        if directory.open_file(WORKSPACE_MARKER.as_ref())?.is_some() {
            validate_marker(&handle, &read_marker(&directory)?)?;
        }
        let resource = record
            .resource
            .as_ref()
            .ok_or(RunError::Lifecycle("workspace is unbound"))?;
        let receipt = provider.status(resource)?;
        directory.ensure_attached()?;
        self.with_journal(|_| Ok(()))?;
        validate_receipt(resource, &receipt.resource)?;
        if receipt.status == WorkspaceProviderStatus::Stopped {
            self.transition(record, WorkspaceState::Stopped)?;
            return Err(RunError::Lifecycle("workspace provider is stopped"));
        }
        if receipt.status != WorkspaceProviderStatus::Active {
            return self.block(record, "workspace provider state is unknown");
        }
        // Check before publication and again at the final observation point.
        if self.record_for_handle(&handle)? != *record {
            return Err(RunError::Lifecycle("workspace changed concurrently"));
        }
        self.publish_marker(&directory, &handle)?;
        self.revalidate_record_directory(record, &handle, Some(&directory))?;
        Ok(handle)
    }

    fn publish_marker(
        &self,
        directory: &Arc<Directory>,
        handle: &WorkspaceHandle,
    ) -> Result<(), RunError> {
        if directory.open_file(WORKSPACE_MARKER.as_ref())?.is_some() {
            directory.tree()?.ensure_attached()?;
            validate_marker(handle, &read_marker(directory)?)
        } else {
            directory.tree_with_reserved_entries(1)?.ensure_attached()?;
            write_marker(directory, handle)
        }
    }

    fn roots_identity(&self) -> Result<String, RunError> {
        let identities = self
            .exposed_roots
            .iter()
            .map(|root| root.identity())
            .collect::<Result<Vec<_>, _>>()?;
        Ok(sha256_digest(identities.join(";").as_bytes()))
    }

    fn handle_from_record(&self, record: &OwnedWorkspace) -> Result<WorkspaceHandle, RunError> {
        let intent = &record.intent;
        if intent.provider_scope != self.provider_scope
            || intent.roots_identity != self.roots_identity()?
        {
            return Err(RunError::Lifecycle(
                "workspace storage or provider scope changed",
            ));
        }
        let resource = record
            .resource
            .as_ref()
            .ok_or(RunError::Lifecycle("workspace is unbound"))?;
        if resource.intent != *intent || resource.provider_identity.is_empty() {
            return Err(RunError::InvalidJournal(
                "workspace resource identity mismatch",
            ));
        }
        Ok(WorkspaceHandle {
            source_binding: intent.source_binding.clone(),
            run_id: intent.run_id.clone(),
            generation: intent.generation,
            source_object_id: intent.source_object_id.clone(),
            source_digest: intent.source_digest.clone(),
            immutable_revision: intent.immutable_revision.clone(),
            source_provider_identity: intent.source_provider_identity.clone(),
            workspace_provider_identity: resource.provider_identity.clone(),
            root: self.workspace_path(&intent.run_id, intent.generation)?,
        })
    }

    fn record_for_handle(&self, handle: &WorkspaceHandle) -> Result<OwnedWorkspace, RunError> {
        let record = self
            .with_journal(|journal| {
                journal.workspace(&stable_workspace_key(&handle.run_id, handle.generation))
            })?
            .ok_or(RunError::Lifecycle("workspace ownership record is missing"))?;
        if self.handle_from_record(&record)? != *handle {
            return Err(RunError::Lifecycle(
                "workspace handle does not match authoritative record",
            ));
        }
        Ok(record)
    }

    fn owned_directory(&self, record: &OwnedWorkspace) -> Result<Option<Arc<Directory>>, RunError> {
        let directory =
            self.workspace_directory(&record.intent.run_id, record.intent.generation, false)?;
        if let Some(directory) = &directory {
            if record.directory_identity.as_ref() != Some(&directory.identity()?) {
                return Err(RunError::Lifecycle(
                    "workspace directory identity differs from journal",
                ));
            }
        }
        Ok(directory)
    }

    fn validated_record_directory(
        &self,
        record: &OwnedWorkspace,
        handle: &WorkspaceHandle,
    ) -> Result<Option<Arc<Directory>>, RunError> {
        let directory = self.owned_directory(record)?;
        if let Some(directory) = &directory {
            directory.tree()?.ensure_attached()?;
            if directory.open_file(WORKSPACE_MARKER.as_ref())?.is_some() {
                validate_marker(handle, &read_marker(directory)?)?;
            } else if record.state != WorkspaceState::Stopped {
                return Err(RunError::Lifecycle(
                    "workspace marker is missing before recorded stop",
                ));
            }
        } else if record.state != WorkspaceState::Stopped {
            return Err(RunError::Lifecycle(
                "workspace directory is missing before recorded stop",
            ));
        }
        Ok(directory)
    }

    fn revalidate_record_directory(
        &self,
        record: &OwnedWorkspace,
        handle: &WorkspaceHandle,
        directory: Option<&Arc<Directory>>,
    ) -> Result<(), RunError> {
        // This post-callback read is the ownership observation's linearization
        // point. Later concurrent changes are not excluded by a returned status.
        if self.record_for_handle(handle)? != *record {
            return Err(RunError::Lifecycle(
                "workspace changed across provider callback",
            ));
        }
        if let Some(directory) = directory {
            directory.ensure_attached()?;
        }
        if self.validated_record_directory(record, handle)?.is_some() != directory.is_some() {
            return Err(RunError::Lifecycle(
                "workspace appeared across provider boundary",
            ));
        }
        Ok(())
    }

    fn cache_path(&self, digest: &str) -> Result<PathBuf, RunError> {
        validate_scalar("Sha256", digest)
            .map_err(|_| RunError::Lifecycle("source digest must be SHA-256"))?;
        let digest_value = digest
            .strip_prefix("sha256:")
            .ok_or(RunError::Lifecycle("source digest must be SHA-256"))?;
        Ok(self
            .cache_root
            .path()
            .join(format!("{digest_value}.source")))
    }

    fn workspace_path(&self, run_id: &str, generation: i64) -> Result<PathBuf, RunError> {
        validate_scalar("UUID", run_id)
            .map_err(|_| RunError::Lifecycle("workspace Run ID must be a canonical UUID"))?;
        if generation <= 0 {
            return Err(RunError::Lifecycle("workspace generation must be positive"));
        }
        Ok(self
            .workspace_root
            .path()
            .join(run_id)
            .join(format!("generation-{generation}")))
    }

    fn workspace_directory(
        &self,
        run_id: &str,
        generation: i64,
        create: bool,
    ) -> Result<Option<Arc<Directory>>, RunError> {
        let path = self.workspace_path(run_id, generation)?;
        let Some(run_root) = self.workspace_root.child(run_id.as_ref(), create)? else {
            return Ok(None);
        };
        run_root.child(file_name(&path)?, create)
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct WorkspaceMarker {
    schema_version: String,
    run_id: String,
    generation: i64,
    source_object_id: String,
    source_digest: String,
    immutable_revision: String,
    source_provider_identity: String,
    workspace_provider_identity: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceCacheMarker {
    schema_version: String,
    content_digest: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct SourceBindingReceipt {
    schema_version: String,
    binding: TrustedLocalSourceBinding,
    source_object_id: String,
    raw_digest: String,
    immutable_revision: ImmutableRevision,
    provider_scope: String,
    provider_identity: String,
}

impl WorkspaceMarker {
    fn from_handle(handle: &WorkspaceHandle) -> Self {
        Self {
            schema_version: "fkst.local-qa-workspace/v1".to_owned(),
            run_id: handle.run_id.clone(),
            generation: handle.generation,
            source_object_id: handle.source_object_id.clone(),
            source_digest: handle.source_digest.clone(),
            immutable_revision: handle.immutable_revision.marker_value(),
            source_provider_identity: handle.source_provider_identity.clone(),
            workspace_provider_identity: handle.workspace_provider_identity.clone(),
        }
    }
}

fn file_name(path: &Path) -> Result<&std::ffi::OsStr, RunError> {
    path.file_name()
        .ok_or(RunError::Lifecycle("lifecycle file name is missing"))
}

fn validate_binding(
    source_reference: &DigestBoundReferenceV2,
    lease: &SourceObjectLease,
    request: &WorkspaceRequest,
    clock: &impl Clock,
) -> Result<(), RunError> {
    validate_source_expectations(&lease.binding)?;
    if source_reference != &lease.binding.reference {
        return Err(RunError::Lifecycle(
            "source reference does not match trusted local binding",
        ));
    }
    validate_scalar("UUID", &request.run_id)
        .map_err(|_| RunError::Lifecycle("workspace Run ID must be a canonical UUID"))?;
    if lease.lease_id.is_empty()
        || lease.generation <= 0
        || request.generation <= 0
        || lease.run_id != request.run_id
        || lease.generation != request.generation
        || lease.deadline_utc != request.deadline_utc
    {
        return Err(RunError::Lifecycle(
            "SourceObject lease does not match the admitted source and Run",
        ));
    }
    ensure_before_deadline(clock, &request.deadline_utc)
}

// The pinned admission-v2 DigestBoundReference shape; this does not resolve the
// referenced schema or grant execution authority to an unregistered schema name.
fn valid_reference_atom(value: &str, punctuation: &[u8]) -> bool {
    value.len() <= 128
        && value
            .as_bytes()
            .first()
            .is_some_and(u8::is_ascii_alphanumeric)
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || punctuation.contains(&byte))
}

fn validate_source_expectations(binding: &TrustedLocalSourceBinding) -> Result<(), RunError> {
    let reference = &binding.reference;
    if reference.kind != "source"
        || !valid_reference_atom(&reference.id, b"._:-")
        || !valid_reference_atom(&reference.schema_version, b"._/-")
        || binding.source_object_id.trim().is_empty()
        || binding.expected_provider_scope.trim().is_empty()
        || binding.expected_provider_identity.trim().is_empty()
    {
        return Err(RunError::Lifecycle(
            "trusted local source binding is incomplete",
        ));
    }
    for digest in [&reference.content_digest, &binding.expected_raw_digest] {
        validate_scalar("Sha256", digest)
            .map_err(|_| RunError::Lifecycle("source binding digests must be SHA-256"))?;
    }
    binding.expected_revision.validate()
}

fn validate_declared_source(
    binding: &TrustedLocalSourceBinding,
    object_id: &str,
    revision: &ImmutableRevision,
    provider_scope: &str,
    provider_identity: &str,
    raw_digest: &str,
) -> Result<(), RunError> {
    validate_source_expectations(binding)?;
    revision.validate()?;
    if object_id != binding.source_object_id
        || revision != &binding.expected_revision
        || provider_scope != binding.expected_provider_scope
        || provider_identity != binding.expected_provider_identity
        || raw_digest != binding.expected_raw_digest
    {
        return Err(RunError::Lifecycle(
            "source facts do not match trusted local binding",
        ));
    }
    Ok(())
}

fn validate_acquired(
    binding: &TrustedLocalSourceBinding,
    acquired: &AcquiredSource,
) -> Result<(), RunError> {
    validate_declared_source(
        binding,
        &acquired.source_object_id,
        &acquired.immutable_revision,
        &acquired.provider_scope,
        &acquired.provider_identity,
        &sha256_digest(&acquired.bytes),
    )
}

fn validate_binding_receipt(
    file: &PinnedFile,
    binding: &TrustedLocalSourceBinding,
) -> Result<(), RunError> {
    let receipt: SourceBindingReceipt = serde_json::from_slice(&file.bytes()?)
        .map_err(|_| RunError::Lifecycle("source binding receipt is invalid"))?;
    if receipt.schema_version != "fkst.local-qa-source-binding/v1" || receipt.binding != *binding {
        return Err(RunError::Lifecycle(
            "source binding receipt ownership mismatch",
        ));
    }
    validate_declared_source(
        binding,
        &receipt.source_object_id,
        &receipt.immutable_revision,
        &receipt.provider_scope,
        &receipt.provider_identity,
        &receipt.raw_digest,
    )
}

fn encode_cache(value: &impl Serialize) -> Result<Vec<u8>, RunError> {
    serde_json::to_vec(value).map_err(|_| RunError::Lifecycle("source cache serialization failed"))
}

fn ensure_before_deadline(clock: &impl Clock, deadline_utc: &str) -> Result<(), RunError> {
    validate_scalar("ISO8601", deadline_utc)
        .map_err(|_| RunError::Lifecycle("lifecycle deadline must be ISO8601"))?;
    let now_utc = clock.now_utc()?;
    if !compare_iso8601_timestamps(&now_utc, deadline_utc)
        .map_err(|_| RunError::Lifecycle("lifecycle clock must return ISO8601"))?
        .is_lt()
    {
        return Err(RunError::Lifecycle("lifecycle deadline expired"));
    }
    Ok(())
}

fn cache_marker_path(cache_path: &Path) -> PathBuf {
    cache_path.with_extension("source.json")
}

fn read_cache_marker(file: &PinnedFile) -> Result<SourceCacheMarker, RunError> {
    let marker = serde_json::from_slice::<SourceCacheMarker>(&file.bytes()?)
        .map_err(|_| RunError::Lifecycle("source cache marker is invalid"))?;
    if marker.schema_version != "fkst.local-qa-source-cache/v2" {
        return Err(RunError::Lifecycle(
            "source cache marker schema is unsupported",
        ));
    }
    Ok(marker)
}

fn verify_file_digest(file: &PinnedFile, expected_digest: &str) -> Result<(), RunError> {
    let bytes = file.bytes()?;
    if sha256_digest(&bytes) != expected_digest {
        return Err(RunError::Lifecycle("verified source cache is corrupt"));
    }
    Ok(())
}

fn write_marker(directory: &Arc<Directory>, handle: &WorkspaceHandle) -> Result<(), RunError> {
    let bytes = serde_json::to_vec(&WorkspaceMarker::from_handle(handle))
        .map_err(|_| RunError::Lifecycle("workspace marker serialization failed"))?;
    directory.write_new(WORKSPACE_MARKER.as_ref(), &bytes, false)
}

fn read_marker(directory: &Arc<Directory>) -> Result<WorkspaceMarker, RunError> {
    let file = directory
        .open_file(WORKSPACE_MARKER.as_ref())?
        .ok_or(RunError::Lifecycle("workspace marker is missing"))?;
    let bytes = file.bytes()?;
    let marker = serde_json::from_slice::<WorkspaceMarker>(&bytes)
        .map_err(|_| RunError::Lifecycle("workspace marker is invalid"))?;
    if marker.schema_version != "fkst.local-qa-workspace/v1" {
        return Err(RunError::Lifecycle(
            "workspace marker schema is unsupported",
        ));
    }
    Ok(marker)
}

fn validate_marker(handle: &WorkspaceHandle, marker: &WorkspaceMarker) -> Result<(), RunError> {
    if marker.run_id != handle.run_id
        || marker.generation != handle.generation
        || marker.source_object_id != handle.source_object_id
        || marker.source_digest != handle.source_digest
        || marker.immutable_revision != handle.immutable_revision.marker_value()
        || marker.source_provider_identity != handle.source_provider_identity
        || marker.workspace_provider_identity != handle.workspace_provider_identity
    {
        return Err(RunError::Lifecycle(
            "workspace marker does not match the owned handle",
        ));
    }
    Ok(())
}

pub fn stable_workspace_key(run_id: &str, generation: i64) -> String {
    format!("fkst-local-qa/workspace/v1/{run_id}/{generation}")
}

fn validate_receipt(
    expected: &WorkspaceResource,
    actual: &WorkspaceResource,
) -> Result<(), RunError> {
    if expected != actual {
        return Err(RunError::Lifecycle("workspace receipt ownership mismatch"));
    }
    Ok(())
}

fn path_is_relative_and_confined(path: &Path) -> bool {
    path.is_relative()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_) | Component::CurDir))
}

pub fn validate_controlled_relative_path(path: &Path) -> Result<(), RunError> {
    if !path_is_relative_and_confined(path) {
        return Err(RunError::Lifecycle(
            "workspace materialization path is not confined",
        ));
    }
    Ok(())
}

pub fn validate_loopback_endpoint(endpoint: SocketAddr) -> Result<(), RunError> {
    if !endpoint.ip().is_loopback() || endpoint.port() == 0 {
        return Err(RunError::Lifecycle(
            "readiness endpoint must be an allocated loopback port",
        ));
    }
    Ok(())
}
