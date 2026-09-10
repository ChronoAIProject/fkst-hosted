use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::source_workspace::{
    lifecycle_authority_blockers, validate_controlled_relative_path, AcquiredSource,
    ImmutableRevision, LifecycleAuthorityBlocker, SourceObjectLease, SourceProvider,
    SourceWorkspaceManager, WorkspaceMaterialization, WorkspaceProvider,
    WorkspaceProviderStatus, WorkspaceProviderStopReceipt, WorkspaceRequest, WorkspaceStatus,
};
use fkst_local_qa_host::{FixedClock, RunError};
use fkst_qa_contracts::{sha256_digest, DigestBoundReferenceV2};

const NOW: &str = "2026-09-10T00:00:00Z";
const DEADLINE: &str = "2026-09-11T00:00:00Z";
const COMMIT: &str = "0123456789abcdef0123456789abcdef01234567";

struct FakeSourceProvider {
    bytes: Vec<u8>,
    revision: ImmutableRevision,
    acquire_calls: usize,
}

impl SourceProvider for FakeSourceProvider {
    fn acquire(&mut self, lease: &SourceObjectLease) -> Result<AcquiredSource, RunError> {
        self.acquire_calls += 1;
        Ok(AcquiredSource {
            source_object_id: lease.source_object_id.clone(),
            immutable_revision: self.revision.clone(),
            provider_identity: "source-provider/object-001".to_owned(),
            bytes: self.bytes.clone(),
        })
    }
}

#[derive(Default)]
struct FakeWorkspaceProvider {
    materialize_calls: usize,
    active: BTreeMap<String, bool>,
}

impl WorkspaceProvider for FakeWorkspaceProvider {
    fn materialize(
        &mut self,
        verified_source_blob: &Path,
        workspace_root: &Path,
        _immutable_revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError> {
        self.materialize_calls += 1;
        fs::write(
            workspace_root.join("source.bin"),
            fs::read(verified_source_blob)?,
        )?;
        let provider_identity = format!("workspace-provider:{}", workspace_root.display());
        self.active.insert(provider_identity.clone(), true);
        Ok(WorkspaceMaterialization { provider_identity })
    }

    fn status(
        &mut self,
        provider_identity: &str,
    ) -> Result<WorkspaceProviderStatus, RunError> {
        Ok(match self.active.get(provider_identity) {
            Some(true) => WorkspaceProviderStatus::Active,
            Some(false) => WorkspaceProviderStatus::Stopped,
            None => WorkspaceProviderStatus::Unknown,
        })
    }

    fn stop(
        &mut self,
        provider_identity: &str,
    ) -> Result<WorkspaceProviderStopReceipt, RunError> {
        let Some(active) = self.active.get_mut(provider_identity) else {
            return Ok(WorkspaceProviderStopReceipt {
                provider_identity: provider_identity.to_owned(),
                stopped: false,
            });
        };
        *active = false;
        Ok(WorkspaceProviderStopReceipt {
            provider_identity: provider_identity.to_owned(),
            stopped: true,
        })
    }
}

#[test]
fn exact_source_cache_and_run_scoped_workspaces_replay_without_duplicate_effects() {
    let root = temporary_root("source-workspace-replay");
    let cache_root = root.join("cache");
    let workspace_root = root.join("workspaces");
    let manager = SourceWorkspaceManager::new(&cache_root, &workspace_root).unwrap();
    let bytes = b"exact immutable source payload".to_vec();
    let reference = source_reference(&bytes);
    let clock = FixedClock::new(NOW).unwrap();
    let mut source_provider = FakeSourceProvider {
        bytes: bytes.clone(),
        revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
        acquire_calls: 0,
    };
    let mut workspace_provider = FakeWorkspaceProvider::default();

    let first_request = workspace_request("00000000-0000-4000-8000-000000000101");
    let first = manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &first_request),
            &first_request,
            &clock,
        )
        .unwrap();
    assert_eq!(fs::read(first.root.join("source.bin")).unwrap(), bytes);
    assert_eq!(source_provider.acquire_calls, 1);
    assert_eq!(workspace_provider.materialize_calls, 1);

    let replay = manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &first_request),
            &first_request,
            &clock,
        )
        .unwrap();
    assert_eq!(replay, first);
    assert_eq!(source_provider.acquire_calls, 1);
    assert_eq!(workspace_provider.materialize_calls, 1);

    let second_request = workspace_request("00000000-0000-4000-8000-000000000102");
    let second = manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &second_request),
            &second_request,
            &clock,
        )
        .unwrap();
    assert_ne!(first.root, second.root);
    assert_eq!(source_provider.acquire_calls, 1);
    assert_eq!(workspace_provider.materialize_calls, 2);
    assert_eq!(manager.status(&mut workspace_provider, &first).unwrap(), WorkspaceStatus::Active);

    fs::write(workspace_root.join("unrelated-resource"), b"preserve").unwrap();
    let stopped = manager.stop(&mut workspace_provider, &first).unwrap();
    assert!(!stopped.already_stopped);
    assert_eq!(manager.status(&mut workspace_provider, &first).unwrap(), WorkspaceStatus::Stopped);
    assert!(manager
        .stop(&mut workspace_provider, &first)
        .unwrap()
        .already_stopped);
    assert_eq!(
        fs::read(workspace_root.join("unrelated-resource")).unwrap(),
        b"preserve"
    );
    assert_eq!(manager.status(&mut workspace_provider, &second).unwrap(), WorkspaceStatus::Active);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn wrong_digest_floating_revision_and_corrupt_cache_fail_before_workspace_effects() {
    let root = temporary_root("source-workspace-fail-closed");
    let cache_root = root.join("cache");
    let workspace_root = root.join("workspaces");
    let manager = SourceWorkspaceManager::new(&cache_root, &workspace_root).unwrap();
    let bytes = b"exact immutable source payload".to_vec();
    let clock = FixedClock::new(NOW).unwrap();
    let request = workspace_request("00000000-0000-4000-8000-000000000201");
    let mut wrong_reference = source_reference(b"different payload");
    wrong_reference.id = "source-001".to_owned();
    let mut source_provider = FakeSourceProvider {
        bytes: bytes.clone(),
        revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
        acquire_calls: 0,
    };
    let mut workspace_provider = FakeWorkspaceProvider::default();
    assert!(manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &wrong_reference,
            &lease(&wrong_reference, &request),
            &request,
            &clock,
        )
        .is_err());
    assert_eq!(workspace_provider.materialize_calls, 0);
    assert_eq!(fs::read_dir(&workspace_root).unwrap().count(), 0);

    let reference = source_reference(&bytes);
    source_provider.revision = ImmutableRevision::GitCommit("main".to_owned());
    assert!(manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &request),
            &request,
            &clock,
        )
        .is_err());
    assert_eq!(workspace_provider.materialize_calls, 0);

    source_provider.revision = ImmutableRevision::GitCommit(COMMIT.to_owned());
    let first = manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &request),
            &request,
            &clock,
        )
        .unwrap();
    let cache_blob = fs::read_dir(&cache_root)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.extension().and_then(|extension| extension.to_str()) == Some("source"))
        .unwrap();
    fs::remove_file(&cache_blob).unwrap();
    fs::write(&cache_blob, b"corrupt").unwrap();

    let second_request = workspace_request("00000000-0000-4000-8000-000000000202");
    let acquire_calls = source_provider.acquire_calls;
    let materialize_calls = workspace_provider.materialize_calls;
    assert!(manager
        .prepare(
            &mut source_provider,
            &mut workspace_provider,
            &reference,
            &lease(&reference, &second_request),
            &second_request,
            &clock,
        )
        .is_err());
    assert_eq!(source_provider.acquire_calls, acquire_calls);
    assert_eq!(workspace_provider.materialize_calls, materialize_calls);
    assert!(!second_request_path(&workspace_root, &second_request).exists());
    fs::remove_dir_all(first.root).unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn missing_authority_and_unsafe_paths_are_explicit_blockers() {
    assert_eq!(
        lifecycle_authority_blockers(),
        [
            LifecycleAuthorityBlocker::MissingSourceObjectLeaseBinding,
            LifecycleAuthorityBlocker::MissingEnvironmentProviderProjection,
            LifecycleAuthorityBlocker::MissingReadinessReceiptMapping,
        ]
    );
    assert!(validate_controlled_relative_path(Path::new("project/src/main.rs")).is_ok());
    assert!(validate_controlled_relative_path(Path::new("../user-checkout")).is_err());
    assert!(validate_controlled_relative_path(Path::new("/home/user/project")).is_err());
}

fn source_reference(bytes: &[u8]) -> DigestBoundReferenceV2 {
    DigestBoundReferenceV2 {
        kind: "source".to_owned(),
        id: "source-001".to_owned(),
        schema_version: "qa.source/v1".to_owned(),
        content_digest: sha256_digest(bytes),
    }
}

fn workspace_request(run_id: &str) -> WorkspaceRequest {
    WorkspaceRequest {
        run_id: run_id.to_owned(),
        generation: 1,
        deadline_utc: DEADLINE.to_owned(),
    }
}

fn lease(reference: &DigestBoundReferenceV2, request: &WorkspaceRequest) -> SourceObjectLease {
    SourceObjectLease {
        lease_id: format!("lease-{}", request.run_id),
        source_object_id: reference.id.clone(),
        run_id: request.run_id.clone(),
        generation: request.generation,
        content_digest: reference.content_digest.clone(),
        deadline_utc: request.deadline_utc.clone(),
    }
}

fn second_request_path(workspace_root: &Path, request: &WorkspaceRequest) -> PathBuf {
    workspace_root
        .join(&request.run_id)
        .join(format!("generation-{}", request.generation))
}

fn temporary_root(name: &str) -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let path = std::env::temp_dir().join(format!("fkst-{name}-{}-{nonce}", std::process::id()));
    fs::create_dir(&path).unwrap();
    path
}
