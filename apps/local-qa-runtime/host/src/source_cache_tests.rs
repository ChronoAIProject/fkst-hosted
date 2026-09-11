use super::*;
use std::fs;
use std::io::{BufRead, Read, Write};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

type Hook = Box<dyn FnMut(&str) -> Result<(), RunError>>;
thread_local! {
    static HOOK: RefCell<Option<Hook>> = RefCell::new(None);
}
pub(super) fn checkpoint(stage: &str) -> Result<(), RunError> {
    HOOK.with_borrow_mut(|hook| match hook {
        Some(hook) => hook(stage),
        None => Ok(()),
    })
}
struct Injection;
impl Injection {
    fn new(hook: impl FnMut(&str) -> Result<(), RunError> + 'static) -> Self {
        HOOK.with_borrow_mut(|slot| *slot = Some(Box::new(hook)));
        Self
    }
}
impl Drop for Injection {
    fn drop(&mut self) {
        HOOK.with_borrow_mut(|slot| *slot = None);
    }
}
struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!(
            "fkst-cache-{}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos(),
            NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ));
        fs::create_dir(&path).unwrap();
        Self(fs::canonicalize(path).unwrap())
    }
    fn manager(&self) -> SourceWorkspaceManager {
        manager(&self.0)
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}
fn manager(root: &Path) -> SourceWorkspaceManager {
    fs::create_dir_all(root.join("journals")).unwrap();
    SourceWorkspaceManager::new(
        root.join("cache"),
        root.join("workspaces"),
        Journal::open(
            &root
                .join("journals")
                .join(format!("host-{}.sqlite", std::process::id())),
        )
        .unwrap(),
        WorkspaceProviderScope {
            identity: "cache-test/v1".into(),
            writable_roots: vec![],
        },
    )
    .unwrap()
}
fn binding(id: &str) -> TrustedLocalSourceBinding {
    TrustedLocalSourceBinding {
        reference: DigestBoundReferenceV2 {
            kind: "source".into(),
            id: id.into(),
            schema_version: "local.fixture/v1".into(),
            content_digest: sha256_digest(id.as_bytes()),
        },
        source_object_id: id.into(),
        expected_raw_digest: sha256_digest(b"immutable shared source"),
        expected_revision: ImmutableRevision::GitCommit("a".repeat(40)),
        expected_provider_scope: "local-source/v1".into(),
        expected_provider_identity: "source/one".into(),
    }
}
fn acquired(binding: &TrustedLocalSourceBinding) -> AcquiredSource {
    AcquiredSource {
        source_object_id: binding.source_object_id.clone(),
        immutable_revision: binding.expected_revision.clone(),
        provider_scope: binding.expected_provider_scope.clone(),
        provider_identity: binding.expected_provider_identity.clone(),
        bytes: b"immutable shared source".to_vec(),
    }
}
fn child(root: &Path, stage: &str, id: &str) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args([
            "--exact",
            "source_workspace::cache_tests::process_child",
            "--nocapture",
        ])
        .env("FKST_SOURCE_CACHE_TEST_ROOT", root)
        .env("FKST_SOURCE_CACHE_TEST_STAGE", stage)
        .env("FKST_SOURCE_CACHE_TEST_BINDING", id);
    command
}

#[test]
fn process_child() {
    let Some(root) = std::env::var_os("FKST_SOURCE_CACHE_TEST_ROOT") else {
        return;
    };
    let stage = std::env::var("FKST_SOURCE_CACHE_TEST_STAGE").unwrap();
    let id = std::env::var("FKST_SOURCE_CACHE_TEST_BINDING").unwrap();
    let manager = manager(Path::new(&root));
    let binding = binding(&id);
    if stage == "contend" {
        assert!(matches!(
            manager.cache_verified_source(&binding, acquired(&binding)),
            Err(RunError::Lifecycle(
                "source cache publication is busy; retry"
            ))
        ));
        return;
    }
    let nth = std::env::var("FKST_SOURCE_CACHE_TEST_NTH")
        .ok()
        .map(|s| s.parse::<usize>().unwrap())
        .unwrap_or(1);
    let mut seen = 0;
    let _injection = Injection::new(move |at| {
        if stage == "hold" && at == "after-intent" {
            println!("CACHE_LOCK_HELD");
            std::io::stdout().flush().unwrap();
            std::io::stdin().read_exact(&mut [0u8; 1]).unwrap();
        } else if at == stage {
            seen += 1;
            if seen == nth {
                std::process::exit(73);
            }
        }
        Ok(())
    });
    manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
}

#[test]
fn process_exit_at_publication_boundaries_reopens_and_recovers() {
    for stage in [
        "before-intent",
        "after-intent",
        "before-raw",
        "after-raw",
        "before-marker",
        "after-marker",
        "before-receipt",
        "after-receipt",
        "temporary-created",
        "temporary-written",
        "temporary-synced",
        "temporary-unlinked",
        "before-directory-sync",
        "after-directory-sync",
    ] {
        let fixture = Fixture::new();
        let output = child(&fixture.0, stage, "one").output().unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "stage={stage}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let manager = fixture.manager();
        let binding = binding("one");
        if stage != "after-receipt" {
            assert!(
                manager.load_cached_source(&binding).unwrap().is_none(),
                "stage={stage}"
            );
        }
        // This is a new validated acquisition, never authority inferred from a temp.
        let source = manager
            .cache_verified_source(&binding, acquired(&binding))
            .unwrap();
        source.validate().unwrap();
        assert_eq!(
            fs::read(source.cache_path).unwrap(),
            b"immutable shared source"
        );
    }
}

#[test]
fn process_exit_inside_each_member_publication_keeps_recovery_bounded() {
    for nth in 1..=4 {
        for stage in [
            "temporary-created",
            "temporary-written",
            "temporary-synced",
            "temporary-unlinked",
            "before-directory-sync",
            "after-directory-sync",
        ] {
            let fixture = Fixture::new();
            let output = child(&fixture.0, stage, "one")
                .env("FKST_SOURCE_CACHE_TEST_NTH", nth.to_string())
                .output()
                .unwrap();
            assert_eq!(
                output.status.code(),
                Some(73),
                "stage={stage}, nth={nth}: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            let manager = fixture.manager();
            let binding = binding("one");
            manager
                .cache_verified_source(&binding, acquired(&binding))
                .unwrap()
                .validate()
                .unwrap();
        }
    }
}

#[test]
fn directory_replacement_at_publication_sync_refuses_success_and_preserves_new_tree() {
    for stage in [
        "temporary-synced",
        "before-directory-sync",
        "after-directory-sync",
    ] {
        let fixture = Fixture::new();
        let manager = fixture.manager();
        let root = fixture.0.clone();
        let mut swapped = false;
        let _injection = Injection::new(move |at| {
            if at == stage && !swapped {
                fs::rename(root.join("cache"), root.join("displaced"))?;
                fs::create_dir(root.join("cache"))?;
                fs::write(root.join("cache/sentinel"), b"replacement tree")?;
                swapped = true;
            }
            Ok(())
        });
        let binding = binding("one");
        assert!(manager
            .cache_verified_source(&binding, acquired(&binding))
            .is_err());
        assert_eq!(fs::read_dir(fixture.0.join("cache")).unwrap().count(), 1);
        assert_eq!(
            fs::read(fixture.0.join("cache/sentinel")).unwrap(),
            b"replacement tree"
        );
    }
}

#[test]
fn link_unlink_crash_window_preserves_explicit_multiple_link_blocker() {
    for nth in 1..=4 {
        let fixture = Fixture::new();
        let output = child(&fixture.0, "temporary-linked", "one")
            .env("FKST_SOURCE_CACHE_TEST_NTH", nth.to_string())
            .output()
            .unwrap();
        assert_eq!(
            output.status.code(),
            Some(73),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let manager = fixture.manager();
        let binding = binding("one");
        let before = fs::read_dir(fixture.0.join("cache")).unwrap().count();
        assert!(manager.load_cached_source(&binding).is_err());
        assert!(manager
            .cache_verified_source(&binding, acquired(&binding))
            .is_err());
        assert_eq!(
            fs::read_dir(fixture.0.join("cache")).unwrap().count(),
            before
        );
    }
}

#[test]
fn two_process_publishers_same_and_distinct_bindings_contend_then_retry() {
    for second_id in ["one", "two"] {
        let fixture = Fixture::new();
        let mut leader = child(&fixture.0, "hold", "one")
            .stdout(Stdio::piped())
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        let mut output = std::io::BufReader::new(leader.stdout.take().unwrap());
        loop {
            let mut line = String::new();
            assert_ne!(
                output.read_line(&mut line).unwrap(),
                0,
                "leader exited before acquiring lock"
            );
            if line.contains("CACHE_LOCK_HELD") {
                break;
            }
        }
        let contender = child(&fixture.0, "contend", second_id).output().unwrap();
        leader.stdin.take().unwrap().write_all(b"x").unwrap();
        assert!(leader.wait().unwrap().success());
        assert!(
            contender.status.success(),
            "{}",
            String::from_utf8_lossy(&contender.stderr)
        );
        let retry = child(&fixture.0, "complete", second_id).output().unwrap();
        assert!(
            retry.status.success(),
            "{}",
            String::from_utf8_lossy(&retry.stderr)
        );
        let manager = fixture.manager();
        assert!(manager
            .load_cached_source(&binding("one"))
            .unwrap()
            .is_some());
        assert!(manager
            .load_cached_source(&binding(second_id))
            .unwrap()
            .is_some());
        let receipt_count = fs::read_dir(fixture.0.join("cache"))
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".binding.json")
            })
            .count();
        assert_eq!(receipt_count, if second_id == "one" { 1 } else { 2 });
    }
}

#[test]
fn fresh_open_descriptions_conflict_and_release_without_creating_lock_files() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let first = manager.cache_root.publication_lock().unwrap();
    assert!(manager.cache_root.publication_lock().is_err());
    let independent = Directory::prepare(manager.cache_root.path()).unwrap();
    assert!(independent.publication_lock().is_err());
    drop(first);
    assert!(independent.publication_lock().is_ok());
    assert_eq!(fs::read_dir(manager.cache_root.path()).unwrap().count(), 0);
}

#[test]
fn failed_intent_sync_prevents_raw_and_replay_sync_failure_refuses_verified_source() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let binding = binding("one");
    let injection = Injection::new(|at| {
        if at == "before-directory-sync" {
            Err(RunError::Lifecycle("injected directory sync failure"))
        } else {
            Ok(())
        }
    });
    assert!(manager
        .cache_verified_source(&binding, acquired(&binding))
        .is_err());
    drop(injection);
    assert!(!manager
        .cache_path(&binding.expected_raw_digest)
        .unwrap()
        .exists());
    let injection = Injection::new(|at| {
        if at == "before-intent-sync" {
            Err(RunError::Lifecycle("injected replay intent sync failure"))
        } else {
            Ok(())
        }
    });
    assert!(manager.load_cached_source(&binding).is_err());
    assert!(manager
        .cache_verified_source(&binding, acquired(&binding))
        .is_err());
    drop(injection);
    assert!(!manager
        .cache_path(&binding.expected_raw_digest)
        .unwrap()
        .exists());
    let source = manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
    let _injection = Injection::new(|at| {
        if at == "before-verified-sync" {
            Err(RunError::Lifecycle("injected replay sync failure"))
        } else {
            Ok(())
        }
    });
    assert!(manager.load_cached_source(&binding).is_err());
    assert!(source.validate().is_err());
}

#[test]
fn replay_accepts_legacy_v2_noncanonical_json_without_rewriting() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let binding = binding("one");
    let source = manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
    fs::remove_file(manager.publication_intent_path(&binding).unwrap()).unwrap();
    let paths = [
        cache_marker_path(&source.cache_path),
        manager.binding_receipt_path(&binding).unwrap(),
    ];
    for path in &paths {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        fs::write(
            path,
            format!(" \n{}\n ", serde_json::to_string_pretty(&value).unwrap()),
        )
        .unwrap();
    }
    let before: Vec<_> = paths.iter().map(|p| fs::read(p).unwrap()).collect();
    manager
        .load_cached_source(&binding)
        .unwrap()
        .unwrap()
        .validate()
        .unwrap();
    assert_eq!(
        paths
            .iter()
            .map(|p| fs::read(p).unwrap())
            .collect::<Vec<_>>(),
        before
    );
}

#[test]
fn digest_has_fixed_buffer_and_checks_size_and_identity_after_read() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let bytes = vec![42; 3 * 64 * 1024 + 7];
    manager
        .cache_root
        .write_new("digest-test".as_ref(), &bytes, false)
        .unwrap();
    let file = manager
        .cache_root
        .open_file("digest-test".as_ref())
        .unwrap()
        .unwrap();
    assert_eq!(file.digest().unwrap(), sha256_digest(&bytes));
    let path = manager.cache_root.path().join("digest-test");
    let _injection = Injection::new(move |at| {
        if at == "digest-chunk" {
            fs::OpenOptions::new()
                .append(true)
                .open(&path)?
                .write_all(b"growth")?;
        }
        Ok(())
    });
    assert!(file.digest().is_err());
}

#[test]
fn metadata_lexical_whitespace_and_escapes_preserve_valid_v2_and_reject_malformed_records() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let mut binding = binding("one");
    binding.expected_provider_identity = "quoted\"\\path/\t \r\n😀".into();
    manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
    let receipt = manager.binding_receipt_path(&binding).unwrap();
    let marker = cache_marker_path(&manager.cache_path(&binding.expected_raw_digest).unwrap());
    let padding = " \r\n\t".repeat(20_000);
    for path in [&receipt, &marker] {
        let value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        let pretty = serde_json::to_string_pretty(&value)
            .unwrap()
            .replace("source", "\\u0073ource")
            .replace('/', "\\/")
            .replace('😀', "\\ud83d\\ude00");
        fs::write(
            path,
            format!("{padding}{}{padding}", pretty.replace('\n', &padding)),
        )
        .unwrap();
    }
    manager
        .load_cached_source(&binding)
        .unwrap()
        .unwrap()
        .validate()
        .unwrap();
    let valid = fs::read(&receipt).unwrap();
    for malformed in [
        b"{\"schema_version\":1 2}".to_vec(),
        b"{\"schema_version\":tr ue}".to_vec(),
        b"{\"schema_version\":\"unterminated".to_vec(),
        b"{\"schema_version\":\"bad\\q\"}".to_vec(),
        vec![b'{', 0xff, b'}'],
        serde_json::to_vec(&serde_json::json!({"schema_version": "fkst.local-qa-source-binding/v1", "unknown": true})).unwrap(),
    ] {
        fs::write(&receipt, malformed).unwrap();
        assert!(manager.load_cached_source(&binding).is_err());
    }
    // Duplicate known fields survive normalization so the original typed serde
    // deserializer, rather than a Value round trip, rejects them.
    let canonical = encode_cache(&SourceBindingReceipt::new(&binding)).unwrap();
    let duplicate = [
        b"{\"schema_version\":\"fkst.local-qa-source-binding/v1\",".as_slice(),
        &canonical[1..],
    ]
    .concat();
    fs::write(&receipt, duplicate).unwrap();
    assert!(manager.load_cached_source(&binding).is_err());
    fs::write(&receipt, valid).unwrap();
    manager
        .load_cached_source(&binding)
        .unwrap()
        .unwrap()
        .validate()
        .unwrap();
    let file = manager
        .cache_root
        .open_file(file_name(&receipt).unwrap())
        .unwrap()
        .unwrap();
    fs::write(&receipt, b"{\"n\":1 \n 2}").unwrap();
    let compact = file.bounded_json(100).unwrap();
    assert_eq!(compact, b"{\"n\":1 2}");
    assert!(serde_json::from_slice::<serde_json::Value>(&compact).is_err());
}

#[test]
fn no_clobber_retains_committed_bytes_and_rejects_linked_receipts_and_intents() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let binding = binding("one");
    let source = manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
    manager
        .cache_root
        .publish_new(file_name(&source.cache_path).unwrap(), b"different", false)
        .unwrap();
    assert_eq!(
        fs::read(&source.cache_path).unwrap(),
        b"immutable shared source"
    );
    for path in [
        manager.binding_receipt_path(&binding).unwrap(),
        manager.publication_intent_path(&binding).unwrap(),
    ] {
        let original = fs::read(&path).unwrap();
        let outside = fixture.0.join("outside");
        fs::write(&outside, &original).unwrap();
        fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(&outside, &path).unwrap();
        assert!(manager.load_cached_source(&binding).is_err());
        fs::remove_file(&path).unwrap();
        fs::hard_link(&outside, &path).unwrap();
        assert!(manager.load_cached_source(&binding).is_err());
        fs::remove_file(&path).unwrap();
        fs::write(&path, original).unwrap();
        manager
            .load_cached_source(&binding)
            .unwrap()
            .unwrap()
            .validate()
            .unwrap();
    }
}

#[test]
fn oversized_metadata_rejected_from_expected_shape_without_reading_payload() {
    let fixture = Fixture::new();
    let manager = fixture.manager();
    let binding = binding("one");
    manager
        .cache_verified_source(&binding, acquired(&binding))
        .unwrap();
    let path = manager.binding_receipt_path(&binding).unwrap();
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .unwrap()
        .set_len(1 << 30)
        .unwrap();
    assert!(matches!(
        manager.load_cached_source(&binding),
        Err(RunError::Lifecycle(
            "source cache metadata exceeds expected binding encoding bound"
        ))
    ));
}
