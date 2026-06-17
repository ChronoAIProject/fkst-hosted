//! Real-engine integration suite for the session runner.
//!
//! Self-skipping: every test resolves the engine binary and quietly returns
//! (with an explanatory line on stderr) when none is available, so the suite
//! never silently passes without the real engine NOR fails on hosts that
//! cannot run it.
//!
//! Resolution order: see `tests/support/mod.rs` (shared with the e2e suite) —
//! `FKST_ENGINE_BIN`, then `/usr/local/bin/fkst-framework`, then (Linux only)
//! Docker extraction from `FKST_ENGINE_IMAGE`, else skip.
//!
//! NOTE: no CI job currently exercises this suite against a real engine —
//! rust-ci's runner has no engine image, and docker-build.yml never runs
//! `cargo test`. The suite engages only when `FKST_ENGINE_BIN` is set, a
//! runnable `/usr/local/bin/fkst-framework` is present, or on Linux with
//! Docker and the engine image available (override via `FKST_ENGINE_IMAGE`,
//! default `fkst-hosted-api:engine-dev`).

mod support;

use std::path::Path;
use std::time::Duration;

use fkst_control_plane::engine::materialize::PackageFile;
use fkst_control_plane::engine::process::signal_group;
use fkst_control_plane::engine::{
    EngineConfig, LiveStatus, PreparedPackage, RunnerError, SessionRunner, StartSpec,
};
use nix::sys::signal::Signal;
use nix::unistd::Pid;
use support::require_engine;

fn config(bin: &Path, temp_root: &Path) -> EngineConfig {
    EngineConfig {
        framework_bin: bin.to_path_buf(),
        temp_root: temp_root.to_path_buf(),
        candidate_prefix: "candidate/".to_string(),
        candidate_from_sep: "::".to_string(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 60,
        ready_timeout_secs: 30,
        error_capture_bytes: 8192,
        log_tail_lines: 200,
        github_token_refresh_secs: 2400,
    }
}

/// The issue #17 spike's minimal runnable package: one department consuming
/// a 1 s cron tick (spike Q4).
fn minimal_package() -> PreparedPackage {
    PreparedPackage {
        package_name: "it-demo".to_string(),
        files: vec![
            PackageFile {
                path: "departments/hello/main.lua".to_string(),
                content: r#"local M = {}
M.spec = {
  consumes = { "tick" },
  stall_window = "30s",
}
function pipeline(event)
  log.info("hello received event on queue: " .. tostring(event.queue))
end
return M
"#
                .to_string(),
            },
            PackageFile {
                path: "raisers/tick.lua".to_string(),
                content: "return {\n  type = \"cron\",\n  interval = \"1s\",\n  produces = \"tick\",\n}\n"
                    .to_string(),
            },
        ],
        composed_deps: Vec::new(),
    }
}

fn fkst_entries(temp_root: &Path) -> usize {
    std::fs::read_dir(temp_root)
        .expect("read temp root")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("fkst-"))
        .count()
}

/// Lay the spike's minimal package on disk under a fake repo's
/// `<repo>/.fkst/packages/<name>/` (issue #115), mirroring what the driver's
/// clone step produces. Returns the temp repo root; the caller passes
/// `--project-root <repo>` + `--package-root <repo>/.fkst/packages/<name>` via a
/// repo-scoped [`StartSpec`].
fn write_fixture_repo(name: &str) -> tempfile::TempDir {
    let repo = tempfile::tempdir().expect("repo dir");
    let pkg = repo.path().join(".fkst").join("packages").join(name);
    std::fs::create_dir_all(pkg.join("departments").join("hello")).expect("mkdir dept");
    std::fs::create_dir_all(pkg.join("raisers")).expect("mkdir raisers");
    for file in minimal_package().files {
        let target = pkg.join(&file.path);
        std::fs::create_dir_all(target.parent().unwrap()).expect("mkdir parent");
        std::fs::write(&target, file.content).expect("write package file");
    }
    repo
}

async fn wait_until(mut predicate: impl FnMut() -> bool, max_ms: u64) -> bool {
    let mut waited = 0;
    while waited < max_ms {
        if predicate() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
        waited += 100;
    }
    predicate()
}

#[tokio::test]
async fn happy_path_ready_running_logs_stop_and_clean() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    let mut session = runner
        .start(&minimal_package())
        .await
        .expect("real engine must start the spike's minimal package");

    // Own process group: PGID == PID.
    let pgid = nix::unistd::getpgid(Some(Pid::from_raw(session.pid))).expect("getpgid");
    assert_eq!(pgid.as_raw(), session.pid);

    assert_eq!(runner.status(&mut session), LiveStatus::Running);

    // Ready markers were observed by start(); the buffer carries them.
    let stderr = session.engine_stderr();
    assert!(stderr.contains("event runtime running"), "{stderr}");
    assert!(stderr.contains("consumer started"), "{stderr}");

    // Child logs appear only after the first dispatched event (~1 s cron).
    let got_logs = wait_until(|| session.tail_logs().is_some(), 15_000).await;
    if got_logs {
        let tail = session.tail_logs().expect("tail present");
        assert!(!tail.is_empty(), "child log tail must carry content");
    } else {
        // Documented alternative: an engine that has not dispatched yet
        // legitimately has no child logs. Do not fail the suite on timing.
        eprintln!("NOTE: no child logs within 15s; None is the documented idle answer");
    }

    let pid = session.pid;
    runner.stop(&mut session).await.expect("stop");
    assert_eq!(runner.status(&mut session), LiveStatus::Stopped);

    // The WHOLE group is gone (supervisor + framework grandchildren).
    assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
    assert!(!session.package_dir.exists());
    assert!(!session.runtime_dir.exists());
    assert_eq!(fkst_entries(temp_root.path()), 0, "no leaked fkst-* dirs");
}

/// Issue #115: the runner loads packages straight from a repo's
/// `<repo>/.fkst/packages/<name>/` (the driver's clone output) — no Mongo store,
/// no materialize copy — by pointing `--project-root` at the repo and
/// `--package-root` at the package dir. Reaching `Running` against the real
/// engine proves the repo-scoped load path end-to-end.
#[tokio::test]
async fn repo_scoped_package_starts_against_the_real_engine() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    let repo = write_fixture_repo("repo-demo");
    let package_root = repo.path().join(".fkst").join("packages").join("repo-demo");

    let spec = StartSpec {
        // Repo-scoped: `packages` stays empty; the dirs already exist on disk.
        packages: Vec::new(),
        goal: None,
        env_profile: std::collections::BTreeMap::new(),
        codex_home: None,
        project_root: Some(repo.path().to_path_buf()),
        package_roots: vec![package_root.clone()],
        session_id: String::new(),
        worker_id: String::new(),
    };
    let mut session = runner
        .start_with_spec(&spec)
        .await
        .expect("real engine must start a repo-scoped package");

    assert_eq!(runner.status(&mut session), LiveStatus::Running);
    let stderr = session.engine_stderr();
    assert!(stderr.contains("event runtime running"), "{stderr}");

    runner.stop(&mut session).await.expect("stop");
    assert_eq!(runner.status(&mut session), LiveStatus::Stopped);
    // The runner must NOT delete the repo dir (the driver owns the clone): only
    // its own runtime temp dirs are cleaned.
    assert!(
        package_root.is_dir(),
        "runner must not remove the driver-owned repo package dir"
    );
    assert_eq!(fkst_entries(temp_root.path()), 0, "no leaked fkst-* dirs");
}

#[tokio::test]
async fn conformance_rejects_a_department_missing_its_spec() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    // Spike negative case 4: main.lua without `M.spec` fails conformance
    // with exit 1 (pre-flight, 400-class).
    let pkg = PreparedPackage {
        package_name: "it-broken".to_string(),
        files: vec![PackageFile {
            path: "departments/broken/main.lua".to_string(),
            content: "local M = {}\nfunction pipeline(event)\n  log.info(\"x\")\nend\nreturn M\n"
                .to_string(),
        }],
        composed_deps: Vec::new(),
    };

    let err = runner.start(&pkg).await.expect_err("must fail pre-flight");
    match err {
        RunnerError::ConformanceFailed { code, stderr } => {
            assert_eq!(code, 1, "engine check failure must be exit 1");
            assert!(!stderr.is_empty(), "captured conformance output expected");
        }
        other => panic!("expected ConformanceFailed, got {other:?}"),
    }
    assert_eq!(fkst_entries(temp_root.path()), 0, "dirs cleaned on fail");
}

#[tokio::test]
async fn empty_package_is_rejected_by_runner_validation() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    // The runner blocks what supervise would silently idle on (spike 3d).
    let pkg = PreparedPackage {
        package_name: "it-empty".to_string(),
        files: Vec::new(),
        composed_deps: Vec::new(),
    };
    let err = runner.start(&pkg).await.expect_err("empty must be invalid");
    assert!(matches!(err, RunnerError::InvalidPackage(_)), "got {err:?}");
    assert_eq!(fkst_entries(temp_root.path()), 0);
}

#[tokio::test]
async fn stop_is_idempotent_against_the_real_engine() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    let mut session = runner.start(&minimal_package()).await.expect("start");
    runner.stop(&mut session).await.expect("first stop");
    runner
        .stop(&mut session)
        .await
        .expect("second stop (no-op)");
    assert_eq!(runner.status(&mut session), LiveStatus::Stopped);
    assert_eq!(fkst_entries(temp_root.path()), 0);
}

#[tokio::test]
async fn out_of_band_kill_turns_failed_and_stop_stays_ok() {
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let runner = SessionRunner::new(config(&bin, temp_root.path()));

    let mut session = runner.start(&minimal_package()).await.expect("start");
    signal_group(session.pid, Signal::SIGKILL).expect("out-of-band kill");

    assert!(
        wait_until(|| session.status() != LiveStatus::Running, 5_000).await,
        "killed engine must turn terminal"
    );
    assert!(
        matches!(session.status(), LiveStatus::Failed { .. }),
        "got {:?}",
        session.status()
    );

    // ESRCH swallowed; dirs still cleaned.
    runner.stop(&mut session).await.expect("stop must stay Ok");
    assert_eq!(fkst_entries(temp_root.path()), 0);
}
