//! Real-engine integration suite for the session runner.
//!
//! Self-skipping: every test resolves the engine binary and quietly returns
//! (with an explanatory line on stderr) when none is available, so the suite
//! never silently passes without the real engine NOR fails on hosts that
//! cannot run it.
//!
//! Resolution order:
//! 1. `FKST_ENGINE_BIN` (path to a runnable `fkst-framework`).
//! 2. `/usr/local/bin/fkst-framework` when executable (the engine image).
//! 3. Linux with Docker: extract the binary from the engine image
//!    (`FKST_ENGINE_IMAGE`, default `fkst-hosted-api:engine-dev`) into a
//!    cached temp path via `docker create` + `docker cp`.
//! 4. Otherwise skip. The docker-extracted binary is a LINUX binary — on
//!    macOS it cannot run on the host, so without `FKST_ENGINE_BIN` the
//!    suite self-skips there.
//!
//! NOTE: no CI job currently exercises this suite against a real engine —
//! rust-ci's runner has no engine image, and docker-build.yml never runs
//! `cargo test`. The suite engages only when `FKST_ENGINE_BIN` is set, a
//! runnable `/usr/local/bin/fkst-framework` is present, or on Linux with
//! Docker and the engine image available (override via `FKST_ENGINE_IMAGE`,
//! default `fkst-hosted-api:engine-dev`).

use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use fkst_hosted_api::engine::process::signal_group;
use fkst_hosted_api::engine::{
    EngineConfig, LiveStatus, PreparedPackage, RunnerError, SessionRunner,
};
use fkst_hosted_api::packages::PackageFile;
use nix::sys::signal::Signal;
use nix::unistd::Pid;

/// Default binary location inside the engine image / engine-based pods.
const IMAGE_ENGINE_BIN: &str = "/usr/local/bin/fkst-framework";

/// Default engine image for the Docker extraction path (overridable via
/// `FKST_ENGINE_IMAGE`). Only the Linux extraction path consumes it.
#[cfg(target_os = "linux")]
const DEFAULT_ENGINE_IMAGE: &str = "fkst-hosted-api:engine-dev";

fn engine_bin() -> Option<PathBuf> {
    static ENGINE: OnceLock<Option<PathBuf>> = OnceLock::new();
    ENGINE.get_or_init(resolve_engine).clone()
}

fn resolve_engine() -> Option<PathBuf> {
    if let Ok(custom) = std::env::var("FKST_ENGINE_BIN") {
        let path = PathBuf::from(custom);
        if is_executable(&path) {
            return Some(path);
        }
        eprintln!(
            "FKST_ENGINE_BIN is set but not an executable file: {}",
            path.display()
        );
        return None;
    }
    let image_bin = Path::new(IMAGE_ENGINE_BIN);
    if is_executable(image_bin) {
        return Some(image_bin.to_path_buf());
    }
    extract_from_docker()
}

fn is_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    path.metadata()
        .map(|meta| meta.is_file() && meta.permissions().mode() & 0o111 != 0)
        .unwrap_or(false)
}

/// Linux-only: pull the engine binary out of the Docker image into a cached
/// temp path. The extracted binary is a Linux ELF, so this path is gated to
/// Linux hosts (macOS cannot exec it; see the module docs).
#[cfg(target_os = "linux")]
fn extract_from_docker() -> Option<PathBuf> {
    use std::process::Command;

    let image =
        std::env::var("FKST_ENGINE_IMAGE").unwrap_or_else(|_| DEFAULT_ENGINE_IMAGE.to_string());
    let docker_ok = Command::new("docker")
        .arg("version")
        .output()
        .map(|out| out.status.success())
        .unwrap_or(false);
    if !docker_ok {
        eprintln!("docker not available; cannot extract the engine binary");
        return None;
    }

    let cache_dir = std::env::temp_dir().join("fkst-engine-it");
    let target = cache_dir.join("fkst-framework");
    if is_executable(&target) {
        return Some(target); // cached from a previous run
    }
    std::fs::create_dir_all(&cache_dir).ok()?;

    let create = Command::new("docker")
        .args(["create", &image])
        .output()
        .ok()?;
    if !create.status.success() {
        eprintln!(
            "docker create {image} failed: {}",
            String::from_utf8_lossy(&create.stderr)
        );
        return None;
    }
    let cid = String::from_utf8_lossy(&create.stdout).trim().to_string();

    let cp = Command::new("docker")
        .args([
            "cp",
            &format!("{cid}:{IMAGE_ENGINE_BIN}"),
            &target.to_string_lossy(),
        ])
        .output();
    let _ = Command::new("docker").args(["rm", "-f", &cid]).output();

    match cp {
        Ok(out) if out.status.success() && is_executable(&target) => Some(target),
        Ok(out) => {
            eprintln!(
                "docker cp of the engine binary failed: {}",
                String::from_utf8_lossy(&out.stderr)
            );
            None
        }
        Err(err) => {
            eprintln!("docker cp of the engine binary failed: {err}");
            None
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn extract_from_docker() -> Option<PathBuf> {
    eprintln!(
        "non-Linux host: the docker-extracted engine binary cannot run here; \
         set FKST_ENGINE_BIN to a runnable fkst-framework to enable this suite"
    );
    None
}

macro_rules! require_engine {
    () => {
        match engine_bin() {
            Some(bin) => bin,
            None => {
                eprintln!(
                    "SKIP: no real fkst-framework available \
                     (FKST_ENGINE_BIN / {IMAGE_ENGINE_BIN} / Linux+Docker image)"
                );
                return;
            }
        }
    };
}

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
