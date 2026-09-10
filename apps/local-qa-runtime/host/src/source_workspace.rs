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

const SOURCE_KIND: &str = "source";
const SOURCE_SCHEMA_VERSION: &str = "qa.source/v1";
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceObjectLease {
    pub lease_id: String,
    pub source_object_id: String,
    pub run_id: String,
    pub generation: i64,
    pub content_digest: String,
    pub deadline_utc: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AcquiredSource {
    pub source_object_id: String,
    pub immutable_revision: ImmutableRevision,
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
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceHandle {
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
        self.workspace_directory(&request.run_id, request.generation, false)?;
        let verified = match self.load_cached_source(source_reference, lease)? {
            Some(verified) => verified,
            None => {
                let acquired = source_provider.acquire(lease)?;
                validate_acquired(source_reference, lease, &acquired)?;
                self.cache_verified_source(source_reference, acquired)?
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
        source_reference: &DigestBoundReferenceV2,
        acquired: AcquiredSource,
    ) -> Result<VerifiedSource, RunError> {
        let cache_path = self.cache_path(&source_reference.content_digest)?;
        self.cache_root
            .write_new(file_name(&cache_path)?, &acquired.bytes, true)?;
        let blob = self
            .cache_root
            .open_file(file_name(&cache_path)?)?
            .ok_or(RunError::Lifecycle("verified source cache is incomplete"))?;
        verify_file_digest(&blob, &source_reference.content_digest)?;
        let marker = SourceCacheMarker {
            schema_version: "fkst.local-qa-source-cache/v1".to_owned(),
            source_object_id: acquired.source_object_id.clone(),
            content_digest: source_reference.content_digest.clone(),
            immutable_revision: acquired.immutable_revision.marker_value(),
            provider_identity: acquired.provider_identity.clone(),
        };
        let marker_path = cache_marker_path(&cache_path);
        let bytes = serde_json::to_vec(&marker)
            .map_err(|_| RunError::Lifecycle("source cache marker serialization failed"))?;
        self.cache_root
            .write_new(file_name(&marker_path)?, &bytes, false)?;
        let marker = self
            .cache_root
            .open_file(file_name(&marker_path)?)?
            .ok_or(RunError::Lifecycle("verified source cache is incomplete"))?;
        Ok(VerifiedSource {
            source_object_id: acquired.source_object_id,
            content_digest: source_reference.content_digest.clone(),
            immutable_revision: acquired.immutable_revision,
            provider_identity: acquired.provider_identity,
            cache_path,
            blob,
            marker,
        })
    }

    fn load_cached_source(
        &self,
        source_reference: &DigestBoundReferenceV2,
        lease: &SourceObjectLease,
    ) -> Result<Option<VerifiedSource>, RunError> {
        let cache_path = self.cache_path(&source_reference.content_digest)?;
        let marker_path = cache_marker_path(&cache_path);
        match (
            self.cache_root.open_file(file_name(&cache_path)?)?,
            self.cache_root.open_file(file_name(&marker_path)?)?,
        ) {
            (None, None) => Ok(None),
            (Some(blob), Some(marker_file)) => {
                verify_file_digest(&blob, &source_reference.content_digest)?;
                let marker = read_cache_marker(&marker_file)?;
                let immutable_revision = parse_marker_revision(&marker.immutable_revision);
                immutable_revision.validate()?;
                if marker.source_object_id != lease.source_object_id
                    || marker.content_digest != source_reference.content_digest
                    || marker.provider_identity.is_empty()
                {
                    return Err(RunError::Lifecycle(
                        "verified source cache metadata does not match the lease",
                    ));
                }
                Ok(Some(VerifiedSource {
                    source_object_id: marker.source_object_id,
                    content_digest: marker.content_digest,
                    immutable_revision,
                    provider_identity: marker.provider_identity,
                    cache_path,
                    blob,
                    marker: marker_file,
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
        source.blob.ensure_attached()?;
        source.marker.ensure_attached()?;
        verify_file_digest(&source.blob, &source.content_digest)?;
        let intent = WorkspaceIntent {
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
                source.blob.ensure_attached()?;
                source.marker.ensure_attached()?;
                verify_file_digest(&source.blob, &source.content_digest)?;
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
        source.blob.ensure_attached()?;
        source.marker.ensure_attached()?;
        let handle = self.handle_from_record(&record)?;
        self.publish_marker(&directory, &handle)?;
        // Bind and publish obtained ownership before observing the Host clock.
        // A late return (or clock failure) must retain recover/status/stop authority.
        ensure_before_deadline(clock, &intent.deadline_utc)?;
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
struct SourceCacheMarker {
    schema_version: String,
    source_object_id: String,
    content_digest: String,
    immutable_revision: String,
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
    if source_reference.kind != SOURCE_KIND
        || source_reference.schema_version != SOURCE_SCHEMA_VERSION
    {
        return Err(RunError::Lifecycle(
            "source reference is not the approved immutable source type",
        ));
    }
    validate_scalar("Sha256", &source_reference.content_digest)
        .map_err(|_| RunError::Lifecycle("source reference digest must be SHA-256"))?;
    validate_scalar("UUID", &request.run_id)
        .map_err(|_| RunError::Lifecycle("workspace Run ID must be a canonical UUID"))?;
    if lease.lease_id.is_empty()
        || lease.source_object_id.is_empty()
        || lease.generation <= 0
        || request.generation <= 0
        || lease.source_object_id != source_reference.id
        || lease.run_id != request.run_id
        || lease.generation != request.generation
        || lease.content_digest != source_reference.content_digest
        || lease.deadline_utc != request.deadline_utc
    {
        return Err(RunError::Lifecycle(
            "SourceObject lease does not match the admitted source and Run",
        ));
    }
    ensure_before_deadline(clock, &request.deadline_utc)
}

fn validate_acquired(
    source_reference: &DigestBoundReferenceV2,
    lease: &SourceObjectLease,
    acquired: &AcquiredSource,
) -> Result<(), RunError> {
    acquired.immutable_revision.validate()?;
    if acquired.source_object_id != lease.source_object_id || acquired.provider_identity.is_empty()
    {
        return Err(RunError::Lifecycle(
            "acquired SourceObject identity does not match the lease",
        ));
    }
    if sha256_digest(&acquired.bytes) != source_reference.content_digest {
        return Err(RunError::Lifecycle(
            "acquired SourceObject digest does not match the admitted source",
        ));
    }
    Ok(())
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
    if marker.schema_version != "fkst.local-qa-source-cache/v1" {
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

fn parse_marker_revision(value: &str) -> ImmutableRevision {
    if let Some(commit) = value.strip_prefix("git:") {
        ImmutableRevision::GitCommit(commit.to_owned())
    } else if let Some(digest) = value.strip_prefix("object:") {
        ImmutableRevision::ObjectDigest(digest.to_owned())
    } else {
        ImmutableRevision::ObjectDigest(String::new())
    }
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
