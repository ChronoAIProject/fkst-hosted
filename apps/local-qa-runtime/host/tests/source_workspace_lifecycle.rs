use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::source_workspace::{
    lifecycle_authority_blockers, validate_controlled_relative_path, AcquiredSource,
    ImmutableRevision, LifecycleAuthorityBlocker, SourceObjectLease, SourceProvider,
    SourceWorkspaceManager, WorkspaceMaterialization, WorkspaceProvider, WorkspaceProviderStatus,
    WorkspaceProviderStopReceipt, WorkspaceRequest, WorkspaceStatus,
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
    stop_calls: usize,
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

    fn status(&mut self, provider_identity: &str) -> Result<WorkspaceProviderStatus, RunError> {
        Ok(match self.active.get(provider_identity) {
            Some(true) => WorkspaceProviderStatus::Active,
            Some(false) => WorkspaceProviderStatus::Stopped,
            None => WorkspaceProviderStatus::Unknown,
        })
    }

    fn stop(&mut self, provider_identity: &str) -> Result<WorkspaceProviderStopReceipt, RunError> {
        self.stop_calls += 1;
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
#[cfg_attr(
    not(unix),
    ignore = "confined workspaces are unsupported on this platform"
)]
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
    assert_eq!(
        manager.status(&mut workspace_provider, &first).unwrap(),
        WorkspaceStatus::Active
    );

    fs::write(workspace_root.join("unrelated-resource"), b"preserve").unwrap();
    let stopped = manager.stop(&mut workspace_provider, &first).unwrap();
    assert!(!stopped.already_stopped);
    assert_eq!(
        manager.status(&mut workspace_provider, &first).unwrap(),
        WorkspaceStatus::Stopped
    );
    assert!(
        manager
            .stop(&mut workspace_provider, &first)
            .unwrap()
            .already_stopped
    );
    assert_eq!(
        fs::read(workspace_root.join("unrelated-resource")).unwrap(),
        b"preserve"
    );
    assert_eq!(
        manager.status(&mut workspace_provider, &second).unwrap(),
        WorkspaceStatus::Active
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
#[cfg_attr(
    not(unix),
    ignore = "confined workspaces are unsupported on this platform"
)]
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

#[cfg(unix)]
#[test]
fn symlinked_run_parent_cannot_create_a_generation_outside_the_owned_root() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("source-workspace-parent-link");
    let workspace_root = root.join("workspaces");
    let manager = SourceWorkspaceManager::new(root.join("cache"), &workspace_root).unwrap();
    let outside = root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"preserve").unwrap();
    let request = workspace_request("00000000-0000-4000-8000-000000000301");
    symlink(&outside, workspace_root.join(&request.run_id)).unwrap();
    let bytes = b"source".to_vec();
    let reference = source_reference(&bytes);
    let mut source = FakeSourceProvider {
        bytes,
        revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
        acquire_calls: 0,
    };
    let mut provider = FakeWorkspaceProvider::default();
    let result = manager.prepare(
        &mut source,
        &mut provider,
        &reference,
        &lease(&reference, &request),
        &request,
        &FixedClock::new(NOW).unwrap(),
    );
    assert!(result.is_err(), "a symlinked Run parent must be rejected");
    assert_eq!(provider.materialize_calls, 0);
    assert_eq!(provider.stop_calls, 0);
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn valid_content_cache_symlink_is_rejected_before_provider_mutation() {
    use std::os::unix::fs::symlink;

    let root = temporary_root("source-workspace-cache-link");
    let cache_root = root.join("cache");
    let manager = SourceWorkspaceManager::new(&cache_root, root.join("workspaces")).unwrap();
    let bytes = b"source".to_vec();
    let reference = source_reference(&bytes);
    let request = workspace_request("00000000-0000-4000-8000-000000000302");
    let mut source = FakeSourceProvider {
        bytes: bytes.clone(),
        revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
        acquire_calls: 0,
    };
    let mut provider = FakeWorkspaceProvider::default();
    manager
        .prepare(
            &mut source,
            &mut provider,
            &reference,
            &lease(&reference, &request),
            &request,
            &FixedClock::new(NOW).unwrap(),
        )
        .unwrap();
    let blob = cache_root.join(format!(
        "{}.source",
        reference.content_digest.strip_prefix("sha256:").unwrap()
    ));
    let outside = root.join("outside-source");
    fs::write(&outside, &bytes).unwrap();
    fs::remove_file(&blob).unwrap();
    symlink(&outside, &blob).unwrap();
    provider.materialize_calls = 0;
    let next = workspace_request("00000000-0000-4000-8000-000000000303");
    assert!(manager
        .prepare(
            &mut source,
            &mut provider,
            &reference,
            &lease(&reference, &next),
            &next,
            &FixedClock::new(NOW).unwrap(),
        )
        .is_err());
    assert_eq!(provider.materialize_calls, 0);
    assert_eq!(provider.stop_calls, 0);
    assert_eq!(source.acquire_calls, 1);
    assert_eq!(fs::read(&outside).unwrap(), bytes);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn low_fd_workspace_lifecycle_handles_two_hundred_files_and_directories() {
    const CHILD: &str = "FKST_6092_LOW_FD_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new("/bin/sh")
            .arg("-c")
            .arg("ulimit -n 128 && exec \"$1\" --exact low_fd_workspace_lifecycle_handles_two_hundred_files_and_directories --nocapture")
            .arg("6092-low-fd")
            .arg(std::env::current_exe().unwrap())
            .env(CHILD, "1")
            .output().unwrap();
        assert!(
            output.status.success(),
            "child failed: {}\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    struct ManyFiles {
        inner: FakeWorkspaceProvider,
        directories: bool,
    }
    impl WorkspaceProvider for ManyFiles {
        fn materialize(
            &mut self,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let receipt = self.inner.materialize(blob, root, revision)?;
            for index in 0..200 {
                let path = root.join(format!("entry-{index}"));
                if self.directories {
                    fs::create_dir(&path)?;
                    fs::write(path.join("payload"), b"ordinary small file")?;
                } else {
                    fs::write(path, b"ordinary small file")?;
                }
            }
            Ok(receipt)
        }
        fn status(&mut self, identity: &str) -> Result<WorkspaceProviderStatus, RunError> {
            self.inner.status(identity)
        }
        fn stop(&mut self, identity: &str) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(identity)
        }
    }
    for directories in [false, true] {
        let mut fixture = WorkspaceFixture::new("low-fd");
        let outside = fixture.outside();
        let mut provider = ManyFiles {
            inner: FakeWorkspaceProvider::default(),
            directories,
        };
        let handle = fixture
            .manager
            .prepare(
                &mut fixture.source,
                &mut provider,
                &fixture.reference,
                &lease(&fixture.reference, &fixture.request),
                &fixture.request,
                &FixedClock::new(NOW).unwrap(),
            )
            .unwrap();
        let replay = fixture
            .manager
            .prepare(
                &mut fixture.source,
                &mut provider,
                &fixture.reference,
                &lease(&fixture.reference, &fixture.request),
                &fixture.request,
                &FixedClock::new(NOW).unwrap(),
            )
            .unwrap();
        assert_eq!(handle, replay);
        assert_eq!(
            fixture.manager.status(&mut provider, &handle).unwrap(),
            WorkspaceStatus::Active
        );
        fixture.manager.stop(&mut provider, &handle).unwrap();
        assert!(!handle.root.exists());
        assert_eq!(provider.inner.materialize_calls, 1);
        assert_eq!(provider.inner.stop_calls, 1);
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn partial_removal_failure_preserves_ownership_marker_and_allows_retry() {
    use std::os::unix::fs::PermissionsExt;
    let mut fixture = WorkspaceFixture::new("partial-removal");
    let handle = fixture.prepare().unwrap();
    let outside = fixture.outside();
    let child = handle.root.join("read-only-child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("payload"), b"retained after failure").unwrap();
    let marker = handle.root.join(".fkst-workspace.json");
    let original_marker = fs::read(&marker).unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o500)).unwrap();
    let result = fixture.manager.stop(&mut fixture.provider, &handle);
    // Restore only our fixture even when the assertion below fails.
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(result.is_err(), "test needs ordinary non-root permissions");
    assert_eq!(fs::read(&marker).unwrap(), original_marker);
    assert_eq!(
        fs::read(child.join("payload")).unwrap(),
        b"retained after failure"
    );
    assert_eq!(fixture.provider.stop_calls, 1);
    assert!(
        fixture
            .manager
            .stop(&mut fixture.provider, &handle)
            .unwrap()
            .already_stopped
    );
    assert_eq!(fixture.provider.stop_calls, 1);
    assert!(!handle.root.exists());
    assert!(
        fixture
            .manager
            .stop(&mut fixture.provider, &handle)
            .unwrap()
            .already_stopped
    );
    assert_eq!(fixture.provider.stop_calls, 1);
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn workspace_depth_budget_accepts_boundary_and_rejects_excess_before_stop() {
    for depth in [32, 33] {
        let mut fixture = WorkspaceFixture::new("depth-budget");
        let handle = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let marker = fs::read(handle.root.join(".fkst-workspace.json")).unwrap();
        let mut nested = handle.root.clone();
        for _ in 0..depth {
            nested.push("d");
            fs::create_dir(&nested).unwrap();
        }
        fs::write(nested.join("payload"), b"nested source").unwrap();
        if depth == 32 {
            assert_eq!(
                fixture
                    .manager
                    .status(&mut fixture.provider, &handle)
                    .unwrap(),
                WorkspaceStatus::Active
            );
            fixture
                .manager
                .stop(&mut fixture.provider, &handle)
                .unwrap();
            assert_eq!(fixture.provider.stop_calls, 1);
            assert!(!handle.root.exists());
        } else {
            assert!(fixture
                .manager
                .status(&mut fixture.provider, &handle)
                .is_err());
            assert!(fixture
                .manager
                .stop(&mut fixture.provider, &handle)
                .is_err());
            assert_eq!(fixture.provider.stop_calls, 0);
            assert_eq!(
                fs::read(handle.root.join(".fkst-workspace.json")).unwrap(),
                marker
            );
            assert_eq!(fs::read(nested.join("payload")).unwrap(), b"nested source");
        }
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn workspace_entry_budget_accepts_ten_thousand_and_rejects_one_more_without_mutation() {
    let mut fixture = WorkspaceFixture::new("entry-budget");
    let handle = fixture.prepare().unwrap();
    let outside = fixture.outside();
    let marker = fs::read(handle.root.join(".fkst-workspace.json")).unwrap();
    // source.bin and the ownership marker consume the first two entries.
    for index in 2..10_000 {
        fs::write(handle.root.join(format!("file-{index}")), b"small").unwrap();
    }
    assert_eq!(
        fixture
            .manager
            .status(&mut fixture.provider, &handle)
            .unwrap(),
        WorkspaceStatus::Active
    );
    fs::write(handle.root.join("over-budget"), b"retain").unwrap();
    assert!(fixture
        .manager
        .stop(&mut fixture.provider, &handle)
        .is_err());
    assert_eq!(fixture.provider.stop_calls, 0);
    assert_eq!(fs::read_dir(&handle.root).unwrap().count(), 10_001);
    assert_eq!(
        fs::read(handle.root.join(".fkst-workspace.json")).unwrap(),
        marker
    );
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn prepare_reserves_one_entry_for_the_ownership_marker_before_publishing() {
    struct EntryCountProvider {
        inner: FakeWorkspaceProvider,
        entries: usize,
    }
    impl WorkspaceProvider for EntryCountProvider {
        fn materialize(
            &mut self,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let receipt = self.inner.materialize(blob, root, revision)?;
            // The inner provider writes source.bin, which counts as entry one.
            for index in 1..self.entries {
                fs::write(root.join(format!("file-{index}")), b"provider data")?;
            }
            Ok(receipt)
        }
        fn status(&mut self, identity: &str) -> Result<WorkspaceProviderStatus, RunError> {
            self.inner.status(identity)
        }
        fn stop(&mut self, identity: &str) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(identity)
        }
    }
    for entries in [9_999, 10_000] {
        let mut fixture = WorkspaceFixture::new("prepare-entry-budget");
        let outside = fixture.outside();
        let mut provider = EntryCountProvider {
            inner: FakeWorkspaceProvider::default(),
            entries,
        };
        let root = second_request_path(&fixture.root.join("workspaces"), &fixture.request);
        let result = fixture.manager.prepare(
            &mut fixture.source,
            &mut provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &FixedClock::new(NOW).unwrap(),
        );
        if entries == 9_999 {
            let handle = result.unwrap();
            assert_eq!(fs::read_dir(&root).unwrap().count(), 10_000);
            assert!(root.join(".fkst-workspace.json").is_file());
            let replay = fixture
                .manager
                .prepare(
                    &mut fixture.source,
                    &mut provider,
                    &fixture.reference,
                    &lease(&fixture.reference, &fixture.request),
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap(),
                )
                .unwrap();
            assert_eq!(replay, handle);
            assert_eq!(
                fixture.manager.status(&mut provider, &handle).unwrap(),
                WorkspaceStatus::Active
            );
            assert!(
                !fixture
                    .manager
                    .stop(&mut provider, &handle)
                    .unwrap()
                    .already_stopped
            );
            assert!(
                fixture
                    .manager
                    .stop(&mut provider, &handle)
                    .unwrap()
                    .already_stopped
            );
            assert_eq!(provider.inner.stop_calls, 1);
            assert!(!root.exists());
        } else {
            assert!(
                result.is_err(),
                "10000 provider entries must be rejected before publishing a marker"
            );
            assert_eq!(
                fs::symlink_metadata(root.join(".fkst-workspace.json"))
                    .unwrap_err()
                    .kind(),
                std::io::ErrorKind::NotFound
            );
            assert_eq!(fs::read_dir(&root).unwrap().count(), 10_000);
            assert_eq!(
                fs::read(root.join("source.bin")).unwrap(),
                fixture.source.bytes
            );
            assert_eq!(fs::read(root.join("file-9999")).unwrap(), b"provider data");
            assert_eq!(provider.inner.stop_calls, 0);
        }
        assert_eq!(provider.inner.materialize_calls, 1);
        assert_eq!(fixture.source.acquire_calls, 1);
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn root_depth_budget_accepts_thirty_two_components_and_rejects_thirty_three() {
    use std::path::Component;
    for components in [32, 33] {
        let root = temporary_root("root-budget");
        let root_components = root
            .components()
            .filter(|part| matches!(part, Component::Normal(_)))
            .count();
        assert!(root_components < 32);
        let mut deep = root.clone();
        for _ in root_components..components {
            deep.push("d");
        }
        assert_eq!(
            deep.components()
                .filter(|part| matches!(part, Component::Normal(_)))
                .count(),
            components
        );
        let result = SourceWorkspaceManager::new(&deep, root.join("workspaces"));
        if components == 32 {
            assert!(result.is_ok());
            assert!(deep.is_dir());
        } else {
            assert!(result.is_err());
            assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        }
        drop(result);
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
struct WorkspaceFixture {
    root: PathBuf,
    manager: SourceWorkspaceManager,
    source: FakeSourceProvider,
    provider: FakeWorkspaceProvider,
    reference: DigestBoundReferenceV2,
    request: WorkspaceRequest,
}

#[cfg(unix)]
impl WorkspaceFixture {
    fn new(name: &str) -> Self {
        let root = temporary_root(name);
        let manager =
            SourceWorkspaceManager::new(root.join("cache"), root.join("workspaces")).unwrap();
        let bytes = b"immutable fixture source".to_vec();
        Self {
            root,
            manager,
            reference: source_reference(&bytes),
            source: FakeSourceProvider {
                bytes,
                revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
                acquire_calls: 0,
            },
            provider: FakeWorkspaceProvider::default(),
            request: workspace_request("00000000-0000-4000-8000-000000000401"),
        }
    }

    fn prepare(
        &mut self,
    ) -> Result<fkst_local_qa_host::source_workspace::WorkspaceHandle, RunError> {
        self.manager.prepare(
            &mut self.source,
            &mut self.provider,
            &self.reference,
            &lease(&self.reference, &self.request),
            &self.request,
            &FixedClock::new(NOW).unwrap(),
        )
    }

    fn outside(&self) -> PathBuf {
        let outside = self.root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"preserve").unwrap();
        outside
    }

    fn assert_untouched(&self, outside: &Path) {
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
        assert_eq!(self.provider.materialize_calls, 0);
        assert_eq!(self.provider.stop_calls, 0);
    }
}

#[cfg(unix)]
impl Drop for WorkspaceFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[cfg(unix)]
#[test]
fn configured_roots_reject_symlinked_ancestors_and_dangling_links_without_creating_outside() {
    use std::os::unix::fs::symlink;
    for dangling in [false, true] {
        let root = temporary_root("source-workspace-root-ancestor");
        let outside = root.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("sentinel"), b"preserve").unwrap();
        let target = if dangling {
            outside.join("absent")
        } else {
            outside.clone()
        };
        symlink(&target, root.join("alias")).unwrap();
        assert!(
            SourceWorkspaceManager::new(root.join("alias/cache"), root.join("workspaces")).is_err()
        );
        assert!(
            SourceWorkspaceManager::new(root.join("cache"), root.join("alias/workspaces")).is_err()
        );
        assert!(SourceWorkspaceManager::new(root.join("alias"), root.join("workspaces")).is_err());
        assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn replaced_pinned_roots_and_run_or_generation_links_have_zero_workspace_effects() {
    use std::os::unix::fs::symlink;
    for location in ["cache", "workspaces", "run", "generation"] {
        for dangling in [false, true] {
            let mut fixture = WorkspaceFixture::new("source-workspace-replaced-root");
            let outside = fixture.outside();
            let workspace_root = fixture.root.join("workspaces");
            let run = workspace_root.join(&fixture.request.run_id);
            let attacked = match location {
                "cache" | "workspaces" => {
                    let attacked = fixture.root.join(location);
                    fs::rename(&attacked, fixture.root.join("displaced")).unwrap();
                    attacked
                }
                "run" => run,
                "generation" => {
                    fs::create_dir(&run).unwrap();
                    run.join("generation-1")
                }
                _ => unreachable!(),
            };
            let target = if dangling {
                outside.join("absent")
            } else {
                outside.clone()
            };
            symlink(target, attacked).unwrap();
            assert!(
                fixture.prepare().is_err(),
                "location={location}, dangling={dangling}"
            );
            fixture.assert_untouched(&outside);
            assert_eq!(fixture.source.acquire_calls, 0);
            assert_eq!(fs::read_dir(outside).unwrap().count(), 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn cache_markers_blobs_and_workspace_objects_reject_links_and_special_files() {
    use nix::sys::stat::Mode;
    use nix::unistd::mkfifo;
    use std::os::unix::fs::symlink;

    for location in ["blob", "cache-marker", "workspace-marker", "nested"] {
        for object in ["symlink", "dangling", "hardlink", "fifo"] {
            let mut fixture = WorkspaceFixture::new("source-workspace-rejected-object");
            let handle = fixture.prepare().unwrap();
            let outside = fixture.outside();
            let blob = fixture.root.join("cache").join(format!(
                "{}.source",
                fixture
                    .reference
                    .content_digest
                    .strip_prefix("sha256:")
                    .unwrap()
            ));
            let attacked = match location {
                "blob" => blob,
                "cache-marker" => blob.with_extension("source.json"),
                "workspace-marker" => handle.root.join(".fkst-workspace.json"),
                "nested" => {
                    let nested = handle.root.join("nested");
                    fs::create_dir(&nested).unwrap();
                    let file = nested.join("payload");
                    fs::write(&file, b"nested data").unwrap();
                    file
                }
                _ => unreachable!(),
            };
            let outside_file = outside.join("payload");
            fs::copy(&attacked, &outside_file).unwrap();
            let original = fs::read(&outside_file).unwrap();
            fs::remove_file(&attacked).unwrap();
            match object {
                "symlink" => symlink(&outside_file, &attacked).unwrap(),
                "dangling" => symlink(outside.join("absent"), &attacked).unwrap(),
                "hardlink" => fs::hard_link(&outside_file, &attacked).unwrap(),
                "fifo" => mkfifo(&attacked, Mode::S_IRUSR | Mode::S_IWUSR).unwrap(),
                _ => unreachable!(),
            }
            fixture.provider.materialize_calls = 0;
            let acquisitions = fixture.source.acquire_calls;
            assert!(
                fixture.prepare().is_err(),
                "location={location}, object={object}"
            );
            if matches!(location, "workspace-marker" | "nested") {
                assert!(fixture
                    .manager
                    .status(&mut fixture.provider, &handle)
                    .is_err());
                assert!(fixture
                    .manager
                    .stop(&mut fixture.provider, &handle)
                    .is_err());
            }
            fixture.assert_untouched(&outside);
            assert_eq!(fixture.source.acquire_calls, acquisitions);
            assert_eq!(fs::read(&outside_file).unwrap(), original);
        }
    }
}

#[cfg(unix)]
#[test]
fn cache_replacement_after_validation_rejects_replay_and_creation_before_workspace_effects() {
    use std::cell::Cell;
    use std::os::unix::fs::symlink;

    struct ReplacingClock {
        reads: Cell<usize>,
        original: PathBuf,
        displaced: PathBuf,
        replacement: PathBuf,
    }
    impl fkst_local_qa_host::Clock for ReplacingClock {
        fn now_utc(&self) -> Result<String, RunError> {
            self.reads.set(self.reads.get() + 1);
            if self.reads.get() == 2 {
                fs::rename(&self.original, &self.displaced)?;
                symlink(&self.replacement, &self.original)?;
            }
            Ok(NOW.to_owned())
        }
    }

    for replay in [false, true] {
        for marker in [false, true] {
            let mut fixture = WorkspaceFixture::new("source-workspace-cache-boundary");
            fixture.prepare().unwrap();
            fixture.provider.materialize_calls = 0;
            if !replay {
                fixture.request.generation = 2;
            }
            let outside = fixture.outside();
            let mut attacked = fixture.root.join("cache").join(format!(
                "{}.source",
                fixture
                    .reference
                    .content_digest
                    .strip_prefix("sha256:")
                    .unwrap()
            ));
            if marker {
                attacked = attacked.with_extension("source.json");
            }
            let replacement = outside.join("payload");
            fs::copy(&attacked, &replacement).unwrap();
            let bytes = fs::read(&replacement).unwrap();
            let clock = ReplacingClock {
                reads: Cell::new(0),
                original: attacked,
                displaced: fixture.root.join("displaced"),
                replacement: replacement.clone(),
            };
            assert!(
                fixture
                    .manager
                    .prepare(
                        &mut fixture.source,
                        &mut fixture.provider,
                        &fixture.reference,
                        &lease(&fixture.reference, &fixture.request),
                        &fixture.request,
                        &clock
                    )
                    .is_err(),
                "replay={replay}, marker={marker}"
            );
            fixture.assert_untouched(&outside);
            assert_eq!(fs::read(&replacement).unwrap(), bytes);
            if !replay {
                assert!(
                    !second_request_path(&fixture.root.join("workspaces"), &fixture.request)
                        .exists()
                );
            }
        }
    }
}

#[cfg(unix)]
struct SwappingProvider {
    inner: FakeWorkspaceProvider,
    swap_during_stop: bool,
    original: PathBuf,
    displaced: PathBuf,
    outside: PathBuf,
    swapped: bool,
}

#[cfg(unix)]
impl SwappingProvider {
    fn swap(&mut self) {
        if !self.swapped {
            fs::rename(&self.original, &self.displaced).unwrap();
            std::os::unix::fs::symlink(&self.outside, &self.original).unwrap();
            self.swapped = true;
        }
    }
}

#[cfg(unix)]
impl WorkspaceProvider for SwappingProvider {
    fn materialize(
        &mut self,
        blob: &Path,
        root: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError> {
        self.inner.materialize(blob, root, revision)
    }
    fn status(&mut self, identity: &str) -> Result<WorkspaceProviderStatus, RunError> {
        if !self.swap_during_stop {
            self.swap();
        }
        self.inner.status(identity)
    }
    fn stop(&mut self, identity: &str) -> Result<WorkspaceProviderStopReceipt, RunError> {
        if self.swap_during_stop {
            self.swap();
        }
        self.inner.stop(identity)
    }
}

#[cfg(unix)]
#[test]
fn parent_replacement_during_status_is_rejected_before_stop_and_during_stop_before_deletion() {
    for during_stop in [false, true] {
        let mut fixture = WorkspaceFixture::new("source-workspace-stop-swap");
        let handle = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let external_generation = outside.join("generation-1");
        fs::create_dir(&external_generation).unwrap();
        fs::write(external_generation.join("sentinel"), b"external generation").unwrap();
        let mut provider = SwappingProvider {
            inner: std::mem::take(&mut fixture.provider),
            swap_during_stop: during_stop,
            original: handle.root.parent().unwrap().to_path_buf(),
            displaced: fixture.root.join("displaced"),
            outside: outside.clone(),
            swapped: false,
        };
        provider.inner.materialize_calls = 0;
        assert!(fixture.manager.stop(&mut provider, &handle).is_err());
        assert_eq!(provider.inner.stop_calls, usize::from(during_stop));
        assert_eq!(provider.inner.materialize_calls, 0);
        assert_eq!(
            fs::read(external_generation.join("sentinel")).unwrap(),
            b"external generation"
        );
        assert!(provider.displaced.join("generation-1/source.bin").is_file());
        assert!(provider
            .displaced
            .join("generation-1/.fkst-workspace.json")
            .is_file());
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn changed_root_ancestor_is_rejected_even_when_leaf_directories_still_exist() {
    use std::os::unix::fs::symlink;
    let outer = temporary_root("source-workspace-root-replacement");
    let owned = outer.join("owned");
    let manager =
        SourceWorkspaceManager::new(owned.join("cache"), owned.join("workspaces")).unwrap();
    let outside = outer.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(outside.join("sentinel"), b"preserve").unwrap();
    fs::rename(&owned, outer.join("displaced")).unwrap();
    symlink(&outside, &owned).unwrap();
    let request = workspace_request("00000000-0000-4000-8000-000000000402");
    let mut source = FakeSourceProvider {
        bytes: b"source".to_vec(),
        revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
        acquire_calls: 0,
    };
    let reference = source_reference(&source.bytes);
    let mut provider = FakeWorkspaceProvider::default();
    assert!(manager
        .prepare(
            &mut source,
            &mut provider,
            &reference,
            &lease(&reference, &request),
            &request,
            &FixedClock::new(NOW).unwrap()
        )
        .is_err());
    assert_eq!(source.acquire_calls, 0);
    assert_eq!(provider.materialize_calls, 0);
    assert_eq!(fs::read_dir(&outside).unwrap().count(), 1);
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    fs::remove_dir_all(outer).unwrap();
}

#[cfg(unix)]
#[test]
fn stop_rejects_symlinked_workspace_roots_run_parents_and_generations_before_mutation() {
    for location in ["workspace-root", "run", "generation"] {
        for dangling in [false, true] {
            let mut fixture = WorkspaceFixture::new("source-workspace-stop-link");
            let handle = fixture.prepare().unwrap();
            fixture.provider.materialize_calls = 0;
            let outside = fixture.outside();
            let attacked = match location {
                "workspace-root" => fixture.root.join("workspaces"),
                "run" => handle.root.parent().unwrap().to_path_buf(),
                "generation" => handle.root.clone(),
                _ => unreachable!(),
            };
            let external_tree = outside.join("owned");
            let external_blob = external_tree
                .join(handle.root.strip_prefix(&attacked).unwrap())
                .join("source.bin");
            fs::rename(&attacked, &external_tree).unwrap();
            let target = if dangling {
                outside.join("absent")
            } else {
                external_tree
            };
            std::os::unix::fs::symlink(target, attacked).unwrap();
            assert!(fixture
                .manager
                .status(&mut fixture.provider, &handle)
                .is_err());
            assert!(fixture
                .manager
                .stop(&mut fixture.provider, &handle)
                .is_err());
            fixture.assert_untouched(&outside);
            assert_eq!(fs::read(&external_blob).unwrap(), fixture.source.bytes);
        }
    }
}

#[cfg(unix)]
#[test]
fn unknown_provider_identity_retains_the_validated_workspace() {
    let mut fixture = WorkspaceFixture::new("source-workspace-unknown-provider");
    let handle = fixture.prepare().unwrap();
    fixture.provider.materialize_calls = 0;
    fixture.provider.active.clear();
    assert!(fixture
        .manager
        .stop(&mut fixture.provider, &handle)
        .is_err());
    assert_eq!(fixture.provider.stop_calls, 0);
    assert_eq!(
        fs::read(handle.root.join("source.bin")).unwrap(),
        fixture.source.bytes
    );
    assert!(handle.root.join(".fkst-workspace.json").is_file());
}

#[cfg(unix)]
#[test]
fn failed_or_invalid_materialization_retains_unknown_objects_without_cleanup() {
    struct UntrustedProvider {
        mode: &'static str,
        outside: PathBuf,
        displaced: PathBuf,
        calls: usize,
    }
    impl WorkspaceProvider for UntrustedProvider {
        fn materialize(
            &mut self,
            _: &Path,
            root: &Path,
            _: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            self.calls += 1;
            fs::write(root.join("partial"), b"retain for diagnosis")?;
            match self.mode {
                "parent-swap" => {
                    let parent = root.parent().unwrap();
                    fs::rename(parent, &self.displaced)?;
                    std::os::unix::fs::symlink(&self.outside, parent)?;
                }
                "nested-link" => std::os::unix::fs::symlink(&self.outside, root.join("nested"))?,
                "marker-link" => std::os::unix::fs::symlink(
                    self.outside.join("sentinel"),
                    root.join(".fkst-workspace.json"),
                )?,
                "provider-error" => {
                    return Err(RunError::Lifecycle("injected materialization failure"))
                }
                "empty-identity" => (),
                _ => unreachable!(),
            }
            Ok(WorkspaceMaterialization {
                provider_identity: if self.mode == "empty-identity" {
                    String::new()
                } else {
                    "provider/materialized".to_owned()
                },
            })
        }
        fn status(&mut self, _: &str) -> Result<WorkspaceProviderStatus, RunError> {
            panic!("prepare must not query this unowned provider")
        }
        fn stop(&mut self, _: &str) -> Result<WorkspaceProviderStopReceipt, RunError> {
            panic!("prepare must not stop this unowned provider")
        }
    }
    for mode in [
        "parent-swap",
        "nested-link",
        "marker-link",
        "provider-error",
        "empty-identity",
    ] {
        let mut fixture = WorkspaceFixture::new("source-workspace-invalid-materialization");
        let outside = fixture.outside();
        fs::create_dir(outside.join("generation-1")).unwrap();
        fs::write(
            outside.join("generation-1/sentinel"),
            b"external generation",
        )
        .unwrap();
        let mut provider = UntrustedProvider {
            mode,
            outside: outside.clone(),
            displaced: fixture.root.join("displaced"),
            calls: 0,
        };
        assert!(
            fixture
                .manager
                .prepare(
                    &mut fixture.source,
                    &mut provider,
                    &fixture.reference,
                    &lease(&fixture.reference, &fixture.request),
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .is_err(),
            "mode={mode}"
        );
        assert_eq!(provider.calls, 1);
        let retained = if mode == "parent-swap" {
            provider.displaced.join("generation-1")
        } else {
            second_request_path(&fixture.root.join("workspaces"), &fixture.request)
        };
        assert_eq!(
            fs::read(retained.join("partial")).unwrap(),
            b"retain for diagnosis"
        );
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
        assert_eq!(
            fs::read(outside.join("generation-1/sentinel")).unwrap(),
            b"external generation"
        );
    }
}

#[cfg(target_os = "macos")]
#[test]
fn macos_var_alias_is_rejected_without_canonicalizing_untrusted_roots() {
    let root = temporary_root("source-workspace-var-alias");
    if let Ok(relative) = root.strip_prefix("/private/var") {
        let alias = Path::new("/var").join(relative);
        assert!(SourceWorkspaceManager::new(alias.join("cache"), root.join("workspaces")).is_err());
        assert!(!root.join("cache").exists());
    } else {
        assert!(SourceWorkspaceManager::new("/var/empty/cache", root.join("workspaces")).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(not(unix))]
#[test]
fn unsupported_platform_rejects_workspace_roots_without_effects() {
    let root = temporary_root("source-workspace-unsupported");
    assert!(SourceWorkspaceManager::new(root.join("cache"), root.join("workspaces")).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
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
    // Resolve only this freshly created trusted fixture root (macOS /var alias).
    fs::canonicalize(path).unwrap()
}
