#![cfg(all(unix, feature = "local-bundle-provider"))]

use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

use fkst_local_qa_host::source_workspace::local_bundle::{
    local_bundle_providers, LocalBundleConfig, LocalBundleLimits, LocalBundleObject,
};
use fkst_local_qa_host::source_workspace::*;
use fkst_local_qa_host::{FixedClock, Journal};
use fkst_qa_contracts::{sha256_digest, DigestBoundReferenceV2};

const DEADLINE: &str = "2099-01-01T00:00:00Z";
const RUN: &str = "00000000-0000-4000-8000-000000000101";
const OTHER: &str = "00000000-0000-4000-8000-000000000102";

struct Fixture {
    root: PathBuf,
    git: PathBuf,
    object: LocalBundleObject,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn git(executable: &Path, cwd: &Path, args: &[&str]) -> String {
    let output = Command::new(executable)
        .current_dir(cwd)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("HOME", cwd)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_AUTHOR_NAME", "Local Fixture")
        .env("GIT_AUTHOR_EMAIL", "fixture@example.invalid")
        .env("GIT_COMMITTER_NAME", "Local Fixture")
        .env("GIT_COMMITTER_EMAIL", "fixture@example.invalid")
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let temporary_root = std::env::temp_dir().canonicalize().unwrap();
        let root = loop {
            let root = temporary_root.join(format!(
                "6092-bundle-provider-fixture-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match fs::create_dir(&root) {
                Ok(()) => break root,
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("fixture allocation: {error}"),
            }
        };
        fs::set_permissions(&root, fs::Permissions::from_mode(0o700)).unwrap();
        for name in [
            "original",
            "store",
            "state",
            "workspaces",
            "cache",
            "journal",
            "empty-template",
        ] {
            fs::create_dir(root.join(name)).unwrap();
            fs::set_permissions(root.join(name), fs::Permissions::from_mode(0o700)).unwrap();
        }
        #[cfg(target_os = "macos")]
        let git_exe = PathBuf::from("/Library/Developer/CommandLineTools/usr/bin/git");
        #[cfg(not(target_os = "macos"))]
        let git_exe = PathBuf::from("/usr/bin/git");
        let repo = root.join("original");
        git(&git_exe, &repo, &["init", "--template=", "."]);
        fs::write(repo.join("hello.txt"), b"immutable source\n").unwrap();
        git(&git_exe, &repo, &["add", "hello.txt"]);
        git(&git_exe, &repo, &["commit", "-m", "fixture"]);
        let commit = git(&git_exe, &repo, &["rev-parse", "HEAD"]);
        let bundle = root.join("store/source.bundle");
        git(
            &git_exe,
            &repo,
            &["bundle", "create", bundle.to_str().unwrap(), "HEAD"],
        );
        let digest = sha256_digest(&fs::read(&bundle).unwrap());
        let object = LocalBundleObject {
            binding: TrustedLocalSourceBinding {
                reference: DigestBoundReferenceV2 {
                    kind: "source".into(),
                    id: "object-one".into(),
                    schema_version: "qa.source-object/v1".into(),
                    content_digest: sha256_digest(b"local-source-metadata"),
                },
                source_object_id: "object-one".into(),
                expected_raw_digest: digest,
                expected_revision: ImmutableRevision::GitCommit(commit),
                expected_provider_scope: "fixed-store/v1".into(),
                expected_provider_identity: "fixed-store/object-one".into(),
            },
            file_name: "source.bundle".into(),
            object_format: "git_bundle".into(),
            revision_strategy: "exact_commit".into(),
        };
        Self {
            root,
            git: git_exe,
            object,
        }
    }
    fn refresh_bundle(&mut self) {
        let repo = self.root.join("original");
        git(&self.git, &repo, &["add", "-A"]);
        git(&self.git, &repo, &["commit", "-m", "fixture content"]);
        git(
            &self.git,
            &repo,
            &[
                "bundle",
                "create",
                self.root.join("store/source.bundle").to_str().unwrap(),
                "HEAD",
            ],
        );
        self.object.binding.expected_revision =
            ImmutableRevision::GitCommit(git(&self.git, &repo, &["rev-parse", "HEAD"]));
        self.object.binding.expected_raw_digest =
            sha256_digest(&fs::read(self.root.join("store/source.bundle")).unwrap());
    }
    fn config(&self) -> LocalBundleConfig {
        LocalBundleConfig {
            source_store: self.root.join("store"),
            state_root: self.root.join("state"),
            workspace_root: self.root.join("workspaces"),
            cache_root: self.root.join("cache"),
            journal_parent: self.root.join("journal"),
            additional_writable_roots: vec![],
            git_executable: self.git.clone(),
            scope: "local-bundle/v1".into(),
            objects: vec![self.object.clone()],
            limits: LocalBundleLimits::default(),
        }
    }
    fn manager(&self) -> SourceWorkspaceManager {
        SourceWorkspaceManager::new(
            self.root.join("cache"),
            self.root.join("workspaces"),
            Journal::open(&self.root.join("journal/host.sqlite")).unwrap(),
            WorkspaceProviderScope {
                identity: "local-bundle/v1".into(),
                writable_roots: vec![self.root.join("state")],
            },
        )
        .unwrap()
    }
    fn lease(&self, run: &str) -> SourceObjectLease {
        SourceObjectLease {
            lease_id: format!("lease/{run}"),
            binding: self.object.binding.clone(),
            run_id: run.into(),
            generation: 1,
            deadline_utc: DEADLINE.into(),
        }
    }
    fn request(&self, run: &str) -> WorkspaceRequest {
        WorkspaceRequest {
            run_id: run.into(),
            generation: 1,
            deadline_utc: DEADLINE.into(),
        }
    }
    fn prepare(
        &self,
        manager: &SourceWorkspaceManager,
        source: &mut impl SourceProvider,
        workspace: &mut impl WorkspaceProvider,
        run: &str,
    ) -> Result<WorkspaceHandle, fkst_local_qa_host::RunError> {
        manager.prepare(
            source,
            workspace,
            &self.object.binding.reference,
            &self.lease(run),
            &self.request(run),
            &FixedClock::new("2026-09-10T00:00:00Z").unwrap(),
        )
    }
}

#[test]
fn bundle_checks_real_commit_and_materializes_independent_detached_workspaces() {
    let f = Fixture::new();
    let original_head = fs::read(f.root.join("original/.git/HEAD")).unwrap();
    let manager = f.manager();
    let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
    let first = f
        .prepare(&manager, &mut source, &mut workspace, RUN)
        .unwrap();
    assert_eq!(
        fs::read(first.root().join("hello.txt")).unwrap(),
        b"immutable source\n"
    );
    let ImmutableRevision::GitCommit(commit) = &f.object.binding.expected_revision else {
        unreachable!()
    };
    assert_eq!(
        fs::read_to_string(first.root().join(".git/HEAD"))
            .unwrap()
            .trim(),
        commit
    );
    assert_eq!(git(&f.git, first.root(), &["rev-parse", "HEAD"]), *commit);
    assert!(git(&f.git, first.root(), &["diff", "--exit-code"]).is_empty());
    assert_eq!(
        f.prepare(&manager, &mut source, &mut workspace, RUN)
            .unwrap(),
        first
    );
    // Raw cache reuse works without reacquiring the original source-store file.
    fs::remove_file(f.root.join("store/source.bundle")).unwrap();
    let second = f
        .prepare(&manager, &mut source, &mut workspace, OTHER)
        .unwrap();
    assert_ne!(first.root(), second.root());
    assert_ne!(
        fs::metadata(first.root().join("hello.txt")).unwrap().ino(),
        fs::metadata(second.root().join("hello.txt")).unwrap().ino()
    );
    fs::write(first.root().join("hello.txt"), b"run one mutation").unwrap();
    assert_eq!(
        fs::read(second.root().join("hello.txt")).unwrap(),
        b"immutable source\n"
    );
    assert!(!first.root().join(".git/objects/info/alternates").exists());
    assert_eq!(
        fs::read(f.root.join("original/.git/HEAD")).unwrap(),
        original_head
    );
    drop(manager);
    drop(source);
    drop(workspace);
    let manager = f.manager();
    let (_, mut workspace) = local_bundle_providers(f.config()).unwrap();
    let recovered = manager
        .recover(&mut workspace, &stable_workspace_key(RUN, 1))
        .unwrap();
    assert_eq!(recovered, first);
    assert_eq!(
        manager.status(&mut workspace, &second).unwrap(),
        WorkspaceStatus::Active
    );
    manager.stop(&mut workspace, &recovered).unwrap();
    assert!(!first.root().exists());
    assert!(second.root().exists());
    assert!(
        manager
            .stop(&mut workspace, &first)
            .unwrap()
            .already_stopped
    );
}

#[test]
fn digest_and_input_size_reject_before_git_or_workspace_effects() {
    for oversized in [false, true] {
        let f = Fixture::new();
        let manager = f.manager();
        let mut config = f.config();
        if oversized {
            config.limits.max_bundle_bytes = 1;
        } else {
            fs::write(f.root.join("store/source.bundle"), b"bad bytes").unwrap();
        }
        let (mut source, mut workspace) = local_bundle_providers(config).unwrap();
        assert!(f
            .prepare(&manager, &mut source, &mut workspace, RUN)
            .is_err());
        assert_eq!(fs::read_dir(f.root.join("state")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(f.root.join("workspaces")).unwrap().count(), 0);
    }
}

#[test]
fn absent_wrong_type_and_malformed_sources_never_report_active() {
    for kind in ["absent", "blob", "malformed", "incomplete"] {
        let mut f = Fixture::new();
        match kind {
            "absent" => {
                f.object.binding.expected_revision = ImmutableRevision::GitCommit("0".repeat(40))
            }
            "blob" => {
                f.object.binding.expected_revision = ImmutableRevision::GitCommit(git(
                    &f.git,
                    &f.root.join("original"),
                    &["rev-parse", "HEAD:hello.txt"],
                ))
            }
            "malformed" => {
                fs::write(f.root.join("store/source.bundle"), b"not a bundle").unwrap();
            }
            "incomplete" => {
                let repo = f.root.join("original");
                fs::write(repo.join("next.txt"), b"next").unwrap();
                git(&f.git, &repo, &["add", "next.txt"]);
                git(&f.git, &repo, &["commit", "-m", "next"]);
                git(
                    &f.git,
                    &repo,
                    &[
                        "bundle",
                        "create",
                        f.root.join("store/source.bundle").to_str().unwrap(),
                        "HEAD",
                        "^HEAD~1",
                    ],
                );
                f.object.binding.expected_revision =
                    ImmutableRevision::GitCommit(git(&f.git, &repo, &["rev-parse", "HEAD"]));
            }
            _ => unreachable!(),
        }
        f.object.binding.expected_raw_digest =
            sha256_digest(&fs::read(f.root.join("store/source.bundle")).unwrap());
        let manager = f.manager();
        let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
        assert!(
            f.prepare(&manager, &mut source, &mut workspace, RUN)
                .is_err(),
            "{kind}"
        );
        assert!(
            f.prepare(&manager, &mut source, &mut workspace, RUN)
                .is_err(),
            "uncertain attempt repeated: {kind}"
        );
        assert!(
            manager
                .recover(&mut workspace, &stable_workspace_key(RUN, 1))
                .is_err(),
            "{kind}"
        );
    }
}

#[test]
fn unsupported_formats_revisions_and_caller_binding_changes_are_rejected() {
    let f = Fixture::new();
    for (format, strategy) in [
        ("git_pack", "exact_commit"),
        ("content_addressed_snapshot", "exact_commit"),
        ("git_bundle", "synthetic_merge_commit"),
    ] {
        let mut config = f.config();
        config.objects[0].object_format = format.into();
        config.objects[0].revision_strategy = strategy.into();
        assert!(local_bundle_providers(config).is_err());
    }
    let (mut source, _) = local_bundle_providers(f.config()).unwrap();
    let mut lease = f.lease(RUN);
    lease.binding.expected_raw_digest = sha256_digest(b"caller alternative");
    assert!(source.acquire(&lease).is_err());
}

#[test]
fn binary_empty_executable_and_filter_attributed_files_are_exact() {
    let mut f = Fixture::new();
    fs::write(f.root.join("original/binary.bin"), b"\0\xffraw\0bytes").unwrap();
    fs::write(f.root.join("original/empty"), b"").unwrap();
    fs::write(f.root.join("original/run.sh"), b"#!/bin/sh\nexit 99\n").unwrap();
    fs::set_permissions(
        f.root.join("original/run.sh"),
        fs::Permissions::from_mode(0o700),
    )
    .unwrap();
    fs::write(f.root.join("original/.gitattributes"), b"* filter=trap\n").unwrap();
    f.refresh_bundle();
    let manager = f.manager();
    let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
    let handle = f
        .prepare(&manager, &mut source, &mut workspace, RUN)
        .unwrap();
    assert_eq!(
        fs::read(handle.root().join("binary.bin")).unwrap(),
        b"\0\xffraw\0bytes"
    );
    assert_eq!(fs::metadata(handle.root().join("empty")).unwrap().len(), 0);
    assert_ne!(
        fs::metadata(handle.root().join("run.sh")).unwrap().mode() & 0o100,
        0
    );
    assert_eq!(
        git(&f.git, handle.root(), &["rev-parse", "--show-toplevel"]),
        handle.root().to_str().unwrap()
    );
    assert!(git(&f.git, handle.root(), &["diff", "--exit-code"]).is_empty());
}

#[test]
fn invalid_configuration_has_no_effects_and_paths_cannot_supply_authority() {
    let f = Fixture::new();
    for mode in [
        "state-cache",
        "state-journal",
        "state-store",
        "state-workspace",
        "extra",
        "missing",
        "parent-link",
        "basename",
        "duration",
        "count",
    ] {
        let mut config = f.config();
        match mode {
            "state-cache" => config.state_root = config.cache_root.clone(),
            "state-journal" => config.state_root = config.journal_parent.clone(),
            "state-store" => config.state_root = config.source_store.clone(),
            "state-workspace" => config.state_root = config.workspace_root.clone(),
            "extra" => config
                .additional_writable_roots
                .push(config.state_root.clone()),
            "missing" => config.state_root = f.root.join("must-not-create"),
            "parent-link" => {
                std::os::unix::fs::symlink(f.root.join("state"), f.root.join("state-link"))
                    .unwrap();
                config.state_root = f.root.join("state-link");
            }
            "basename" => config.objects[0].file_name = "../store/source.bundle".into(),
            "duration" => config.limits.operation_timeout = std::time::Duration::ZERO,
            "count" => config.limits.max_files = usize::MAX,
            _ => unreachable!(),
        }
        assert!(local_bundle_providers(config).is_err(), "{mode}");
        assert!(!f.root.join("must-not-create").exists());
        assert_eq!(fs::read_dir(f.root.join("state")).unwrap().count(), 0);
        assert_eq!(fs::read_dir(f.root.join("workspaces")).unwrap().count(), 0);
    }
    let (mut source, _) = local_bundle_providers(f.config()).unwrap();
    fs::rename(
        f.root.join("store/source.bundle"),
        f.root.join("outside.bundle"),
    )
    .unwrap();
    std::os::unix::fs::symlink(
        f.root.join("outside.bundle"),
        f.root.join("store/source.bundle"),
    )
    .unwrap();
    assert!(source.acquire(&f.lease(RUN)).is_err());
}

#[test]
fn source_symlinks_and_gitlinks_are_rejected_before_source_writes() {
    for mode in ["symlink", "gitlink"] {
        let mut f = Fixture::new();
        if mode == "symlink" {
            std::os::unix::fs::symlink("../../outside", f.root.join("original/link")).unwrap();
            f.refresh_bundle();
        } else {
            let ImmutableRevision::GitCommit(commit) = &f.object.binding.expected_revision else {
                unreachable!()
            };
            git(
                &f.git,
                &f.root.join("original"),
                &[
                    "update-index",
                    "--add",
                    "--cacheinfo",
                    &format!("160000,{commit},submodule"),
                ],
            );
            git(
                &f.git,
                &f.root.join("original"),
                &["commit", "-m", "gitlink"],
            );
            git(
                &f.git,
                &f.root.join("original"),
                &[
                    "bundle",
                    "create",
                    f.root.join("store/source.bundle").to_str().unwrap(),
                    "HEAD",
                ],
            );
            f.object.binding.expected_revision = ImmutableRevision::GitCommit(git(
                &f.git,
                &f.root.join("original"),
                &["rev-parse", "HEAD"],
            ));
            f.object.binding.expected_raw_digest =
                sha256_digest(&fs::read(f.root.join("store/source.bundle")).unwrap());
        }
        let manager = f.manager();
        let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
        assert!(
            f.prepare(&manager, &mut source, &mut workspace, RUN)
                .is_err(),
            "{mode}"
        );
        let root = f.root.join("workspaces").join(RUN).join("generation-1");
        assert!(!root.join("hello.txt").exists(), "{mode}");
        assert!(manager
            .recover(&mut workspace, &stable_workspace_key(RUN, 1))
            .is_err());
    }
}

#[test]
fn expanded_bytes_objects_files_and_observed_disk_limits_reject() {
    for mode in ["blob", "expanded", "objects", "files", "disk", "output"] {
        let mut f = Fixture::new();
        fs::write(f.root.join("original/large"), vec![0u8; 4096]).unwrap();
        f.refresh_bundle();
        let manager = f.manager();
        let mut config = f.config();
        match mode {
            "blob" => config.limits.max_blob_bytes = 1024,
            "expanded" => config.limits.max_tree_bytes = 4096,
            "objects" => config.limits.max_objects = 1,
            "files" => config.limits.max_files = 1,
            "disk" => config.limits.max_git_disk_bytes = 1,
            "output" => config.limits.max_output_bytes = 1,
            _ => unreachable!(),
        }
        let (mut source, mut workspace) = local_bundle_providers(config).unwrap();
        assert!(
            f.prepare(&manager, &mut source, &mut workspace, RUN)
                .is_err(),
            "{mode}"
        );
        assert!(
            manager
                .recover(&mut workspace, &stable_workspace_key(RUN, 1))
                .is_err(),
            "{mode}"
        );
    }
}

struct LostReply<W>(W);
impl<W: WorkspaceProvider> WorkspaceProvider for LostReply<W> {
    fn scope(&self) -> &str {
        self.0.scope()
    }
    fn discover(
        &mut self,
        intent: &WorkspaceIntent,
    ) -> Result<WorkspaceDiscovery, fkst_local_qa_host::RunError> {
        self.0.discover(intent)
    }
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        blob: &Path,
        root: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, fkst_local_qa_host::RunError> {
        self.0.materialize(intent, blob, root, revision)?;
        Err(fkst_local_qa_host::RunError::Lifecycle(
            "simulated identity reply loss",
        ))
    }
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, fkst_local_qa_host::RunError> {
        self.0.status(resource)
    }
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, fkst_local_qa_host::RunError> {
        self.0.stop(resource)
    }
}

#[test]
fn completed_effect_reply_loss_is_discovered_after_restart() {
    let f = Fixture::new();
    let manager = f.manager();
    let (mut source, workspace) = local_bundle_providers(f.config()).unwrap();
    let mut workspace = LostReply(workspace);
    assert!(f
        .prepare(&manager, &mut source, &mut workspace, RUN)
        .is_err());
    let key = stable_workspace_key(RUN, 1);
    let root = f.root.join("workspaces").join(RUN).join("generation-1");
    let inode = fs::metadata(root.join("hello.txt")).unwrap().ino();
    drop(manager);
    drop(workspace);
    let manager = f.manager();
    let (_, mut workspace) = local_bundle_providers(f.config()).unwrap();
    let recovered = manager.recover(&mut workspace, &key).unwrap();
    assert_eq!(
        fs::metadata(recovered.root().join("hello.txt"))
            .unwrap()
            .ino(),
        inode
    );
    assert_eq!(
        manager.status(&mut workspace, &recovered).unwrap(),
        WorkspaceStatus::Active
    );
}

#[test]
fn discovery_rejects_git_symlinks_alternates_special_files_and_oversized_records() {
    for mode in [
        "config-link",
        "objects-link",
        "alternates",
        "fifo",
        "record",
    ] {
        let f = Fixture::new();
        let manager = f.manager();
        let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
        let handle = f
            .prepare(&manager, &mut source, &mut workspace, RUN)
            .unwrap();
        let gitroot = handle.root().join(".git");
        match mode {
            "config-link" => {
                fs::rename(gitroot.join("config"), f.root.join("config")).unwrap();
                std::os::unix::fs::symlink(f.root.join("config"), gitroot.join("config")).unwrap();
            }
            "objects-link" => {
                fs::rename(gitroot.join("objects"), f.root.join("objects")).unwrap();
                std::os::unix::fs::symlink(f.root.join("objects"), gitroot.join("objects"))
                    .unwrap();
            }
            "alternates" => fs::write(
                gitroot.join("objects/info/alternates"),
                b"/arbitrary/objects\n",
            )
            .unwrap(),
            "fifo" => {
                nix::unistd::mkfifo(&gitroot.join("pipe"), nix::sys::stat::Mode::S_IRUSR).unwrap();
            }
            "record" => {
                let record = fs::read_dir(f.root.join("state"))
                    .unwrap()
                    .map(|e| e.unwrap().path())
                    .find(|p| p.to_string_lossy().ends_with("complete.json"))
                    .unwrap();
                fs::OpenOptions::new()
                    .write(true)
                    .open(record)
                    .unwrap()
                    .set_len(128 * 1024 + 1)
                    .unwrap();
            }
            _ => unreachable!(),
        }
        assert!(manager.status(&mut workspace, &handle).is_err(), "{mode}");
        let record = Journal::open(&f.root.join("journal/host.sqlite"))
            .unwrap()
            .workspace(&stable_workspace_key(RUN, 1))
            .unwrap()
            .unwrap();
        assert!(
            !matches!(workspace.discover(&record.intent), Ok(WorkspaceDiscovery::Found(receipt)) if receipt.status == WorkspaceProviderStatus::Active),
            "{mode}"
        );
    }
}

#[test]
fn inherited_git_configuration_and_helpers_cannot_execute() {
    let f = Fixture::new();
    let traps = f.root.join("traps");
    fs::create_dir(&traps).unwrap();
    let sentinel = f.root.join("must-not-exist");
    for name in [
        "post-checkout",
        "reference-transaction",
        "post-index-change",
        "git-upload-pack",
        "git-index-pack",
        "git-cat-file",
    ] {
        fs::write(
            traps.join(name),
            format!("#!/bin/sh\ntouch '{}'\nexit 99\n", sentinel.display()),
        )
        .unwrap();
        fs::set_permissions(traps.join(name), fs::Permissions::from_mode(0o700)).unwrap();
    }
    let config = f.root.join("global.gitconfig");
    fs::write(&config, format!("[core]\n hooksPath = {}\n[filter \"trap\"]\n smudge = touch {}\n required = true\n[include]\n path = /nonexistent/trap-config\n", traps.display(), sentinel.display())).unwrap();
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "inherited_git_configuration_child",
            "--nocapture",
        ])
        .env("BUNDLE_TRAP_CHILD", "1")
        .env("GIT_CONFIG_GLOBAL", config)
        .env("GIT_EXEC_PATH", &traps)
        .env("GIT_TEMPLATE_DIR", &traps)
        .env("GIT_DIR", f.root.join("original/.git"))
        .env("GIT_ALTERNATE_OBJECT_DIRECTORIES", "/nonexistent/alternate")
        .env("GIT_CONFIG_COUNT", "1")
        .env("GIT_CONFIG_KEY_0", "core.hooksPath")
        .env("GIT_CONFIG_VALUE_0", &traps)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(!sentinel.exists());
}

#[test]
fn inherited_git_configuration_child() {
    if std::env::var_os("BUNDLE_TRAP_CHILD").is_none() {
        return;
    }
    binary_empty_executable_and_filter_attributed_files_are_exact();
}

struct CountedSource<S> {
    inner: S,
    calls: usize,
}
impl<S: SourceProvider> SourceProvider for CountedSource<S> {
    fn acquire(
        &mut self,
        lease: &SourceObjectLease,
    ) -> Result<AcquiredSource, fkst_local_qa_host::RunError> {
        self.calls += 1;
        self.inner.acquire(lease)
    }
}
struct CountedWorkspace<W> {
    inner: W,
    creates: usize,
}
impl<W: WorkspaceProvider> WorkspaceProvider for CountedWorkspace<W> {
    fn scope(&self) -> &str {
        self.inner.scope()
    }
    fn discover(
        &mut self,
        intent: &WorkspaceIntent,
    ) -> Result<WorkspaceDiscovery, fkst_local_qa_host::RunError> {
        self.inner.discover(intent)
    }
    fn materialize(
        &mut self,
        intent: &WorkspaceIntent,
        blob: &Path,
        root: &Path,
        revision: &ImmutableRevision,
    ) -> Result<WorkspaceMaterialization, fkst_local_qa_host::RunError> {
        self.creates += 1;
        self.inner.materialize(intent, blob, root, revision)
    }
    fn status(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStatusReceipt, fkst_local_qa_host::RunError> {
        self.inner.status(resource)
    }
    fn stop(
        &mut self,
        resource: &WorkspaceResource,
    ) -> Result<WorkspaceProviderStopReceipt, fkst_local_qa_host::RunError> {
        self.inner.stop(resource)
    }
}
#[test]
#[cfg_attr(
    target_os = "macos",
    ignore = "macOS filesystem rejects this non-UTF8 fixture path"
)]
fn review_regression_non_utf8_host_roots_preserve_argv_and_sibling() {
    use std::os::unix::ffi::{OsStrExt, OsStringExt};
    let f = Fixture::new();
    let raw = f
        .root
        .join(std::ffi::OsString::from_vec(b"host space-\xff".to_vec()));
    let sibling = f.root.join("host space-\u{fffd}");
    fs::create_dir(&raw).unwrap();
    fs::create_dir(&sibling).unwrap();
    let mut config = f.config();
    // Keep SQLite's UTF-8 journal location contract; exercise raw bytes in every
    // root used for Git arguments and source/workspace I/O instead.
    for path in [
        &mut config.source_store,
        &mut config.state_root,
        &mut config.workspace_root,
        &mut config.cache_root,
    ] {
        let target = raw.join(path.file_name().unwrap());
        fs::rename(&*path, &target).unwrap();
        *path = target;
    }
    let sibling_workspace = sibling.join("workspaces").join(RUN).join("generation-1");
    fs::create_dir_all(&sibling_workspace).unwrap();
    fs::write(sibling_workspace.join("sentinel"), b"unchanged").unwrap();
    let manager = SourceWorkspaceManager::new(
        &config.cache_root,
        &config.workspace_root,
        Journal::open(&config.journal_parent.join("host.sqlite")).unwrap(),
        WorkspaceProviderScope {
            identity: config.scope.clone(),
            writable_roots: vec![config.state_root.clone()],
        },
    )
    .unwrap();
    let (mut source, mut workspace) = local_bundle_providers(config).unwrap();
    let result = f.prepare(&manager, &mut source, &mut workspace, RUN);
    assert_eq!(
        fs::read_dir(&sibling_workspace).unwrap().count(),
        1,
        "lossy sibling must receive no Git files"
    );
    assert_eq!(
        fs::read(sibling_workspace.join("sentinel")).unwrap(),
        b"unchanged"
    );
    let handle = result.unwrap();
    assert_eq!(
        fs::read(handle.root().join("hello.txt")).unwrap(),
        b"immutable source\n"
    );
    let output = Command::new(&f.git)
        .env_clear()
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .current_dir(handle.root())
        .args(["rev-parse", "--show-toplevel"])
        .output()
        .unwrap();
    assert!(output.status.success());
    let mut expected = handle.root().as_os_str().as_bytes().to_vec();
    expected.push(b'\n');
    assert_eq!(output.stdout, expected);
}

fn state_lock(f: &Fixture) -> nix::fcntl::Flock<fs::File> {
    nix::fcntl::Flock::lock(
        fs::File::open(f.root.join("state")).unwrap(),
        nix::fcntl::FlockArg::LockExclusiveNonblock,
    )
    .unwrap()
}
fn record(f: &Fixture, run: &str) -> OwnedWorkspace {
    Journal::open(&f.root.join("journal/host.sqlite"))
        .unwrap()
        .workspace(&stable_workspace_key(run, 1))
        .unwrap()
        .unwrap()
}

#[test]
fn review_regression_state_contention_retries_without_effect_in_same_and_restarted_manager() {
    for restart in [false, true] {
        let f = Fixture::new();
        let mut manager = f.manager();
        let (source, workspace) = local_bundle_providers(f.config()).unwrap();
        let mut source = CountedSource {
            inner: source,
            calls: 0,
        };
        let mut workspace = CountedWorkspace {
            inner: workspace,
            creates: 0,
        };
        let lock = state_lock(&f);
        assert!(f
            .prepare(&manager, &mut source, &mut workspace, RUN)
            .is_err());
        assert_eq!((source.calls, workspace.creates), (1, 0));
        let before = record(&f, RUN);
        assert_eq!(before.state, WorkspaceState::DirectoryReady);
        assert!(before.resource.is_none());
        assert_eq!(fs::read_dir(f.root.join("state")).unwrap().count(), 0);
        drop(lock);
        if restart {
            drop(manager);
            manager = f.manager();
        }
        let handle = f
            .prepare(&manager, &mut source, &mut workspace, RUN)
            .expect("before-effect contention must be retryable");
        assert_eq!((source.calls, workspace.creates), (1, 1));
        assert!(before.blocker.is_none());
        assert_eq!(record(&f, RUN).state, WorkspaceState::Bound);
        assert_eq!(
            f.prepare(&manager, &mut source, &mut workspace, RUN)
                .unwrap(),
            handle
        );
        assert_eq!((source.calls, workspace.creates), (1, 1));
    }
}

#[test]
fn review_regression_canonical_fractional_acquisition_and_materialization() {
    let f = Fixture::new();
    let manager = f.manager();
    let (mut source, mut workspace) = local_bundle_providers(f.config()).unwrap();
    let mut lease = f.lease(RUN);
    for fraction in ["001", "000001", "000000001", "0000000001"] {
        lease.deadline_utc = format!("2099-01-01T00:00:00.{fraction}Z");
        assert!(
            source.acquire(&lease).is_ok(),
            "canonical fraction {fraction}"
        );
    }
    lease.deadline_utc = "2099-01-01T00:00:00.001Z".into();
    let mut request = f.request(RUN);
    request.deadline_utc = lease.deadline_utc.clone();
    let handle = manager
        .prepare(
            &mut source,
            &mut workspace,
            &f.object.binding.reference,
            &lease,
            &request,
            &FixedClock::new("2026-09-10T00:00:00.000000001Z").unwrap(),
        )
        .unwrap();
    assert_eq!(
        manager.status(&mut workspace, &handle).unwrap(),
        WorkspaceStatus::Active
    );
    for timestamp in [
        "2099-01-01T00:00:00.0Z",
        "2099-01-01T00:00:00.50Z",
        "2000-01-01T00:00:00.001Z",
    ] {
        lease.deadline_utc = timestamp.into();
        assert!(source.acquire(&lease).is_err(), "{timestamp}");
    }
}
