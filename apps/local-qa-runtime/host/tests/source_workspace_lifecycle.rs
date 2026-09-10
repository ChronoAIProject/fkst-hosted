use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use fkst_local_qa_host::source_workspace::{
    lifecycle_authority_blockers, validate_controlled_relative_path, AcquiredSource,
    ImmutableRevision, LifecycleAuthorityBlocker, SourceObjectLease, SourceProvider,
    SourceWorkspaceManager, TrustedLocalSourceBinding, WorkspaceDiscovery, WorkspaceIntent,
    WorkspaceMaterialization, WorkspaceProvider, WorkspaceProviderScope, WorkspaceProviderStatus,
    WorkspaceProviderStatusReceipt, WorkspaceProviderStopReceipt, WorkspaceRequest,
    WorkspaceResource, WorkspaceStatus,
};
#[cfg(unix)]
use fkst_local_qa_host::source_workspace::{stable_workspace_key, WorkspaceState};
use fkst_local_qa_host::Journal;
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
            source_object_id: lease.binding.source_object_id.clone(),
            immutable_revision: self.revision.clone(),
            provider_scope: "fixture-source/v1".to_owned(),
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
    resources: BTreeMap<String, WorkspaceResource>,
}

impl WorkspaceProvider for FakeWorkspaceProvider {
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
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
        let resource = WorkspaceResource {
            intent: intent.clone(),
            provider_identity,
        };
        self.resources
            .insert(resource.provider_identity.clone(), resource.clone());
        Ok(WorkspaceMaterialization { resource })
    }

    fn scope(&self) -> &str {
        "fixture-provider/v1"
    }
    fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
        let resource = self
            .resources
            .values()
            .find(|resource| resource.intent.stable_key == intent.stable_key)
            .cloned();
        match resource {
            Some(resource) => Ok(WorkspaceDiscovery::Found(Box::new(self.status(&resource)?))),
            None => Ok(WorkspaceDiscovery::Absent),
        }
    }
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
        let status = match self.active.get(&resource.provider_identity) {
            Some(true) => WorkspaceProviderStatus::Active,
            Some(false) => WorkspaceProviderStatus::Stopped,
            None => WorkspaceProviderStatus::Unknown,
        };
        Ok(WorkspaceProviderStatusReceipt {
            resource: self
                .resources
                .get(&resource.provider_identity)
                .unwrap_or(resource)
                .clone(),
            status,
        })
    }
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, RunError> {
        self.stop_calls += 1;
        let Some(active) = self.active.get_mut(&resource.provider_identity) else {
            return Ok(WorkspaceProviderStopReceipt {
                resource: resource.clone(),
                stopped: false,
            });
        };
        *active = false;
        Ok(WorkspaceProviderStopReceipt {
            resource: self
                .resources
                .get(&resource.provider_identity)
                .unwrap()
                .clone(),
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
    let manager = test_manager(&cache_root, &workspace_root).unwrap();
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
    assert_eq!(fs::read(first.root().join("source.bin")).unwrap(), bytes);
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
    assert_ne!(first.root(), second.root());
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
    let manager = test_manager(&cache_root, &workspace_root).unwrap();
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
    fs::remove_dir_all(first.root()).unwrap();
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
    let manager = test_manager(root.join("cache"), &workspace_root).unwrap();
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
    let manager = test_manager(&cache_root, root.join("workspaces")).unwrap();
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
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }

        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let receipt = self.inner.materialize(intent, blob, root, revision)?;
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
        fn status(
            &mut self,
            identity: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            self.inner.status(identity)
        }
        fn stop(
            &mut self,
            identity: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
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
        assert!(!handle.root().exists());
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
    let child = handle.root().join("read-only-child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("payload"), b"retained after failure").unwrap();
    let marker = handle.root().join(".fkst-workspace.json");
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
    assert!(!handle.root().exists());
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
        let marker = fs::read(handle.root().join(".fkst-workspace.json")).unwrap();
        let mut nested = handle.root().to_path_buf();
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
            assert!(!handle.root().exists());
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
                fs::read(handle.root().join(".fkst-workspace.json")).unwrap(),
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
    let marker = fs::read(handle.root().join(".fkst-workspace.json")).unwrap();
    // source.bin and the ownership marker consume the first two entries.
    for index in 2..10_000 {
        fs::write(handle.root().join(format!("file-{index}")), b"small").unwrap();
    }
    assert_eq!(
        fixture
            .manager
            .status(&mut fixture.provider, &handle)
            .unwrap(),
        WorkspaceStatus::Active
    );
    fs::write(handle.root().join("over-budget"), b"retain").unwrap();
    assert!(fixture
        .manager
        .stop(&mut fixture.provider, &handle)
        .is_err());
    assert_eq!(fixture.provider.stop_calls, 0);
    assert_eq!(fs::read_dir(handle.root()).unwrap().count(), 10_001);
    assert_eq!(
        fs::read(handle.root().join(".fkst-workspace.json")).unwrap(),
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
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }

        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let receipt = self.inner.materialize(intent, blob, root, revision)?;
            // The inner provider writes source.bin, which counts as entry one.
            for index in 1..self.entries {
                fs::write(root.join(format!("file-{index}")), b"provider data")?;
            }
            Ok(receipt)
        }
        fn status(
            &mut self,
            identity: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            self.inner.status(identity)
        }
        fn stop(
            &mut self,
            identity: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
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
        let result = test_manager(&deep, root.join("workspaces"));
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
    manager: TestManager,
    source: FakeSourceProvider,
    provider: FakeWorkspaceProvider,
    reference: DigestBoundReferenceV2,
    request: WorkspaceRequest,
}

#[cfg(unix)]
impl WorkspaceFixture {
    fn new(name: &str) -> Self {
        let root = temporary_root(name);
        let manager = test_manager(root.join("cache"), root.join("workspaces")).unwrap();
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
        assert!(test_manager(root.join("alias/cache"), root.join("workspaces")).is_err());
        assert!(test_manager(root.join("cache"), root.join("alias/workspaces")).is_err());
        assert!(test_manager(root.join("alias"), root.join("workspaces")).is_err());
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
                "workspace-marker" => handle.root().join(".fkst-workspace.json"),
                "nested" => {
                    let nested = handle.root().join("nested");
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
    fn scope(&self) -> &str {
        self.inner.scope()
    }
    fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
        self.inner.discover(intent)
    }

    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        blob: &Path,
        root: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError> {
        self.inner.materialize(intent, blob, root, revision)
    }
    fn status(
        &mut self,
        identity: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
        if !self.swap_during_stop {
            self.swap();
        }
        self.inner.status(identity)
    }
    fn stop(
        &mut self,
        identity: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, RunError> {
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
            original: handle.root().parent().unwrap().to_path_buf(),
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
    let manager = test_manager(owned.join("cache"), owned.join("workspaces")).unwrap();
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
                "run" => handle.root().parent().unwrap().to_path_buf(),
                "generation" => handle.root().to_path_buf(),
                _ => unreachable!(),
            };
            let external_tree = outside.join("owned");
            let external_blob = external_tree
                .join(handle.root().strip_prefix(&attacked).unwrap())
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
        fs::read(handle.root().join("source.bin")).unwrap(),
        fixture.source.bytes
    );
    assert!(handle.root().join(".fkst-workspace.json").is_file());
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
        fn scope(&self) -> &str {
            "fixture-provider/v1"
        }
        fn discover(&mut self, _: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            Ok(WorkspaceDiscovery::Absent)
        }

        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
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
                resource: WorkspaceResource {
                    intent: intent.clone(),
                    provider_identity: if self.mode == "empty-identity" {
                        String::new()
                    } else {
                        "provider/materialized".to_owned()
                    },
                },
            })
        }
        fn status(
            &mut self,
            _: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            panic!("prepare must not query this unowned provider")
        }
        fn stop(
            &mut self,
            _: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
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
        assert!(test_manager(alias.join("cache"), root.join("workspaces")).is_err());
        assert!(!root.join("cache").exists());
    } else {
        assert!(test_manager("/var/empty/cache", root.join("workspaces")).is_err());
    }
    fs::remove_dir_all(root).unwrap();
}

#[cfg(not(unix))]
#[test]
fn unsupported_platform_rejects_workspace_roots_without_effects() {
    let root = temporary_root("source-workspace-unsupported");
    assert!(test_manager(root.join("cache"), root.join("workspaces")).is_err());
    assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(unix)]
#[test]
fn writable_marker_cannot_redirect_workspace_a_to_live_workspace_b() {
    let mut fixture = WorkspaceFixture::new("marker-identity-red");
    let a = fixture.prepare().unwrap();
    fixture.request.generation = 2;
    let b = fixture.prepare().unwrap();
    fixture.request.generation = 1;
    let marker_path = a.root().join(".fkst-workspace.json");
    let mut marker: serde_json::Value =
        serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
    marker["workspace_provider_identity"] = b.workspace_provider_identity().to_owned().into();
    fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
    if let Ok(replay) = fixture.prepare() {
        let _ = fixture.manager.stop(&mut fixture.provider, &replay);
    }
    assert_eq!(
        fixture.provider.active.get(b.workspace_provider_identity()),
        Some(&true)
    );
    assert_eq!(fixture.provider.stop_calls, 0);
}

#[cfg(unix)]
#[test]
fn replay_never_returns_a_stopped_or_unknown_provider_as_usable() {
    for stopped in [true, false] {
        let mut fixture = WorkspaceFixture::new("provider-replay-red");
        let handle = fixture.prepare().unwrap();
        if stopped {
            fixture
                .provider
                .active
                .insert(handle.workspace_provider_identity().to_owned(), false);
        } else {
            fixture.provider.active.clear();
        }
        assert!(fixture.prepare().is_err());
        assert_eq!(fixture.provider.materialize_calls, 1);
    }
}

#[cfg(unix)]
struct InterruptedProvider {
    inner: FakeWorkspaceProvider,
    database: PathBuf,
    discovery: &'static str,
    interrupt: bool,
}

#[cfg(unix)]
impl WorkspaceProvider for InterruptedProvider {
    fn scope(&self) -> &str {
        self.inner.scope()
    }
    fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
        match self.discovery {
            "unknown" => Ok(WorkspaceDiscovery::Unknown),
            "conflict" => Ok(WorkspaceDiscovery::Conflict),
            "absent" => Ok(WorkspaceDiscovery::Absent),
            "contradictory" => {
                let mut receipt = match self.inner.discover(intent)? {
                    WorkspaceDiscovery::Found(receipt) => receipt,
                    _ => return Ok(WorkspaceDiscovery::Absent),
                };
                receipt.resource.intent.generation += 1;
                Ok(WorkspaceDiscovery::Found(receipt))
            }
            _ => self.inner.discover(intent),
        }
    }
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        blob: &Path,
        root: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, RunError> {
        let independent = Journal::open(&self.database)?;
        let record = independent
            .workspace(&intent.stable_key)?
            .expect("intent visible before provider effect");
        assert_eq!(record.intent, *intent);
        assert_eq!(record.state, WorkspaceState::CreateAttempted);
        assert!(record.directory_identity.is_some());
        assert!(record.resource.is_none());
        let receipt = self.inner.materialize(intent, blob, root, revision)?;
        if self.interrupt {
            return Err(RunError::Lifecycle(
                "injected create-before-bind interruption",
            ));
        }
        Ok(receipt)
    }
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
        self.inner.status(resource)
    }
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, RunError> {
        self.inner.stop(resource)
    }
}

#[cfg(unix)]
impl WorkspaceFixture {
    fn database(&self) -> PathBuf {
        self.manager.journal_root.join("host.sqlite")
    }
    fn reopen(&mut self) {
        self.manager.inner = SourceWorkspaceManager::new(
            self.root.join("cache"),
            self.root.join("workspaces"),
            Journal::open(&self.database()).unwrap(),
            fixture_scope(),
        )
        .unwrap();
    }
}

#[cfg(unix)]
fn fixture_scope() -> WorkspaceProviderScope {
    WorkspaceProviderScope {
        identity: "fixture-provider/v1".to_owned(),
        writable_roots: vec![],
    }
}

#[cfg(unix)]
#[test]
fn intent_is_committed_before_effect_and_restart_discovers_create_without_recreation() {
    let mut fixture = WorkspaceFixture::new("durable-create-recovery");
    let mut provider = InterruptedProvider {
        inner: FakeWorkspaceProvider::default(),
        database: fixture.database(),
        discovery: "exact",
        interrupt: true,
    };
    assert!(fixture
        .manager
        .prepare(
            &mut fixture.source,
            &mut provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &FixedClock::new(NOW).unwrap()
        )
        .is_err());
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    let record = Journal::open(&fixture.database())
        .unwrap()
        .workspace(&key)
        .unwrap()
        .unwrap();
    assert_eq!(record.state, WorkspaceState::CreateAttempted);
    assert!(record.blocker.is_some());
    let path = second_request_path(&fixture.root.join("workspaces"), &fixture.request);
    assert_eq!(
        fs::read(path.join("source.bin")).unwrap(),
        fixture.source.bytes
    );
    fixture.reopen();
    let handle = fixture.manager.recover(&mut provider, &key).unwrap();
    assert_eq!(
        fixture.manager.status(&mut provider, &handle).unwrap(),
        WorkspaceStatus::Active
    );
    assert_eq!(provider.inner.materialize_calls, 1);
    let rebound = Journal::open(&fixture.database())
        .unwrap()
        .workspace(&key)
        .unwrap()
        .unwrap();
    assert_eq!(rebound.state, WorkspaceState::Bound);
    assert_eq!(
        rebound.resource.unwrap().provider_identity,
        handle.workspace_provider_identity()
    );
    fixture.manager.stop(&mut provider, &handle).unwrap();
    assert!(!path.exists());
    assert!(
        fixture
            .manager
            .stop(&mut provider, &handle)
            .unwrap()
            .already_stopped
    );
    assert_eq!(provider.inner.stop_calls, 1);
}

#[cfg(unix)]
#[test]
fn uncertain_or_conflicting_discovery_never_repeats_attempted_creation() {
    for discovery in ["unknown", "conflict", "absent", "contradictory"] {
        let mut fixture = WorkspaceFixture::new("uncertain-create");
        let outside = fixture.outside();
        let mut provider = InterruptedProvider {
            inner: FakeWorkspaceProvider::default(),
            database: fixture.database(),
            discovery: "exact",
            interrupt: true,
        };
        assert!(fixture
            .manager
            .prepare(
                &mut fixture.source,
                &mut provider,
                &fixture.reference,
                &lease(&fixture.reference, &fixture.request),
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        provider.discovery = discovery;
        fixture.reopen();
        let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
        assert!(fixture.manager.recover(&mut provider, &key).is_err());
        assert!(fixture
            .manager
            .prepare(
                &mut fixture.source,
                &mut provider,
                &fixture.reference,
                &lease(&fixture.reference, &fixture.request),
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        let record = Journal::open(&fixture.database())
            .unwrap()
            .workspace(&key)
            .unwrap()
            .unwrap();
        assert_eq!(record.state, WorkspaceState::CreateAttempted);
        assert!(record.blocker.is_some());
        assert_eq!(provider.inner.materialize_calls, 1);
        assert_eq!(provider.inner.stop_calls, 0);
        assert_eq!(
            fs::read(
                second_request_path(&fixture.root.join("workspaces"), &fixture.request)
                    .join("source.bin")
            )
            .unwrap(),
            fixture.source.bytes
        );
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn stopped_before_marker_publication_is_recoverable_without_becoming_usable() {
    let mut fixture = WorkspaceFixture::new("stopped-before-bind");
    let mut provider = InterruptedProvider {
        inner: FakeWorkspaceProvider::default(),
        database: fixture.database(),
        discovery: "exact",
        interrupt: true,
    };
    assert!(fixture
        .manager
        .prepare(
            &mut fixture.source,
            &mut provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &FixedClock::new(NOW).unwrap()
        )
        .is_err());
    for active in provider.inner.active.values_mut() {
        *active = false;
    }
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    fixture.reopen();
    let handle = fixture.manager.recover(&mut provider, &key).unwrap();
    assert_eq!(
        fixture.manager.status(&mut provider, &handle).unwrap(),
        WorkspaceStatus::Stopped
    );
    // Unmarked partial provider data is retained, not swept speculatively.
    assert!(fixture.manager.stop(&mut provider, &handle).is_err());
    assert!(handle.root().join("source.bin").exists());
    assert_eq!(provider.inner.materialize_calls, 1);
    assert_eq!(provider.inner.stop_calls, 0);
}

#[cfg(unix)]
#[test]
fn marker_unlink_rmdir_window_recovers_only_the_same_empty_stopped_directory() {
    for change in ["none", "new-file", "replacement"] {
        let mut fixture = WorkspaceFixture::new("final-rmdir-window");
        let handle = fixture.prepare().unwrap();
        let outside = fixture.outside();
        fixture
            .provider
            .active
            .insert(handle.workspace_provider_identity().to_owned(), false);
        assert_eq!(
            fixture
                .manager
                .status(&mut fixture.provider, &handle)
                .unwrap(),
            WorkspaceStatus::Stopped
        );
        fs::remove_file(handle.root().join("source.bin")).unwrap();
        fs::remove_file(handle.root().join(".fkst-workspace.json")).unwrap();
        match change {
            "new-file" => fs::write(handle.root().join("new-file"), b"retain").unwrap(),
            "replacement" => {
                fs::rename(handle.root(), fixture.root.join("displaced")).unwrap();
                fs::create_dir(handle.root()).unwrap();
            }
            _ => (),
        }
        fixture.reopen();
        let result = fixture.manager.stop(&mut fixture.provider, &handle);
        if change == "none" {
            assert!(result.unwrap().already_stopped);
            assert!(!handle.root().exists());
            assert!(
                fixture
                    .manager
                    .stop(&mut fixture.provider, &handle)
                    .unwrap()
                    .already_stopped
            );
        } else {
            assert!(result.is_err());
            assert!(handle.root().is_dir());
            if change == "new-file" {
                assert_eq!(fs::read(handle.root().join("new-file")).unwrap(), b"retain");
            }
        }
        assert_eq!(fixture.provider.stop_calls, 0);
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn foreign_handle_and_mismatched_status_or_stop_receipts_are_rejected() {
    struct Mismatch {
        inner: FakeWorkspaceProvider,
        during_stop: bool,
    }
    impl WorkspaceProvider for Mismatch {
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            self.inner.materialize(intent, blob, root, revision)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            let mut receipt = self.inner.status(resource)?;
            if !self.during_stop {
                receipt.resource.intent.generation += 1;
            }
            Ok(receipt)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            let mut receipt = self.inner.stop(resource)?;
            receipt.resource.provider_identity = "unrelated-resource".to_owned();
            Ok(receipt)
        }
    }
    for during_stop in [false, true] {
        let mut fixture = WorkspaceFixture::new("mismatched-receipt");
        let handle = fixture.prepare().unwrap();
        let foreign = WorkspaceFixture::new("foreign-handle");
        assert!(foreign
            .manager
            .stop(&mut fixture.provider, &handle)
            .is_err());
        assert_eq!(fixture.provider.stop_calls, 0);
        let mut provider = Mismatch {
            inner: std::mem::take(&mut fixture.provider),
            during_stop,
        };
        assert!(fixture.manager.stop(&mut provider, &handle).is_err());
        assert_eq!(provider.inner.stop_calls, usize::from(during_stop));
        assert!(handle.root().join("source.bin").is_file());
        assert!(handle.root().join(".fkst-workspace.json").is_file());
        let key = stable_workspace_key(handle.run_id(), handle.generation());
        let record = Journal::open(&fixture.database())
            .unwrap()
            .workspace(&key)
            .unwrap()
            .unwrap();
        assert_eq!(
            record.state,
            if during_stop {
                WorkspaceState::StopAttempted
            } else {
                WorkspaceState::Bound
            }
        );
    }
}

#[cfg(unix)]
#[test]
fn changed_initial_deadline_or_provider_scope_cannot_rebind_an_existing_workspace() {
    let mut fixture = WorkspaceFixture::new("intent-immutability");
    let handle = fixture.prepare().unwrap();
    fixture.request.deadline_utc = "2026-09-12T00:00:00Z".to_owned();
    assert!(fixture.prepare().is_err());
    assert_eq!(fixture.provider.materialize_calls, 1);
    // Status and stop have no admission deadline gate.
    fixture
        .manager
        .stop(&mut fixture.provider, &handle)
        .unwrap();
    let alternate = SourceWorkspaceManager::new(
        fixture.root.join("cache"),
        fixture.root.join("workspaces"),
        Journal::open(&fixture.database()).unwrap(),
        WorkspaceProviderScope {
            identity: "other-provider".to_owned(),
            writable_roots: vec![],
        },
    )
    .unwrap();
    assert!(alternate.status(&mut fixture.provider, &handle).is_err());
}

#[cfg(unix)]
#[test]
fn journal_location_rejects_exposed_storage_and_replaced_database_or_parent() {
    for exposed in ["cache", "workspaces", "provider"] {
        let root = temporary_root("journal-exposed");
        let path = root.join(exposed);
        fs::create_dir(&path).unwrap();
        let journal = Journal::open(&path.join("host.sqlite")).unwrap();
        let mut scope = fixture_scope();
        scope.writable_roots.push(root.join("provider"));
        assert!(SourceWorkspaceManager::new(
            root.join("cache"),
            root.join("workspaces"),
            journal,
            scope
        )
        .is_err());
        fs::remove_dir_all(root).unwrap();
    }
    for replaced in ["database", "parent", "wal", "shm"] {
        let mut fixture = WorkspaceFixture::new("journal-replaced");
        let handle = fixture.prepare().unwrap();
        let path = match replaced {
            "database" => fixture.database(),
            "parent" => fixture.manager.journal_root.clone(),
            "wal" => fixture.manager.journal_root.join("host.sqlite-wal"),
            "shm" => fixture.manager.journal_root.join("host.sqlite-shm"),
            _ => unreachable!(),
        };
        let displaced = fixture.root.join("old-journal-object");
        fs::rename(&path, &displaced).unwrap();
        if replaced == "parent" {
            fs::create_dir(&path).unwrap();
        } else {
            fs::write(&path, b"replacement").unwrap();
        }
        assert!(fixture
            .manager
            .stop(&mut fixture.provider, &handle)
            .is_err());
        assert_eq!(fixture.provider.stop_calls, 0);
        assert!(handle.root().join("source.bin").is_file());
        // Restore only our fixture before SQLite closes its connection.
        if replaced == "parent" {
            fs::remove_dir(&path).unwrap();
        } else {
            fs::remove_file(&path).unwrap();
        }
        fs::rename(displaced, path).unwrap();
    }
}

#[cfg(unix)]
#[test]
fn two_connections_contending_during_create_preserve_one_recoverable_effect() {
    struct Contender<'a> {
        inner: FakeWorkspaceProvider,
        manager: &'a SourceWorkspaceManager,
        source: FakeSourceProvider,
        reference: DigestBoundReferenceV2,
        request: WorkspaceRequest,
        attempted: bool,
    }
    impl WorkspaceProvider for Contender<'_> {
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let mut competing_provider = FakeWorkspaceProvider::default();
            assert!(self
                .manager
                .prepare(
                    &mut self.source,
                    &mut competing_provider,
                    &self.reference,
                    &lease(&self.reference, &self.request),
                    &self.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .is_err());
            assert_eq!(competing_provider.materialize_calls, 0);
            self.attempted = true;
            self.inner.materialize(intent, blob, root, revision)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            self.inner.status(resource)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(resource)
        }
    }
    let mut fixture = WorkspaceFixture::new("two-connections-create");
    let other = SourceWorkspaceManager::new(
        fixture.root.join("cache"),
        fixture.root.join("workspaces"),
        Journal::open(&fixture.database()).unwrap(),
        fixture_scope(),
    )
    .unwrap();
    let mut provider = Contender {
        inner: FakeWorkspaceProvider::default(),
        manager: &other,
        source: FakeSourceProvider {
            bytes: fixture.source.bytes.clone(),
            revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
            acquire_calls: 0,
        },
        reference: fixture.reference.clone(),
        request: fixture.request.clone(),
        attempted: false,
    };
    // The competing observation may win the record CAS; the effect remains discoverable.
    assert!(fixture
        .manager
        .prepare(
            &mut fixture.source,
            &mut provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &FixedClock::new(NOW).unwrap()
        )
        .is_err());
    assert!(provider.attempted);
    assert_eq!(provider.inner.materialize_calls, 1);
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    let recovered = other.recover(&mut provider, &key).unwrap();
    assert_eq!(provider.inner.materialize_calls, 1);
    other.stop(&mut provider, &recovered).unwrap();
}

#[cfg(unix)]
#[test]
fn two_simultaneous_managers_claim_only_one_directory_and_provider_create() {
    use std::sync::{Arc, Barrier, Mutex};
    struct SharedProvider(Arc<Mutex<FakeWorkspaceProvider>>);
    impl WorkspaceProvider for SharedProvider {
        fn scope(&self) -> &str {
            "fixture-provider/v1"
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.0.lock().unwrap().discover(intent)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            self.0
                .lock()
                .unwrap()
                .materialize(intent, blob, root, revision)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            self.0.lock().unwrap().status(resource)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.0.lock().unwrap().stop(resource)
        }
    }
    let mut fixture = WorkspaceFixture::new("simultaneous-managers");
    fixture.prepare().unwrap(); // Prime only the existing source cache.
    fixture.request.generation = 2;
    let shared = Arc::new(Mutex::new(std::mem::take(&mut fixture.provider)));
    let start = Arc::new(Barrier::new(2));
    let mut threads = Vec::new();
    for _ in 0..2 {
        let (root, database, reference, request, bytes) = (
            fixture.root.clone(),
            fixture.database(),
            fixture.reference.clone(),
            fixture.request.clone(),
            fixture.source.bytes.clone(),
        );
        let shared = shared.clone();
        let start = start.clone();
        threads.push(std::thread::spawn(move || {
            let manager = SourceWorkspaceManager::new(
                root.join("cache"),
                root.join("workspaces"),
                Journal::open(&database).unwrap(),
                fixture_scope(),
            )
            .unwrap();
            let mut source = FakeSourceProvider {
                bytes,
                revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
                acquire_calls: 0,
            };
            start.wait();
            manager.prepare(
                &mut source,
                &mut SharedProvider(shared),
                &reference,
                &lease(&reference, &request),
                &request,
                &FixedClock::new(NOW).unwrap(),
            )
        }));
    }
    for thread in threads {
        let _ = thread.join().unwrap();
    }
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    assert_eq!(shared.lock().unwrap().materialize_calls, 2); // One per generation.
    let recovered = fixture
        .manager
        .recover(&mut SharedProvider(shared.clone()), &key)
        .unwrap();
    assert_eq!(shared.lock().unwrap().materialize_calls, 2);
    fixture
        .manager
        .stop(&mut SharedProvider(shared), &recovered)
        .unwrap();
}

#[cfg(unix)]
#[test]
fn clock_reentry_returns_error_without_journal_borrow_panic() {
    use std::cell::Cell;
    struct ReentrantClock<'a> {
        manager: &'a SourceWorkspaceManager,
        called: Cell<bool>,
    }
    impl fkst_local_qa_host::Clock for ReentrantClock<'_> {
        fn now_utc(&self) -> Result<String, RunError> {
            self.called.set(true);
            assert!(self
                .manager
                .recover(&mut FakeWorkspaceProvider::default(), "unused")
                .is_err());
            Ok(NOW.to_owned())
        }
    }
    let mut fixture = WorkspaceFixture::new("reentrant-clock");
    let clock = ReentrantClock {
        manager: &fixture.manager,
        called: Cell::new(false),
    };
    let handle = fixture
        .manager
        .prepare(
            &mut fixture.source,
            &mut fixture.provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &clock,
        )
        .unwrap();
    assert!(clock.called.get());
    fixture
        .manager
        .stop(&mut fixture.provider, &handle)
        .unwrap();
}

#[cfg(unix)]
#[test]
fn process_exit_after_create_reopens_wal_and_recovers_without_a_second_create() {
    const CHILD: &str = "FKST_6092_CREATE_CRASH_CHILD";
    struct DiskProvider {
        registry: PathBuf,
    }
    impl WorkspaceProvider for DiskProvider {
        fn scope(&self) -> &str {
            "fixture-provider/v1"
        }
        fn discover(&mut self, _: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            if !self.registry.join("observed-resource.json").exists() {
                return Ok(WorkspaceDiscovery::Absent);
            }
            let resource: WorkspaceResource =
                serde_json::from_slice(&fs::read(self.registry.join("observed-resource.json"))?)
                    .unwrap();
            Ok(WorkspaceDiscovery::Found(Box::new(self.status(&resource)?)))
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            _: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            assert!(
                !self.registry.join("create-once").exists(),
                "no second provider create is allowed"
            );
            fs::write(self.registry.join("create-once"), b"one effect")?;
            fs::write(root.join("source.bin"), fs::read(blob)?)?;
            let resource = WorkspaceResource {
                intent: intent.clone(),
                provider_identity: "durable-fake-resource".to_owned(),
            };
            fs::write(
                self.registry.join("observed-resource.json"),
                serde_json::to_vec(&resource).unwrap(),
            )?;
            std::process::exit(71);
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            let observed: WorkspaceResource =
                serde_json::from_slice(&fs::read(self.registry.join("observed-resource.json"))?)
                    .unwrap();
            assert_eq!(resource, &observed);
            Ok(WorkspaceProviderStatusReceipt {
                resource: observed,
                status: if self.registry.join("stopped").exists() {
                    WorkspaceProviderStatus::Stopped
                } else {
                    WorkspaceProviderStatus::Active
                },
            })
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            let observed = self.status(resource)?.resource;
            fs::write(self.registry.join("stopped"), b"stopped")?;
            Ok(WorkspaceProviderStopReceipt {
                resource: observed,
                stopped: true,
            })
        }
    }
    if let Some(root) = std::env::var_os(CHILD) {
        let root = PathBuf::from(root);
        let database = PathBuf::from(std::env::var_os("FKST_6092_CREATE_CRASH_DB").unwrap());
        let mut scope = fixture_scope();
        scope.writable_roots.push(root.join("provider"));
        let manager = SourceWorkspaceManager::new(
            root.join("cache"),
            root.join("workspaces"),
            Journal::open(&database).unwrap(),
            scope,
        )
        .unwrap();
        let mut source = FakeSourceProvider {
            bytes: b"immutable fixture source".to_vec(),
            revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
            acquire_calls: 0,
        };
        let reference = source_reference(&source.bytes);
        let request = workspace_request("00000000-0000-4000-8000-000000000401");
        let _ = manager.prepare(
            &mut source,
            &mut DiskProvider {
                registry: root.join("provider"),
            },
            &reference,
            &lease(&reference, &request),
            &request,
            &FixedClock::new(NOW).unwrap(),
        );
        panic!("child must exit in provider before recording resource");
    }
    let mut fixture = WorkspaceFixture::new("process-create-crash");
    let registry = fixture.root.join("provider");
    fs::create_dir(&registry).unwrap();
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "process_exit_after_create_reopens_wal_and_recovers_without_a_second_create",
            "--nocapture",
        ])
        .env(CHILD, &fixture.root)
        .env("FKST_6092_CREATE_CRASH_DB", fixture.database())
        .output()
        .unwrap();
    assert_eq!(
        output.status.code(),
        Some(71),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let mut scope = fixture_scope();
    scope.writable_roots.push(registry.clone());
    fixture.manager.inner = SourceWorkspaceManager::new(
        fixture.root.join("cache"),
        fixture.root.join("workspaces"),
        Journal::open(&fixture.database()).unwrap(),
        scope,
    )
    .unwrap();
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    let before = Journal::open(&fixture.database())
        .unwrap()
        .workspace(&key)
        .unwrap()
        .unwrap();
    assert_eq!(before.state, WorkspaceState::CreateAttempted);
    assert!(before.resource.is_none());
    let mut provider = DiskProvider { registry };
    let handle = fixture.manager.recover(&mut provider, &key).unwrap();
    assert_eq!(
        fs::read(handle.root().join("source.bin")).unwrap(),
        fixture.source.bytes
    );
    fixture.manager.stop(&mut provider, &handle).unwrap();
    assert_eq!(
        fs::read(provider.registry.join("create-once")).unwrap(),
        b"one effect"
    );
    assert!(!handle.root().exists());
}

#[cfg(unix)]
#[test]
fn provider_reentry_returns_error_and_leaves_outer_claim_recoverable() {
    struct Reentrant<'a> {
        inner: FakeWorkspaceProvider,
        manager: &'a SourceWorkspaceManager,
        called: bool,
    }
    impl WorkspaceProvider for Reentrant<'_> {
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            assert!(self
                .manager
                .recover(&mut FakeWorkspaceProvider::default(), &intent.stable_key)
                .is_err());
            self.called = true;
            self.inner.materialize(intent, blob, root, revision)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            self.inner.status(resource)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(resource)
        }
    }
    let mut fixture = WorkspaceFixture::new("provider-reentry");
    let mut provider = Reentrant {
        inner: FakeWorkspaceProvider::default(),
        manager: &fixture.manager,
        called: false,
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
    assert!(provider.called);
    assert_eq!(provider.inner.materialize_calls, 1);
    fixture.manager.stop(&mut provider, &handle).unwrap();
}

#[cfg(unix)]
#[test]
fn review_regression_status_cannot_report_active_after_concurrent_recorded_stop() {
    use std::os::unix::fs::PermissionsExt;
    struct StoppingStatus<'a> {
        inner: FakeWorkspaceProvider,
        other: &'a SourceWorkspaceManager,
        handle: fkst_local_qa_host::source_workspace::WorkspaceHandle,
        stopped: bool,
    }
    impl WorkspaceProvider for StoppingStatus<'_> {
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            self.inner.discover(intent)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            self.inner.materialize(intent, blob, root, revision)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            let old_receipt = self.inner.status(resource)?;
            assert!(self.other.stop(&mut self.inner, &self.handle).is_err());
            self.stopped = true;
            Ok(old_receipt)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(resource)
        }
    }
    let mut fixture = WorkspaceFixture::new("concurrent-stopped-status");
    let handle = fixture.prepare().unwrap();
    let child = handle.root().join("read-only-child");
    fs::create_dir(&child).unwrap();
    fs::write(child.join("payload"), b"retain after stop").unwrap();
    fs::set_permissions(&child, fs::Permissions::from_mode(0o500)).unwrap();
    let other = SourceWorkspaceManager::new(
        fixture.root.join("cache"),
        fixture.root.join("workspaces"),
        Journal::open(&fixture.database()).unwrap(),
        fixture_scope(),
    )
    .unwrap();
    let mut provider = StoppingStatus {
        inner: std::mem::take(&mut fixture.provider),
        other: &other,
        handle: handle.clone(),
        stopped: false,
    };
    let result = fixture.manager.status(&mut provider, &handle);
    fs::set_permissions(&child, fs::Permissions::from_mode(0o700)).unwrap();
    assert!(provider.stopped);
    assert_eq!(provider.inner.stop_calls, 1);
    assert!(handle.root().join(".fkst-workspace.json").exists());
    let key = stable_workspace_key(handle.run_id(), handle.generation());
    assert_eq!(
        Journal::open(&fixture.database())
            .unwrap()
            .workspace(&key)
            .unwrap()
            .unwrap()
            .state,
        WorkspaceState::Stopped
    );
    assert!(
        !matches!(result, Ok(WorkspaceStatus::Active)),
        "stale callback must never override durable stop"
    );
    fixture.manager.stop(&mut provider.inner, &handle).unwrap();
}

#[cfg(unix)]
#[test]
fn review_regression_constructor_rejects_replaced_main_database_with_original_wal() {
    let root = temporary_root("preconstructor-main-swap");
    let journal_root = temporary_root("preconstructor-journal");
    let database = journal_root.join("host.sqlite");
    let journal = Journal::open(&database).unwrap();
    let checkpoint = rusqlite::Connection::open(&database).unwrap();
    checkpoint
        .execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .unwrap();
    let displaced = journal_root.join("original.sqlite");
    fs::rename(&database, &displaced).unwrap();
    fs::copy(&displaced, &database).unwrap();
    assert!(journal_root.join("host.sqlite-wal").is_file());
    assert!(journal_root.join("host.sqlite-shm").is_file());
    let result = SourceWorkspaceManager::new(
        root.join("cache"),
        root.join("workspaces"),
        journal,
        fixture_scope(),
    );
    let rejected = matches!(
        &result,
        Err(RunError::Lifecycle(
            "workspace journal connection location changed"
        ))
    );
    drop(result);
    drop(checkpoint);
    fs::remove_dir_all(root).unwrap();
    fs::remove_dir_all(journal_root).unwrap();
    assert!(
        rejected,
        "shared WAL visibility must not certify a replacement main database"
    );
}

#[cfg(unix)]
#[test]
fn deadline_regression_source_workspace_fractional_boundaries() {
    struct RawClock(&'static str);
    impl fkst_local_qa_host::Clock for RawClock {
        fn now_utc(&self) -> Result<String, RunError> {
            Ok(self.0.to_owned())
        }
    }
    for (now, deadline, allowed) in [
        (NOW, "2026-09-10T00:00:00.5Z", true),
        ("2026-09-10T00:00:00.5Z", NOW, false),
        (NOW, NOW, false),
        ("2026-09-10T00:00:00.5Z", "2026-09-10T00:00:00.5Z", false),
        ("2026-09-10T00:00:00.05Z", "2026-09-10T00:00:00.5Z", true),
        ("malformed", DEADLINE, false),
        (NOW, "malformed", false),
        (NOW, "2026-09-10T00:00:00.0Z", false),
    ] {
        let mut fixture = WorkspaceFixture::new("fractional-deadline");
        fixture.request.deadline_utc = deadline.to_owned();
        let result = fixture.manager.prepare(
            &mut fixture.source,
            &mut fixture.provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &RawClock(now),
        );
        assert_eq!(
            result.is_ok(),
            allowed,
            "now={now}, deadline={deadline}: {result:?}"
        );
        assert_eq!(fixture.source.acquire_calls, usize::from(allowed));
        assert_eq!(fixture.provider.materialize_calls, usize::from(allowed));
    }
}

#[cfg(unix)]
#[test]
fn deadline_regression_workspace_late_returns_preserve_recoverable_ownership() {
    use std::cell::Cell;
    use std::rc::Rc;
    struct CallbackClock(Rc<Cell<bool>>, Option<&'static str>);
    impl fkst_local_qa_host::Clock for CallbackClock {
        fn now_utc(&self) -> Result<String, RunError> {
            if !self.0.get() {
                return Ok(NOW.to_owned());
            }
            self.1
                .map(str::to_owned)
                .ok_or(RunError::Lifecycle("clock unavailable"))
        }
    }
    struct ReturningWorkspace {
        inner: FakeWorkspaceProvider,
        returned: Rc<Cell<bool>>,
        boundary: &'static str,
        interrupt: bool,
    }
    impl WorkspaceProvider for ReturningWorkspace {
        fn scope(&self) -> &str {
            self.inner.scope()
        }
        fn discover(&mut self, intent: &WorkspaceIntent) -> Result<WorkspaceDiscovery, RunError> {
            let receipt = self.inner.discover(intent)?;
            if self.boundary == "discover" {
                self.returned.set(true);
            }
            Ok(receipt)
        }
        fn materialize(
            &mut self,
            intent: &WorkspaceIntent,
            blob: &Path,
            root: &Path,
            revision: &ImmutableRevision,
        ) -> Result<WorkspaceMaterialization, RunError> {
            let receipt = self.inner.materialize(intent, blob, root, revision)?;
            if self.interrupt {
                return Err(RunError::Lifecycle("interrupted create"));
            }
            if self.boundary == "create" {
                self.returned.set(true);
            }
            Ok(receipt)
        }
        fn status(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStatusReceipt, RunError> {
            let receipt = self.inner.status(resource)?;
            if self.boundary == "status" {
                self.returned.set(true);
            }
            Ok(receipt)
        }
        fn stop(
            &mut self,
            resource: &WorkspaceResource,
        ) -> Result<WorkspaceProviderStopReceipt, RunError> {
            self.inner.stop(resource)
        }
    }
    for boundary in ["create", "discover", "status", "absent"] {
        for returned_at in [
            Some(DEADLINE),
            Some("2026-09-11T00:00:00.5Z"),
            Some("malformed"),
            None,
        ] {
            let mut fixture = WorkspaceFixture::new("workspace-late-return");
            let returned = Rc::new(Cell::new(false));
            let clock = CallbackClock(returned.clone(), returned_at);
            let mut provider = ReturningWorkspace {
                inner: FakeWorkspaceProvider::default(),
                returned,
                boundary: "none",
                interrupt: boundary == "discover",
            };
            if matches!(boundary, "discover" | "status") {
                let result = fixture.manager.prepare(
                    &mut fixture.source,
                    &mut provider,
                    &fixture.reference,
                    &lease(&fixture.reference, &fixture.request),
                    &fixture.request,
                    &clock,
                );
                assert_eq!(result.is_ok(), boundary == "status");
                fixture.reopen();
            }
            provider.boundary = if boundary == "absent" {
                "discover"
            } else {
                boundary
            };
            provider.interrupt = false;
            let result = fixture.manager.prepare(
                &mut fixture.source,
                &mut provider,
                &fixture.reference,
                &lease(&fixture.reference, &fixture.request),
                &fixture.request,
                &clock,
            );
            assert!(
                result.is_err(),
                "boundary={boundary}, clock={returned_at:?}: {result:?}"
            );
            if boundary == "absent" {
                assert_eq!(provider.inner.materialize_calls, 0);
                continue;
            }
            let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
            let record = Journal::open(&fixture.database())
                .unwrap()
                .workspace(&key)
                .unwrap()
                .unwrap();
            assert_eq!(record.state, WorkspaceState::Bound);
            assert!(
                record.resource.is_some(),
                "bind must precede late or invalid clock rejection"
            );
            fixture.reopen();
            let handle = fixture.manager.recover(&mut provider, &key).unwrap();
            assert_eq!(
                fixture.manager.status(&mut provider, &handle).unwrap(),
                WorkspaceStatus::Active
            );
            fixture.manager.stop(&mut provider, &handle).unwrap();
            assert_eq!(
                fixture.manager.status(&mut provider, &handle).unwrap(),
                WorkspaceStatus::Stopped
            );
            assert_eq!(provider.inner.materialize_calls, 1);
            assert_eq!(provider.inner.stop_calls, 1);
        }
    }
}

#[cfg(unix)]
#[test]
fn deadline_regression_late_source_acquisition_cannot_create_workspace() {
    use std::cell::Cell;
    struct SourceClock<'a>(&'a Cell<bool>);
    impl fkst_local_qa_host::Clock for SourceClock<'_> {
        fn now_utc(&self) -> Result<String, RunError> {
            Ok(if self.0.get() { DEADLINE } else { NOW }.to_owned())
        }
    }
    struct LateSource<'a>(&'a Cell<bool>, FakeSourceProvider);
    impl SourceProvider for LateSource<'_> {
        fn acquire(&mut self, lease: &SourceObjectLease) -> Result<AcquiredSource, RunError> {
            let acquired = self.1.acquire(lease)?;
            self.0.set(true);
            Ok(acquired)
        }
    }
    let mut fixture = WorkspaceFixture::new("late-source");
    let returned = Cell::new(false);
    let mut source = LateSource(
        &returned,
        FakeSourceProvider {
            bytes: fixture.source.bytes.clone(),
            revision: fixture.source.revision.clone(),
            acquire_calls: 0,
        },
    );
    assert!(fixture
        .manager
        .prepare(
            &mut source,
            &mut fixture.provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &SourceClock(&returned),
        )
        .is_err());
    assert_eq!(source.1.acquire_calls, 1);
    assert_eq!(fixture.provider.materialize_calls, 0);
    assert_eq!(
        fs::read_dir(fixture.root.join("workspaces"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(fs::read_dir(fixture.root.join("cache")).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn source_binding_wrong_valid_commit_rejected_before_workspace_effects() {
    let mut fixture = WorkspaceFixture::new("source-binding-wrong-commit");
    let outside = fixture.outside();
    fixture.source.revision = ImmutableRevision::GitCommit("a".repeat(40));
    assert!(
        fixture.prepare().is_err(),
        "matching bytes do not bind a different commit"
    );
    fixture.assert_untouched(&outside);
    assert_eq!(fixture.source.acquire_calls, 1);
    assert_eq!(
        fs::read_dir(fixture.root.join("workspaces"))
            .unwrap()
            .count(),
        0
    );
    assert_eq!(fs::read_dir(fixture.root.join("cache")).unwrap().count(), 0);
}

#[cfg(unix)]
#[test]
fn source_binding_cached_declared_facts_must_match_expected_binding() {
    for (field, wrong) in [
        (
            "immutable_revision",
            serde_json::to_value(ImmutableRevision::GitCommit("a".repeat(40))).unwrap(),
        ),
        ("provider_scope", serde_json::json!("wrong-scope")),
        (
            "provider_identity",
            serde_json::json!("wrong-provider/object"),
        ),
        ("source_object_id", serde_json::json!("wrong-object")),
        ("raw_digest", serde_json::json!(sha256_digest(b"other"))),
    ] {
        let mut fixture = WorkspaceFixture::new("source-binding-cached-facts");
        let original = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let marker_path = fs::read_dir(fixture.root.join("cache"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(".binding.json"))
            .unwrap();
        let mut marker: serde_json::Value =
            serde_json::from_slice(&fs::read(&marker_path).unwrap()).unwrap();
        marker[field] = wrong;
        fs::write(marker_path, serde_json::to_vec(&marker).unwrap()).unwrap();
        fixture.request.generation = 2;
        fixture.provider.materialize_calls = 0;
        assert!(
            fixture.prepare().is_err(),
            "cached {field} must match explicit expectation"
        );
        fixture.assert_untouched(&outside);
        assert_eq!(fixture.source.acquire_calls, 1);
        assert_eq!(
            fs::read(original.root().join("source.bin")).unwrap(),
            fixture.source.bytes
        );
        assert!(!second_request_path(&fixture.root.join("workspaces"), &fixture.request).exists());
    }
}

#[cfg(unix)]
struct DeclaredSourceProvider {
    acquired: AcquiredSource,
    calls: usize,
}

#[cfg(unix)]
impl SourceProvider for DeclaredSourceProvider {
    fn acquire(&mut self, _: &SourceObjectLease) -> Result<AcquiredSource, RunError> {
        self.calls += 1;
        Ok(self.acquired.clone())
    }
}

#[cfg(unix)]
fn declared_source(fixture: &WorkspaceFixture) -> DeclaredSourceProvider {
    DeclaredSourceProvider {
        acquired: AcquiredSource {
            source_object_id: fixture.reference.id.clone(),
            immutable_revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
            provider_scope: "fixture-source/v1".to_owned(),
            provider_identity: "source-provider/object-001".to_owned(),
            bytes: fixture.source.bytes.clone(),
        },
        calls: 0,
    }
}

#[cfg(unix)]
#[test]
fn source_binding_fresh_declared_identity_and_revision_mismatches_have_no_effects() {
    for field in [
        "object",
        "scope",
        "identity",
        "empty-scope",
        "empty-identity",
        "revision",
        "floating",
        "bytes",
    ] {
        let mut fixture = WorkspaceFixture::new("source-binding-fresh");
        let outside = fixture.outside();
        let mut source = declared_source(&fixture);
        match field {
            "object" => source.acquired.source_object_id = "another-object".into(),
            "scope" => source.acquired.provider_scope = "another-scope".into(),
            "identity" => source.acquired.provider_identity = "another-identity".into(),
            "empty-scope" => source.acquired.provider_scope.clear(),
            "empty-identity" => source.acquired.provider_identity.clear(),
            "revision" => {
                source.acquired.immutable_revision = ImmutableRevision::GitCommit("b".repeat(40))
            }
            "floating" => {
                source.acquired.immutable_revision = ImmutableRevision::GitCommit("main".into())
            }
            "bytes" => source.acquired.bytes = b"other bytes".to_vec(),
            _ => unreachable!(),
        }
        let result = fixture.manager.prepare(
            &mut source,
            &mut fixture.provider,
            &fixture.reference,
            &lease(&fixture.reference, &fixture.request),
            &fixture.request,
            &FixedClock::new(NOW).unwrap(),
        );
        assert!(result.is_err(), "field={field}");
        assert_eq!(source.calls, 1);
        fixture.assert_untouched(&outside);
        assert_eq!(fs::read_dir(fixture.root.join("cache")).unwrap().count(), 0);
        assert_eq!(
            fs::read_dir(fixture.root.join("workspaces"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn source_binding_reference_shape_and_exact_match_checked_before_acquisition() {
    for field in [
        "kind",
        "id",
        "schema",
        "digest",
        "empty-id",
        "bad-id",
        "empty-schema",
        "bad-schema",
        "bad-digest",
        "bad-raw",
        "floating",
        "bad-object-revision",
        "empty-scope",
        "empty-identity",
    ] {
        let mut fixture = WorkspaceFixture::new("source-binding-input");
        let outside = fixture.outside();
        let mut expected = lease(&fixture.reference, &fixture.request);
        match field {
            "kind" => fixture.reference.kind = "environment".into(),
            "id" => fixture.reference.id = "different-id".into(),
            "schema" => fixture.reference.schema_version = "qa.other/v2".into(),
            "digest" => fixture.reference.content_digest = sha256_digest(b"another reference"),
            "empty-id" => expected.binding.reference.id.clear(),
            "bad-id" => expected.binding.reference.id = "../escape".into(),
            "empty-schema" => expected.binding.reference.schema_version.clear(),
            "bad-schema" => expected.binding.reference.schema_version = " invalid schema".into(),
            "bad-digest" => expected.binding.reference.content_digest = "not-digest".into(),
            "bad-raw" => expected.binding.expected_raw_digest = "not-digest".into(),
            "floating" => {
                expected.binding.expected_revision = ImmutableRevision::GitCommit("main".into())
            }
            "bad-object-revision" => {
                expected.binding.expected_revision = ImmutableRevision::ObjectDigest("bad".into())
            }
            "empty-scope" => expected.binding.expected_provider_scope.clear(),
            "empty-identity" => expected.binding.expected_provider_identity = "  ".into(),
            _ => unreachable!(),
        }
        if !matches!(field, "kind" | "id" | "schema" | "digest") {
            fixture.reference = expected.binding.reference.clone();
        }
        assert!(
            fixture
                .manager
                .prepare(
                    &mut fixture.source,
                    &mut fixture.provider,
                    &fixture.reference,
                    &expected,
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .is_err(),
            "field={field}"
        );
        assert_eq!(fixture.source.acquire_calls, 0);
        fixture.assert_untouched(&outside);
        assert_eq!(fs::read_dir(fixture.root.join("cache")).unwrap().count(), 0);
        assert_eq!(
            fs::read_dir(fixture.root.join("workspaces"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[cfg(unix)]
#[test]
fn source_binding_reference_and_object_revision_digests_are_distinct_from_raw_bytes() {
    for object_revision in [false, true] {
        let mut fixture = WorkspaceFixture::new("source-binding-distinct-digests");
        let outside = fixture.outside();
        let raw_digest = sha256_digest(&fixture.source.bytes);
        fixture.reference.content_digest = sha256_digest(b"source reference metadata");
        fixture.reference.schema_version = "local.fixture-source/v7".into();
        let mut expected = lease(&fixture.reference, &fixture.request);
        expected.binding.expected_raw_digest = raw_digest.clone();
        expected.binding.source_object_id = "separate-raw-object-id".into();
        let mut source = declared_source(&fixture);
        source.acquired.source_object_id = expected.binding.source_object_id.clone();
        if object_revision {
            expected.binding.expected_revision =
                ImmutableRevision::ObjectDigest(sha256_digest(b"declared snapshot tree"));
            source.acquired.immutable_revision = expected.binding.expected_revision.clone();
        }
        assert_ne!(
            expected.binding.reference.content_digest,
            expected.binding.expected_raw_digest
        );
        let handle = fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &fixture.reference,
                &expected,
                &fixture.request,
                &FixedClock::new(NOW).unwrap(),
            )
            .unwrap();
        assert_eq!(
            fs::read(handle.root().join("source.bin")).unwrap(),
            fixture.source.bytes
        );
        fixture.reopen();
        assert_eq!(
            fixture
                .manager
                .prepare(
                    &mut source,
                    &mut fixture.provider,
                    &fixture.reference,
                    &expected,
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .unwrap(),
            handle
        );
        assert_eq!(source.calls, 1);
        let original_bytes = fs::read(handle.root().join("source.bin")).unwrap();
        let mut changed_reference = fixture.reference.clone();
        changed_reference.content_digest = raw_digest.clone();
        assert!(fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &changed_reference,
                &expected,
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        let mut changed_binding = expected.clone();
        changed_binding.binding.expected_raw_digest = fixture.reference.content_digest.clone();
        assert!(fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &fixture.reference,
                &changed_binding,
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        assert_eq!(source.calls, 1);
        assert_eq!(fixture.provider.materialize_calls, 1);
        assert_eq!(
            fs::read(handle.root().join("source.bin")).unwrap(),
            original_bytes
        );
        fixture.request.generation = 2;
        expected.generation = 2;
        source.acquired.immutable_revision = if object_revision {
            ImmutableRevision::ObjectDigest(raw_digest)
        } else {
            ImmutableRevision::GitCommit("c".repeat(40))
        };
        // A new binding has no receipt; declaring a different revision cannot borrow the old receipt.
        expected.binding.reference.id = "second-reference".into();
        fixture.reference = expected.binding.reference.clone();
        assert!(fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &fixture.reference,
                &expected,
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        assert_eq!(source.calls, 2);
        assert_eq!(fixture.provider.materialize_calls, 1);
        fixture
            .manager
            .stop(&mut fixture.provider, &handle)
            .unwrap();
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn source_binding_each_new_binding_freshly_acquires_before_sharing_raw_bytes() {
    for field in [
        "object",
        "revision",
        "scope",
        "identity",
        "reference-id",
        "reference-schema",
        "reference-digest",
    ] {
        let mut fixture = WorkspaceFixture::new("source-binding-byte-sharing");
        let first = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let original = fs::read(first.root().join("source.bin")).unwrap();
        let mut expected = lease(&fixture.reference, &fixture.request);
        let mut source = declared_source(&fixture);
        match field {
            "object" => expected.binding.source_object_id = "second-object".into(),
            "revision" => {
                expected.binding.expected_revision = ImmutableRevision::GitCommit("d".repeat(40))
            }
            "scope" => expected.binding.expected_provider_scope = "second-scope".into(),
            "identity" => {
                expected.binding.expected_provider_identity = "second-provider/object".into()
            }
            "reference-id" => expected.binding.reference.id = "second-ref".into(),
            "reference-schema" => {
                expected.binding.reference.schema_version = "local.fixture/v8".into()
            }
            "reference-digest" => {
                expected.binding.reference.content_digest = sha256_digest(b"second reference")
            }
            _ => unreachable!(),
        }
        fixture.reference = expected.binding.reference.clone();
        assert!(
            fixture
                .manager
                .prepare(
                    &mut source,
                    &mut fixture.provider,
                    &fixture.reference,
                    &expected,
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .is_err(),
            "same run/generation binding must be immutable: {field}"
        );
        assert_eq!(source.calls, 0);
        fixture.request.generation = 2;
        expected.generation = 2;
        source.acquired.source_object_id = expected.binding.source_object_id.clone();
        source.acquired.immutable_revision = expected.binding.expected_revision.clone();
        source.acquired.provider_scope = expected.binding.expected_provider_scope.clone();
        source.acquired.provider_identity = expected.binding.expected_provider_identity.clone();
        // Failure must retain old bytes and cannot silently authorize the new binding.
        let valid = source.acquired.clone();
        source.acquired.provider_identity = "wrong-but-nonempty".into();
        assert!(fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &fixture.reference,
                &expected,
                &fixture.request,
                &FixedClock::new(NOW).unwrap()
            )
            .is_err());
        assert_eq!(source.calls, 1);
        assert_eq!(fixture.provider.materialize_calls, 1);
        assert!(!second_request_path(&fixture.root.join("workspaces"), &fixture.request).exists());
        source.acquired = valid;
        let second = fixture
            .manager
            .prepare(
                &mut source,
                &mut fixture.provider,
                &fixture.reference,
                &expected,
                &fixture.request,
                &FixedClock::new(NOW).unwrap(),
            )
            .unwrap();
        assert_eq!(source.calls, 2);
        assert_eq!(fixture.provider.materialize_calls, 2);
        let cache = fs::read_dir(fixture.root.join("cache"))
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect::<Vec<_>>();
        assert_eq!(
            cache
                .iter()
                .filter(|path| path.extension().is_some_and(|ext| ext == "source"))
                .count(),
            1
        );
        assert_eq!(
            cache
                .iter()
                .filter(|path| path.to_string_lossy().ends_with(".binding.json"))
                .count(),
            2
        );
        fixture.reopen();
        assert_eq!(
            fixture
                .manager
                .prepare(
                    &mut source,
                    &mut fixture.provider,
                    &fixture.reference,
                    &expected,
                    &fixture.request,
                    &FixedClock::new(NOW).unwrap()
                )
                .unwrap(),
            second
        );
        assert_eq!(source.calls, 2);
        assert_eq!(fs::read(first.root().join("source.bin")).unwrap(), original);
        fixture.manager.stop(&mut fixture.provider, &first).unwrap();
        fixture
            .manager
            .stop(&mut fixture.provider, &second)
            .unwrap();
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn source_binding_old_partial_and_corrupt_cache_is_unavailable_without_repair() {
    for damage in [
        "v1",
        "missing-blob",
        "missing-marker",
        "missing-receipt",
        "invalid-receipt",
        "receipt-owner",
        "receipt-digest",
        "marker-digest",
    ] {
        let mut fixture = WorkspaceFixture::new("source-binding-cache-unavailable");
        let original = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let cache = fixture.root.join("cache");
        let blob = cache.join(format!(
            "{}.source",
            fixture
                .reference
                .content_digest
                .strip_prefix("sha256:")
                .unwrap()
        ));
        let marker = blob.with_extension("source.json");
        let receipt = fs::read_dir(&cache)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| path.to_string_lossy().ends_with(".binding.json"))
            .unwrap();
        match damage {
            "v1" => fs::write(&marker, serde_json::to_vec(&serde_json::json!({
                "schema_version": "fkst.local-qa-source-cache/v1",
                "source_object_id": fixture.reference.id,
                "content_digest": fixture.reference.content_digest,
                "immutable_revision": format!("git:{COMMIT}"),
                "provider_identity": "source-provider/object-001"
            })).unwrap()).unwrap(),
            "missing-blob" => fs::remove_file(&blob).unwrap(),
            "missing-marker" => fs::remove_file(&marker).unwrap(),
            "missing-receipt" => fs::remove_file(&receipt).unwrap(),
            "invalid-receipt" => fs::write(&receipt, b"{partial").unwrap(),
            "receipt-owner" | "receipt-digest" => {
                let mut value: serde_json::Value = serde_json::from_slice(&fs::read(&receipt).unwrap()).unwrap();
                if damage == "receipt-owner" { value["binding"]["reference"]["id"] = "foreign-reference".into(); }
                else { value["binding"]["expected_raw_digest"] = sha256_digest(b"other").into(); }
                fs::write(&receipt, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            "marker-digest" => fs::write(&marker, serde_json::to_vec(&serde_json::json!({
                "schema_version": "fkst.local-qa-source-cache/v2", "content_digest": sha256_digest(b"other")
            })).unwrap()).unwrap(),
            _ => unreachable!(),
        }
        let snapshot = || {
            let mut files = fs::read_dir(&cache)
                .unwrap()
                .map(|entry| {
                    let path = entry.unwrap().path();
                    (
                        path.file_name().unwrap().to_owned(),
                        fs::read(path).unwrap(),
                    )
                })
                .collect::<Vec<_>>();
            files.sort();
            files
        };
        let before = snapshot();
        fixture.provider.materialize_calls = 0;
        assert!(fixture.prepare().is_err(), "damage={damage}");
        assert_eq!(fixture.source.acquire_calls, 1);
        fixture.assert_untouched(&outside);
        assert_eq!(snapshot(), before, "no delete/rebuild/upgrade: {damage}");
        assert_eq!(
            fs::read(original.root().join("source.bin")).unwrap(),
            fixture.source.bytes
        );
        // Cache availability does not remove the independent obligation to stop.
        fixture
            .manager
            .stop(&mut fixture.provider, &original)
            .unwrap();
    }
}

#[cfg(unix)]
#[test]
fn source_binding_v8_legacy_lifecycle_preserves_saved_bytes_and_denies_prepare() {
    for state in ["bound", "stop_attempted", "create_attempted"] {
        let mut fixture = WorkspaceFixture::new("source-binding-v8-legacy");
        let original = fixture.prepare().unwrap();
        let outside = fixture.outside();
        let original_marker = fs::read(original.root().join(".fkst-workspace.json")).unwrap();
        let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
        for resource in fixture.provider.resources.values_mut() {
            resource.intent.source_binding = None;
        }
        let legacy_resource = fixture.provider.resources.values().next().unwrap().clone();
        let legacy_intent = format!(
            " \n{}\n ",
            serde_json::to_string_pretty(&legacy_resource.intent).unwrap()
        );
        let legacy_resource_bytes = format!(
            " \n{}\n ",
            serde_json::to_string_pretty(&legacy_resource).unwrap()
        );
        assert!(!legacy_intent.contains("source_binding"));
        assert!(!legacy_resource_bytes.contains("source_binding"));
        let connection = rusqlite::Connection::open(fixture.database()).unwrap();
        connection.execute("UPDATE workspace_ownership SET intent_json=?1, resource_json=?2, state=?3 WHERE stable_key=?4",
            rusqlite::params![legacy_intent, legacy_resource_bytes, state, key]).unwrap();
        if state == "create_attempted" {
            connection.execute("UPDATE workspace_ownership SET resource_json=NULL, provider_identity=NULL WHERE stable_key=?1", [&key]).unwrap();
        }
        connection.pragma_update(None, "user_version", 8).unwrap();
        let saved = || {
            connection.query_row("SELECT intent_json, resource_json FROM workspace_ownership WHERE stable_key=?1", [&key],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))).unwrap()
        };
        let before = saved();
        fixture.reopen();
        assert_eq!(
            connection
                .pragma_query_value(None, "user_version", |row| row.get::<_, i64>(0))
                .unwrap(),
            9
        );
        assert_eq!(saved(), before);
        let acquisitions = fixture.source.acquire_calls;
        let effects = fixture.provider.materialize_calls;
        assert!(
            fixture.prepare().is_err(),
            "legacy {state} must not acquire/create"
        );
        assert_eq!(fixture.source.acquire_calls, acquisitions);
        assert_eq!(fixture.provider.materialize_calls, effects);
        assert_eq!(saved(), before);
        if state == "stop_attempted" {
            fixture
                .provider
                .active
                .insert(original.workspace_provider_identity().to_owned(), false);
        }
        let recovered = fixture
            .manager
            .recover(&mut fixture.provider, &key)
            .unwrap();
        assert_eq!(
            fs::read(recovered.root().join(".fkst-workspace.json")).unwrap(),
            original_marker
        );
        assert_eq!(saved().0, legacy_intent);
        if state != "create_attempted" {
            assert_eq!(saved(), before);
        }
        let after_recover = saved();
        assert!(!after_recover.1.as_ref().unwrap().contains("source_binding"));
        let status = fixture
            .manager
            .status(&mut fixture.provider, &recovered)
            .unwrap();
        assert_eq!(
            status,
            if state == "stop_attempted" {
                WorkspaceStatus::Stopped
            } else {
                WorkspaceStatus::Active
            }
        );
        assert_eq!(
            saved(),
            after_recover,
            "status must preserve actual saved legacy bytes"
        );
        fixture
            .manager
            .stop(&mut fixture.provider, &recovered)
            .unwrap();
        assert_eq!(
            saved(),
            after_recover,
            "stop SQL must preserve actual saved legacy bytes"
        );
        fixture.reopen();
        let stopped = fixture
            .manager
            .recover(&mut fixture.provider, &key)
            .unwrap();
        assert_eq!(
            fixture
                .manager
                .status(&mut fixture.provider, &stopped)
                .unwrap(),
            WorkspaceStatus::Stopped
        );
        assert!(
            fixture
                .manager
                .stop(&mut fixture.provider, &stopped)
                .unwrap()
                .already_stopped
        );
        assert_eq!(saved(), after_recover);
        assert_eq!(fixture.provider.materialize_calls, effects);
        assert_eq!(
            fixture.provider.stop_calls,
            usize::from(state != "stop_attempted")
        );
        assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
    }
}

#[cfg(unix)]
#[test]
fn source_binding_final_clock_callback_revalidates_receipt_and_preserves_ownership() {
    use std::cell::Cell;
    struct TamperingClock {
        reads: Cell<usize>,
        receipt: PathBuf,
    }
    impl fkst_local_qa_host::Clock for TamperingClock {
        fn now_utc(&self) -> Result<String, RunError> {
            let read = self.reads.get() + 1;
            self.reads.set(read);
            if read == 4 {
                let mut value: serde_json::Value =
                    serde_json::from_slice(&fs::read(&self.receipt)?).unwrap();
                value["provider_identity"] = "changed-after-materialization".into();
                fs::write(&self.receipt, serde_json::to_vec(&value).unwrap())?;
            }
            Ok(NOW.to_owned())
        }
    }
    let mut fixture = WorkspaceFixture::new("source-binding-final-callback");
    let first = fixture.prepare().unwrap();
    let outside = fixture.outside();
    fixture.request.generation = 2;
    let receipt = fs::read_dir(fixture.root.join("cache"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .find(|path| path.to_string_lossy().ends_with(".binding.json"))
        .unwrap();
    let clock = TamperingClock {
        reads: Cell::new(0),
        receipt,
    };
    let result = fixture.manager.prepare(
        &mut fixture.source,
        &mut fixture.provider,
        &fixture.reference,
        &lease(&fixture.reference, &fixture.request),
        &fixture.request,
        &clock,
    );
    assert_eq!(clock.reads.get(), 4);
    assert!(
        result.is_err(),
        "source receipt must be checked after the last external callback"
    );
    let key = stable_workspace_key(&fixture.request.run_id, fixture.request.generation);
    let recovered = fixture
        .manager
        .recover(&mut fixture.provider, &key)
        .unwrap();
    assert_eq!(fixture.provider.materialize_calls, 2);
    assert_eq!(fixture.source.acquire_calls, 1);
    fixture
        .manager
        .stop(&mut fixture.provider, &recovered)
        .unwrap();
    fixture.manager.stop(&mut fixture.provider, &first).unwrap();
    assert_eq!(fs::read(outside.join("sentinel")).unwrap(), b"preserve");
}

#[cfg(unix)]
#[test]
fn source_cache_interrupted_publication_requires_exact_intent_and_fresh_acquisition() {
    for state in ["intent", "raw", "marker"] {
        let mut fixture = WorkspaceFixture::new("source-cache-publication-recovery");
        let binding = lease(&fixture.reference, &fixture.request).binding;
        let cache = fixture.root.join("cache");
        let key = sha256_digest(&serde_json::to_vec(&binding).unwrap());
        let intent = cache.join(format!(
            "{}.publication.json",
            key.strip_prefix("sha256:").unwrap()
        ));
        fs::write(
            &intent,
            serde_json::to_vec(&serde_json::json!({
                "schema_version": "fkst.local-qa-source-publication/v1",
                "binding": binding,
            }))
            .unwrap(),
        )
        .unwrap();
        let blob = cache.join(format!(
            "{}.source",
            binding.expected_raw_digest.strip_prefix("sha256:").unwrap()
        ));
        if state != "intent" {
            fs::write(&blob, &fixture.source.bytes).unwrap();
        }
        if state == "marker" {
            fs::write(
                blob.with_extension("source.json"),
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": "fkst.local-qa-source-cache/v2",
                    "content_digest": binding.expected_raw_digest,
                }))
                .unwrap(),
            )
            .unwrap();
        }
        // An unrelated abandoned temporary artifact is never a recovery credential.
        let stale = cache.join(".unrelated-publication.tmp");
        fs::write(&stale, b"retain unrelated partial bytes").unwrap();
        let before = fs::read(&intent).unwrap();
        fixture.source.revision = ImmutableRevision::GitCommit("a".repeat(40));
        assert!(fixture.prepare().is_err());
        assert_eq!(fixture.source.acquire_calls, 1, "state={state}");
        assert_eq!(fixture.provider.materialize_calls, 0);
        assert_eq!(fs::read(&intent).unwrap(), before);
        fixture.source.revision = ImmutableRevision::GitCommit(COMMIT.into());
        fixture.reopen();
        let handle = fixture.prepare().unwrap();
        assert_eq!(fixture.source.acquire_calls, 2, "state={state}");
        assert_eq!(fs::read(&blob).unwrap(), fixture.source.bytes);
        assert_eq!(fs::read(&stale).unwrap(), b"retain unrelated partial bytes");
        assert_eq!(fixture.prepare().unwrap(), handle);
        assert_eq!(fixture.source.acquire_calls, 2);
    }
}

#[cfg(unix)]
#[test]
fn source_cache_orphan_or_foreign_intent_never_authorizes_partial_repair() {
    for evidence in ["absent", "foreign", "truncated"] {
        let mut fixture = WorkspaceFixture::new("source-cache-orphan");
        let binding = lease(&fixture.reference, &fixture.request).binding;
        let cache = fixture.root.join("cache");
        let blob = cache.join(format!(
            "{}.source",
            binding.expected_raw_digest.strip_prefix("sha256:").unwrap()
        ));
        fs::write(&blob, &fixture.source.bytes).unwrap();
        let key = sha256_digest(&serde_json::to_vec(&binding).unwrap());
        let intent = cache.join(format!(
            "{}.publication.json",
            key.strip_prefix("sha256:").unwrap()
        ));
        if evidence == "foreign" {
            let mut foreign = binding.clone();
            foreign.expected_provider_identity = "other-provider".into();
            fs::write(
                &intent,
                serde_json::to_vec(&serde_json::json!({
                    "schema_version": "fkst.local-qa-source-publication/v1", "binding": foreign,
                }))
                .unwrap(),
            )
            .unwrap();
        } else if evidence == "truncated" {
            fs::write(&intent, b"{partial").unwrap();
        }
        assert!(fixture.prepare().is_err(), "evidence={evidence}");
        assert_eq!(fixture.source.acquire_calls, 0);
        assert_eq!(fixture.provider.materialize_calls, 0);
        assert_eq!(fs::read(&blob).unwrap(), fixture.source.bytes);
    }
}

struct TestManager {
    inner: SourceWorkspaceManager,
    journal_root: PathBuf,
}
impl std::ops::Deref for TestManager {
    type Target = SourceWorkspaceManager;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}
impl Drop for TestManager {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.journal_root);
    }
}
fn test_manager(
    cache: impl AsRef<Path>,
    workspace: impl AsRef<Path>,
) -> Result<TestManager, RunError> {
    let journal_root = temporary_root("workspace-journal");
    let journal = Journal::open(&journal_root.join("host.sqlite"))?;
    match SourceWorkspaceManager::new(
        cache,
        workspace,
        journal,
        WorkspaceProviderScope {
            identity: "fixture-provider/v1".to_owned(),
            writable_roots: vec![],
        },
    ) {
        Ok(inner) => Ok(TestManager {
            inner,
            journal_root,
        }),
        Err(error) => {
            fs::remove_dir_all(journal_root)?;
            Err(error)
        }
    }
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
        binding: TrustedLocalSourceBinding {
            reference: reference.clone(),
            source_object_id: reference.id.clone(),
            expected_raw_digest: reference.content_digest.clone(),
            expected_revision: ImmutableRevision::GitCommit(COMMIT.to_owned()),
            expected_provider_scope: "fixture-source/v1".to_owned(),
            expected_provider_identity: "source-provider/object-001".to_owned(),
        },
        run_id: request.run_id.clone(),
        generation: request.generation,
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
    static NEXT_ROOT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let unique = NEXT_ROOT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let path = std::env::temp_dir().join(format!(
        "fkst-{name}-{}-{nonce}-{unique}",
        std::process::id()
    ));
    fs::create_dir(&path).unwrap();
    // Resolve only this freshly created trusted fixture root (macOS /var alias).
    fs::canonicalize(path).unwrap()
}
