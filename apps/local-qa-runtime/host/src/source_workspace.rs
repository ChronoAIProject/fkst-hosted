use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

#[path = "source_workspace_fs.rs"]
mod filesystem;
use filesystem::{Directory, PinnedFile};

use fkst_qa_contracts::{sha256_digest, validate_scalar, DigestBoundReferenceV2};
use serde::{Deserialize, Serialize};

use crate::ownership::Clock;
use crate::RunError;

const SOURCE_KIND: &str = "source";
const SOURCE_SCHEMA_VERSION: &str = "qa.source/v1";
const WORKSPACE_MARKER: &str = ".fkst-workspace.json";

#[derive(Debug, Clone, PartialEq, Eq)]
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceMaterialization {
    pub provider_identity: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkspaceProviderStatus {
    Active,
    Stopped,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceProviderStopReceipt {
    pub provider_identity: String,
    pub stopped: bool,
}

pub trait WorkspaceProvider {
    fn materialize(
        &mut self,
        verified_source_blob: &Path,
        workspace_root: &Path,
        immutable_revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError>;

    fn status(&mut self, provider_identity: &str) -> Result<WorkspaceProviderStatus, RunError>;

    fn stop(&mut self, provider_identity: &str) -> Result<WorkspaceProviderStopReceipt, RunError>;
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
    pub run_id: String,
    pub generation: i64,
    pub source_object_id: String,
    pub source_digest: String,
    pub immutable_revision: ImmutableRevision,
    pub source_provider_identity: String,
    pub workspace_provider_identity: String,
    pub root: PathBuf,
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

pub struct SourceWorkspaceManager {
    cache_root: Arc<Directory>,
    workspace_root: Arc<Directory>,
}

impl SourceWorkspaceManager {
    pub fn new(
        cache_root: impl AsRef<Path>,
        workspace_root: impl AsRef<Path>,
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
        Ok(Self {
            cache_root: Directory::prepare(cache_root)?,
            workspace_root: Directory::prepare(workspace_root)?,
        })
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
        self.materialize_workspace(workspace_provider, request, &verified, clock)
    }

    pub fn status<W: WorkspaceProvider>(
        &self,
        workspace_provider: &mut W,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceStatus, RunError> {
        let directory = self.validated_workspace(handle)?;
        let workspace_exists = directory.is_some();
        let status = workspace_provider.status(&handle.workspace_provider_identity)?;
        self.revalidate_workspace(handle, directory.as_ref())?;
        match status {
            WorkspaceProviderStatus::Active if workspace_exists => Ok(WorkspaceStatus::Active),
            WorkspaceProviderStatus::Stopped if !workspace_exists => Ok(WorkspaceStatus::Stopped),
            WorkspaceProviderStatus::Active | WorkspaceProviderStatus::Stopped => Err(
                RunError::Lifecycle("workspace filesystem and provider status disagree"),
            ),
            WorkspaceProviderStatus::Unknown => Err(RunError::Lifecycle(
                "workspace provider ownership is unknown",
            )),
        }
    }

    pub fn stop<W: WorkspaceProvider>(
        &self,
        workspace_provider: &mut W,
        handle: &WorkspaceHandle,
    ) -> Result<WorkspaceStopReceipt, RunError> {
        let directory = self.validated_workspace(handle)?;
        let status = workspace_provider.status(&handle.workspace_provider_identity)?;
        self.revalidate_workspace(handle, directory.as_ref())?;
        let already_stopped = match status {
            WorkspaceProviderStatus::Active => {
                if directory.is_none() {
                    return Err(RunError::Lifecycle(
                        "active workspace has no owned filesystem marker",
                    ));
                }
                let receipt = workspace_provider.stop(&handle.workspace_provider_identity)?;
                if receipt.provider_identity != handle.workspace_provider_identity
                    || !receipt.stopped
                {
                    return Err(RunError::Lifecycle(
                        "workspace stop receipt does not match owned identity",
                    ));
                }
                false
            }
            WorkspaceProviderStatus::Stopped => true,
            WorkspaceProviderStatus::Unknown => {
                return Err(RunError::Lifecycle(
                    "workspace provider ownership is unknown",
                ));
            }
        };
        self.revalidate_workspace(handle, directory.as_ref())?;
        if let Some(directory) = directory {
            directory.tree()?.remove(WORKSPACE_MARKER.as_ref())?;
        }
        Ok(WorkspaceStopReceipt {
            run_id: handle.run_id.clone(),
            generation: handle.generation,
            workspace_provider_identity: handle.workspace_provider_identity.clone(),
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
        request: &WorkspaceRequest,
        source: &VerifiedSource,
        clock: &impl Clock,
    ) -> Result<WorkspaceHandle, RunError> {
        ensure_before_deadline(clock, &request.deadline_utc)?;
        source.blob.ensure_attached()?;
        source.marker.ensure_attached()?;
        verify_file_digest(&source.blob, &source.content_digest)?;
        let root = self.workspace_path(&request.run_id, request.generation)?;
        if let Some(directory) =
            self.workspace_directory(&request.run_id, request.generation, false)?
        {
            directory.tree()?.ensure_attached()?;
            let marker = read_marker(&directory)?;
            let handle = marker.to_handle(root);
            validate_replay(request, source, &handle)?;
            return Ok(handle);
        }

        let run_root = self
            .workspace_root
            .child(request.run_id.as_ref(), true)?
            .ok_or(RunError::Lifecycle("workspace Run directory is missing"))?;
        let directory = run_root.create_child(file_name(&root)?)?;
        source.blob.ensure_attached()?;
        source.marker.ensure_attached()?;
        verify_file_digest(&source.blob, &source.content_digest)?;
        directory.ensure_attached()?;
        // The provider receives paths, not descriptors. It must protect its own
        // filesystem accesses; these checks only bracket that external boundary.
        let materialization = workspace_provider.materialize(
            &source.cache_path,
            &root,
            &source.immutable_revision,
        )?;
        directory.ensure_attached()?;
        source.blob.ensure_attached()?;
        source.marker.ensure_attached()?;
        if materialization.provider_identity.is_empty() {
            return Err(RunError::Lifecycle(
                "workspace provider identity must not be empty",
            ));
        }
        // The ownership marker written below consumes one entry in the final tree.
        directory.tree_with_reserved_entries(1)?.ensure_attached()?;
        let handle = WorkspaceHandle {
            run_id: request.run_id.clone(),
            generation: request.generation,
            source_object_id: source.source_object_id.clone(),
            source_digest: source.content_digest.clone(),
            immutable_revision: source.immutable_revision.clone(),
            source_provider_identity: source.provider_identity.clone(),
            workspace_provider_identity: materialization.provider_identity,
            root,
        };
        write_marker(&directory, &handle)?;
        Ok(handle)
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

    fn validated_workspace(
        &self,
        handle: &WorkspaceHandle,
    ) -> Result<Option<Arc<Directory>>, RunError> {
        self.validate_handle_path(handle)?;
        let directory = self.workspace_directory(&handle.run_id, handle.generation, false)?;
        if let Some(directory) = &directory {
            directory.tree()?.ensure_attached()?;
            validate_marker(handle, &read_marker(directory)?)?;
        }
        Ok(directory)
    }

    fn revalidate_workspace(
        &self,
        handle: &WorkspaceHandle,
        directory: Option<&Arc<Directory>>,
    ) -> Result<(), RunError> {
        if let Some(directory) = directory {
            directory.ensure_attached()?;
            directory.tree()?.ensure_attached()?;
            validate_marker(handle, &read_marker(directory)?)?;
        } else if self
            .workspace_directory(&handle.run_id, handle.generation, false)?
            .is_some()
        {
            return Err(RunError::Lifecycle(
                "workspace appeared across provider boundary",
            ));
        }
        Ok(())
    }

    fn validate_handle_path(&self, handle: &WorkspaceHandle) -> Result<(), RunError> {
        if handle.root != self.workspace_path(&handle.run_id, handle.generation)? {
            return Err(RunError::Lifecycle(
                "workspace handle path is not Runtime-derived",
            ));
        }
        Ok(())
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

    fn to_handle(&self, root: PathBuf) -> WorkspaceHandle {
        WorkspaceHandle {
            run_id: self.run_id.clone(),
            generation: self.generation,
            source_object_id: self.source_object_id.clone(),
            source_digest: self.source_digest.clone(),
            immutable_revision: parse_marker_revision(&self.immutable_revision),
            source_provider_identity: self.source_provider_identity.clone(),
            workspace_provider_identity: self.workspace_provider_identity.clone(),
            root,
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
    validate_scalar("ISO8601", &now_utc)
        .map_err(|_| RunError::Lifecycle("lifecycle clock must return ISO8601"))?;
    if now_utc.as_str() >= deadline_utc {
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

fn validate_replay(
    request: &WorkspaceRequest,
    source: &VerifiedSource,
    handle: &WorkspaceHandle,
) -> Result<(), RunError> {
    if handle.run_id != request.run_id
        || handle.generation != request.generation
        || handle.source_object_id != source.source_object_id
        || handle.source_digest != source.content_digest
        || handle.immutable_revision != source.immutable_revision
        || handle.source_provider_identity != source.provider_identity
    {
        return Err(RunError::Lifecycle(
            "existing workspace does not match the same-Run source intent",
        ));
    }
    handle.immutable_revision.validate()?;
    if handle.workspace_provider_identity.is_empty() {
        return Err(RunError::Lifecycle(
            "workspace provider identity must not be empty",
        ));
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
