//! The session runner: validate -> materialize -> conformance -> spawn
//! `supervise` -> bounded ready-wait, plus live status, log tail, and the
//! idempotent group stop.
//!
//! Accepted v1 posture (issue #17 spike):
//! - **A2 (non-git package dir):** packages are materialized as PLAIN
//!   directories — neither `conformance` nor `supervise` needs a git repo
//!   (spike Q2). A package that calls the Lua `sdk_git` API against this
//!   non-git dir would fail at RUNTIME; that is an accepted 500-class error
//!   for v1 (no defensive `git init`).
//! - **Durable root derivation:** `FKST_DURABLE_ROOT = <runtime_dir>/durable`,
//!   created fresh per start attempt — a stale `delivery.redb` would replay
//!   lease state across attempts (spike Q6). The engine creates
//!   `logs/framework-child/` itself (`mkdir -p` semantics, spike Q5), so only
//!   `durable/` is pre-created here.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::process::Child;

use crate::engine::config::EngineConfig;
use crate::engine::error::RunnerError;
use crate::engine::logs::tail_child_logs;
use crate::engine::materialize::{materialize_package, write_fkst_env, PreparedPackage};
use crate::engine::process::{
    is_panicked, is_pid_alive, is_ready, kill_group_quiet, reap_with_grace, run_conformance,
    spawn_supervise, ChildGroupGuard, SpawnedChild, StderrBuffer,
};

/// Poll interval of the post-spawn ready-wait loop.
const READY_POLL_INTERVAL: Duration = Duration::from_millis(50);

/// Live process state of a running session, derived from `try_wait`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "state", rename_all = "lowercase")]
pub enum LiveStatus {
    /// The supervise process is alive.
    Running,
    /// Terminal: clean exit (code 0), or reaped by [`RunningSession::stop`].
    Stopped,
    /// Terminal: unexpected non-zero exit or killed by a signal.
    Failed {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

/// Live handle for one engine process, held by the caller for the session's
/// lifetime. Owns the spawned [`Child`] and the two temp-dir guards; dropping
/// the handle removes the dirs but NEVER kills a live engine
/// (`kill_on_drop(false)` — lifecycle is explicit via [`Self::stop`]).
#[derive(Debug)]
pub struct RunningSession {
    /// PID of the supervise process; == PGID (own process group).
    pub pid: i32,
    /// Absolute `FKST_RUNTIME_ROOT` for this run.
    pub runtime_dir: PathBuf,
    /// Absolute materialized package root for this run.
    pub package_dir: PathBuf,
    child: Child,
    package_guard: Option<TempDir>,
    runtime_guard: Option<TempDir>,
    stderr: StderrBuffer,
    /// First terminal observation, cached so repeated `status()` calls are
    /// stable and never re-`try_wait` an already-reaped child.
    terminal_status: Option<LiveStatus>,
    log_tail_lines: usize,
}

impl RunningSession {
    /// Liveness via cached `try_wait` (never blocks, never panics).
    pub fn status(&mut self) -> LiveStatus {
        if let Some(cached) = self.terminal_status {
            return cached;
        }
        match self.child.try_wait() {
            Ok(None) => LiveStatus::Running,
            Ok(Some(exit)) => {
                let status = classify_exit(exit);
                tracing::info!(pid = self.pid, ?exit, "session.status: terminal");
                self.terminal_status = Some(status);
                status
            }
            Err(err) => {
                tracing::error!(pid = self.pid, error = %err, "session.status: try_wait failed");
                let status = LiveStatus::Failed {
                    code: None,
                    signal: None,
                };
                self.terminal_status = Some(status);
                status
            }
        }
    }

    /// Best-effort tail of the newest engine child log under
    /// `<runtime_dir>/logs/framework-child/`. `None` is a normal answer for
    /// an idle session (child logs appear only after the first dispatched
    /// event — spike Q9).
    pub fn tail_logs(&self) -> Option<String> {
        tail_child_logs(&self.runtime_dir, self.log_tail_lines)
    }

    /// Snapshot of the engine's recent stderr (the bounded ring buffer the
    /// drain task feeds) — the place to look when a ready session later
    /// turns `Failed`.
    pub fn engine_stderr(&self) -> String {
        self.stderr.snapshot()
    }

    /// Stop the session: SIGTERM the process GROUP, wait up to `grace`,
    /// escalate to SIGKILL, always reap, then clean both temp dirs.
    ///
    /// Idempotent: an already-dead/absent group (`ESRCH`, out-of-band kill,
    /// double stop) is a no-op success; the temp-dir guards are taken
    /// exactly once. A pre-existing terminal `Failed` observation is kept
    /// (stop does not rewrite history); otherwise the cached status becomes
    /// `Stopped`.
    pub async fn stop(&mut self, grace: Duration) -> Result<(), RunnerError> {
        tracing::info!(pid = self.pid, "session.stopping");
        let result = reap_with_grace(&mut self.child, self.pid, grace).await;
        match result {
            Ok(_escalated) => {
                if self.terminal_status.is_none() {
                    self.terminal_status = Some(LiveStatus::Stopped);
                }
                self.cleanup();
                Ok(())
            }
            Err(err) => {
                // Unreapable group: report it, but never leak the dirs.
                self.terminal_status = Some(LiveStatus::Failed {
                    code: None,
                    signal: None,
                });
                self.cleanup();
                Err(err)
            }
        }
    }

    /// Drop the temp-dir guards (each taken at most once), removing the
    /// package and runtime dirs.
    fn cleanup(&mut self) {
        let mut dirs_removed = 0;
        if self.package_guard.take().is_some() {
            dirs_removed += 1;
        }
        if self.runtime_guard.take().is_some() {
            dirs_removed += 1;
        }
        if dirs_removed > 0 {
            tracing::info!(pid = self.pid, dirs_removed, "session.cleanup");
        } else {
            tracing::debug!(pid = self.pid, "session.cleanup: already cleaned (no-op)");
        }
    }
}

/// Map a reaped exit status onto the live-status enum.
fn classify_exit(exit: std::process::ExitStatus) -> LiveStatus {
    use std::os::unix::process::ExitStatusExt;
    match exit.code() {
        Some(0) => LiveStatus::Stopped,
        Some(code) => LiveStatus::Failed {
            code: Some(code),
            signal: None,
        },
        None => LiveStatus::Failed {
            code: None,
            signal: exit.signal(),
        },
    }
}

/// Cheap, `Clone`-able runner: a config holder whose `start` produces one
/// [`RunningSession`] per engine process.
#[derive(Debug, Clone)]
pub struct SessionRunner {
    config: EngineConfig,
}

impl SessionRunner {
    pub fn new(config: EngineConfig) -> Self {
        Self { config }
    }

    /// Liveness primitive for the pool-manager pre-takeover check
    /// (re-export of [`crate::engine::process::is_pid_alive`]; see the
    /// PID-reuse caveat there).
    pub fn is_pid_alive(pid: i32) -> bool {
        is_pid_alive(pid)
    }

    /// Best-effort engine child-log tail for a known runtime dir (spec
    /// surface for callers that persist `runtime_dir` without holding the
    /// session handle).
    pub fn tail_logs(&self, runtime_dir: &Path) -> Option<String> {
        tail_child_logs(runtime_dir, self.config.log_tail_lines)
    }

    /// Delegate: [`RunningSession::status`].
    pub fn status(&self, session: &mut RunningSession) -> LiveStatus {
        session.status()
    }

    /// Delegate: [`RunningSession::stop`] with the configured grace.
    pub async fn stop(&self, session: &mut RunningSession) -> Result<(), RunnerError> {
        session
            .stop(Duration::from_secs(self.config.stop_grace_secs))
            .await
    }

    /// Start a session: validate -> materialize (`fkst-pkg-*`) -> `fkst.env`
    /// -> runtime root (`fkst-rt-*`, plus `durable/`) -> `conformance`
    /// pre-flight -> spawn `supervise` (own process group) -> bounded
    /// ready-wait.
    ///
    /// Readiness requires BOTH stderr markers (`event runtime running
    /// handles=` and at least one `consumer started dept=`); child-exit,
    /// a `panicked at` line, or the ready timeout fail startup with the
    /// stderr tail, after group-killing and reaping the child. On EVERY
    /// failure path both temp dirs are cleaned before returning.
    pub async fn start(&self, pkg: &PreparedPackage) -> Result<RunningSession, RunnerError> {
        // 1. Pure validation — no temp dir is created on the reject path.
        pkg.validate()?;

        // 2. Materialize the package tree (fails => RAII-cleaned).
        let package_guard = materialize_package(pkg, &self.config.temp_root)?;
        write_fkst_env(
            package_guard.path(),
            &self.config.candidate_prefix,
            &self.config.candidate_from_sep,
        )?;

        // 3. Fresh runtime root with a fresh durable root per attempt.
        let runtime_guard = tempfile::Builder::new()
            .prefix("fkst-rt-")
            .tempdir_in(&self.config.temp_root)
            .map_err(RunnerError::Io)?;
        std::fs::create_dir(runtime_guard.path().join("durable")).map_err(RunnerError::Io)?;

        let package_dir = package_guard
            .path()
            .canonicalize()
            .map_err(RunnerError::Io)?;
        let runtime_dir = runtime_guard
            .path()
            .canonicalize()
            .map_err(RunnerError::Io)?;

        // 4. Conformance pre-flight (mandatory: supervise is NOT a validator
        //    — it idles on an empty package, spike case 3d).
        run_conformance(
            &self.config.framework_bin,
            &package_dir,
            &runtime_dir,
            Duration::from_secs(self.config.conformance_timeout_secs),
            self.config.error_capture_bytes,
        )
        .await?;

        // 5. Spawn supervise in its own process group.
        let spawned = spawn_supervise(&self.config.framework_bin, &package_dir, &runtime_dir)?;

        // 6. Bounded ready-wait. Every failure path group-kills, reaps, and
        //    (by dropping the guards still held here) cleans both dirs.
        //    Cancellation safety: the armed ChildGroupGuard ensures that
        //    dropping THIS future mid-ready-wait (client disconnect, outer
        //    select!/timeout) also group-kills and reaps the spawned engine;
        //    it is defused when ownership moves to the RunningSession or to
        //    fail_startup (which kills + reaps itself).
        let SpawnedChild { child, pid, stderr } = spawned;
        let mut guard = ChildGroupGuard::new(child, pid);
        let ready_timeout = Duration::from_secs(self.config.ready_timeout_secs);
        let started = Instant::now();
        loop {
            // Child-exit-first: an engine that dies (or finishes) before
            // emitting the ready markers is a startup failure.
            let exited = match guard.child_mut().try_wait() {
                Ok(maybe_exit) => maybe_exit,
                Err(err) => {
                    return Err(fail_startup(
                        guard.defuse(),
                        pid,
                        &stderr,
                        &format!("supervise wait failed: {err}"),
                        self.config.error_capture_bytes,
                    )
                    .await);
                }
            };
            if let Some(exit) = exited {
                return Err(fail_startup(
                    guard.defuse(),
                    pid,
                    &stderr,
                    &format!("supervise exited before ready ({exit})"),
                    self.config.error_capture_bytes,
                )
                .await);
            }

            let snapshot = stderr.snapshot();
            if is_panicked(&snapshot) {
                return Err(fail_startup(
                    guard.defuse(),
                    pid,
                    &stderr,
                    "supervise panicked during startup",
                    self.config.error_capture_bytes,
                )
                .await);
            }
            if is_ready(&snapshot) {
                break;
            }
            if started.elapsed() >= ready_timeout {
                return Err(fail_startup(
                    guard.defuse(),
                    pid,
                    &stderr,
                    &format!(
                        "supervise not ready after {}s",
                        self.config.ready_timeout_secs
                    ),
                    self.config.error_capture_bytes,
                )
                .await);
            }
            tokio::time::sleep(READY_POLL_INTERVAL).await;
        }

        tracing::info!(
            package_name = %pkg.package_name,
            pid,
            runtime_dir = %runtime_dir.display(),
            ready_in_ms = started.elapsed().as_millis() as u64,
            "session.ready"
        );

        Ok(RunningSession {
            pid,
            runtime_dir,
            package_dir,
            child: guard.defuse(),
            package_guard: Some(package_guard),
            runtime_guard: Some(runtime_guard),
            stderr,
            terminal_status: None,
            log_tail_lines: self.config.log_tail_lines,
        })
    }
}

/// Kill the startup-failed child's whole group, reap it (no zombies), and
/// build the `StartupFailed` error carrying `reason` plus the stderr tail.
async fn fail_startup(
    mut child: Child,
    pid: i32,
    stderr: &StderrBuffer,
    reason: &str,
    error_capture_bytes: usize,
) -> RunnerError {
    kill_group_quiet(pid);
    let _ = child.wait().await;

    let tail = stderr_tail(&stderr.snapshot(), error_capture_bytes);
    tracing::error!(pid, reason, stderr = %tail, "session.start: startup failed");
    RunnerError::StartupFailed {
        stderr: format!("{reason}\n{tail}"),
    }
}

/// Keep the NEWEST `cap` bytes of `text`, cutting at a char boundary (the
/// end of the stderr stream carries the exit/panic reason).
fn stderr_tail(text: &str, cap: usize) -> String {
    if text.len() <= cap {
        return text.to_string();
    }
    let mut start = text.len() - cap;
    while start < text.len() && !text.is_char_boundary(start) {
        start += 1;
    }
    text[start..].to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::PathBuf;

    use nix::sys::signal::Signal;
    use nix::unistd::Pid;

    use super::*;
    use crate::engine::process::signal_group;
    use crate::packages::model::PackageFile;

    /// Conformance branch that passes; supervise body is per-test.
    fn engine_stub(dir: &Path, supervise_body: &str) -> PathBuf {
        let path = dir.join("stub-framework.sh");
        let script = format!(
            r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0
    ;;
  supervise)
{supervise_body}
    ;;
esac
"#
        );
        fs::write(&path, script).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    /// Stub whose conformance branch is custom and supervise never runs.
    fn conformance_stub(dir: &Path, conformance_body: &str) -> PathBuf {
        let path = dir.join("stub-conformance.sh");
        let script = format!(
            r#"#!/bin/sh
case "$1" in
  conformance)
{conformance_body}
    ;;
  supervise)
    echo "supervise must not run" >&2
    exit 97
    ;;
esac
"#
        );
        fs::write(&path, script).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    const READY_SUPERVISE: &str = r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 30"#;

    fn config(bin: &Path, temp_root: &Path) -> EngineConfig {
        EngineConfig {
            framework_bin: bin.to_path_buf(),
            temp_root: temp_root.to_path_buf(),
            candidate_prefix: "candidate/".to_string(),
            candidate_from_sep: "::".to_string(),
            stop_grace_secs: 5,
            conformance_timeout_secs: 5,
            ready_timeout_secs: 5,
            error_capture_bytes: 8192,
            log_tail_lines: 200,
        }
    }

    fn minimal_package() -> PreparedPackage {
        PreparedPackage {
            package_name: "demo".to_string(),
            files: vec![
                PackageFile {
                    path: "departments/hello/main.lua".to_string(),
                    content: "local M = {}\nM.spec = { consumes = { \"tick\" } }\n\
                              function pipeline(event) end\nreturn M\n"
                        .to_string(),
                },
                PackageFile {
                    path: "raisers/tick.lua".to_string(),
                    content: "return { type = \"cron\", interval = \"1s\", produces = \"tick\" }\n"
                        .to_string(),
                },
            ],
            composed_deps: Vec::new(),
        }
    }

    /// Count `fkst-*` entries under the test temp root (leak scan).
    fn fkst_entries(temp_root: &Path) -> usize {
        fs::read_dir(temp_root)
            .expect("read temp root")
            .flatten()
            .filter(|entry| entry.file_name().to_string_lossy().starts_with("fkst-"))
            .count()
    }

    async fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        for _ in 0..160 {
            if predicate() {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        false
    }

    // ---- happy path -----------------------------------------------------------

    #[tokio::test]
    async fn start_runs_ready_session_and_stop_reaps_group_and_cleans_dirs() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");

        // Own process group: PGID == PID.
        let pgid = nix::unistd::getpgid(Some(Pid::from_raw(session.pid))).expect("getpgid");
        assert_eq!(pgid.as_raw(), session.pid);

        // Materialized tree + fkst.env + runtime layout.
        assert!(session
            .package_dir
            .join("departments/hello/main.lua")
            .is_file());
        assert!(session.package_dir.join("raisers/tick.lua").is_file());
        assert_eq!(
            fs::read(session.package_dir.join("fkst.env")).expect("fkst.env"),
            b"FKST_CANDIDATE_FROM_SEP=::\nFKST_CANDIDATE_PREFIX=candidate/\n"
        );
        assert!(session.runtime_dir.join("durable").is_dir());
        assert!(session.runtime_dir.is_absolute());

        assert_eq!(runner.status(&mut session), LiveStatus::Running);
        assert!(SessionRunner::is_pid_alive(session.pid));

        // The held stderr buffer surfaces the engine's ready markers.
        let engine_stderr = session.engine_stderr();
        assert!(engine_stderr.contains("event runtime running handles="));
        assert!(engine_stderr.contains("consumer started dept=hello"));

        // No engine ran, so no child logs yet: None is the documented answer.
        assert_eq!(session.tail_logs(), None);

        let pid = session.pid;
        runner.stop(&mut session).await.expect("stop");

        assert_eq!(runner.status(&mut session), LiveStatus::Stopped);
        // The whole group is gone.
        assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
        assert!(!session.package_dir.exists());
        assert!(!session.runtime_dir.exists());
        assert_eq!(fkst_entries(temp_root.path()), 0, "no leaked fkst-* dirs");
    }

    #[tokio::test]
    async fn tail_logs_surfaces_engine_child_logs() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");

        // Simulate the engine writing a child log after the first dispatch.
        let log_dir = session.runtime_dir.join("logs").join("framework-child");
        fs::create_dir_all(&log_dir).expect("log dir");
        fs::write(log_dir.join("hello-1-2-0.log"), "CMD=stub\nframework ok\n").expect("write log");

        assert_eq!(
            session.tail_logs().as_deref(),
            Some("CMD=stub\nframework ok")
        );
        // The runner-level surface reads the same tail from a bare path.
        assert_eq!(
            runner.tail_logs(&session.runtime_dir).as_deref(),
            Some("CMD=stub\nframework ok")
        );

        runner.stop(&mut session).await.expect("stop");
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    // ---- validation -------------------------------------------------------------

    #[tokio::test]
    async fn invalid_package_is_rejected_before_any_temp_dir() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut pkg = minimal_package();
        pkg.files.clear();
        let err = runner.start(&pkg).await.expect_err("empty package");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert_eq!(fkst_entries(temp_root.path()), 0, "no temp dir on reject");
    }

    // ---- conformance ---------------------------------------------------------------

    #[tokio::test]
    async fn conformance_check_failure_maps_to_exit_one_and_cleans_dirs() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = conformance_stub(
            stub_dir.path(),
            r#"    echo "FAIL graph-scan department broken missing M.spec" >&2
    exit 1"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let err = runner
            .start(&minimal_package())
            .await
            .expect_err("conformance fail");
        match err {
            RunnerError::ConformanceFailed { code, stderr } => {
                assert_eq!(code, 1, "engine check failure is the 400-class code");
                assert!(stderr.contains("FAIL graph-scan"), "{stderr}");
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }
        assert_eq!(fkst_entries(temp_root.path()), 0, "dirs cleaned on fail");
    }

    #[tokio::test]
    async fn conformance_sdk_error_maps_to_exit_two() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = conformance_stub(
            stub_dir.path(),
            r#"    echo "[framework] startup error: boom" >&2
    exit 2"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let err = runner.start(&minimal_package()).await.expect_err("exit 2");
        assert!(
            matches!(err, RunnerError::ConformanceFailed { code: 2, .. }),
            "got {err:?}"
        );
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn conformance_timeout_group_kills_and_cleans_dirs() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = conformance_stub(stub_dir.path(), "    sleep 60");
        let mut cfg = config(&bin, temp_root.path());
        cfg.conformance_timeout_secs = 1;
        let runner = SessionRunner::new(cfg);

        let err = runner.start(&minimal_package()).await.expect_err("hang");
        match err {
            RunnerError::ConformanceFailed { code, stderr } => {
                assert_eq!(code, -1);
                assert!(stderr.contains("timed out"), "{stderr}");
            }
            other => panic!("expected ConformanceFailed, got {other:?}"),
        }
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    // ---- spawn / startup failures ----------------------------------------------------

    #[tokio::test]
    async fn missing_framework_bin_is_a_spawn_error_with_clean_dirs() {
        let temp_root = tempfile::tempdir().expect("temp root");
        let runner = SessionRunner::new(config(
            Path::new("/definitely/missing/fkst-framework"),
            temp_root.path(),
        ));

        let err = runner
            .start(&minimal_package())
            .await
            .expect_err("missing bin");
        assert!(matches!(err, RunnerError::Spawn(_)), "got {err:?}");
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn supervise_exit_before_ready_is_startup_failed_with_stderr_tail() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "[framework] startup error: FKST_DURABLE_ROOT must be set" >&2
    exit 2"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let err = runner
            .start(&minimal_package())
            .await
            .expect_err("exit-first");
        match err {
            RunnerError::StartupFailed { stderr } => {
                assert!(stderr.contains("exited before ready"), "{stderr}");
                assert!(stderr.contains("FKST_DURABLE_ROOT must be set"), "{stderr}");
            }
            other => panic!("expected StartupFailed, got {other:?}"),
        }
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn supervise_panic_is_startup_failed_and_group_is_killed() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Leave a pid breadcrumb OUTSIDE the runtime dir (which is cleaned).
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo $$ > "$FKST_RUNTIME_ROOT/../supervise.pid"
    echo "event runtime running handles=3" >&2
    echo "thread 'main' (1) panicked at consumer.rs:59:14:" >&2
    sleep 30"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let err = runner.start(&minimal_package()).await.expect_err("panic");
        match err {
            RunnerError::StartupFailed { stderr } => {
                assert!(stderr.contains("panicked"), "{stderr}");
            }
            other => panic!("expected StartupFailed, got {other:?}"),
        }

        let pid: i32 = fs::read_to_string(temp_root.path().join("supervise.pid"))
            .expect("pid breadcrumb")
            .trim()
            .parse()
            .expect("pid");
        assert!(
            wait_until(move || !is_pid_alive(pid)).await,
            "panicked supervise group must be killed"
        );
        fs::remove_file(temp_root.path().join("supervise.pid")).expect("rm breadcrumb");
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn half_alive_runtime_running_without_consumers_times_out() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // The A3 guard: `event runtime running` alone must NOT count as
        // ready (it is emitted even in the half-alive mode, spike Q9).
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo $$ > "$FKST_RUNTIME_ROOT/../supervise.pid"
    echo "event runtime running handles=3" >&2
    sleep 30"#,
        );
        let mut cfg = config(&bin, temp_root.path());
        cfg.ready_timeout_secs = 2;
        let runner = SessionRunner::new(cfg);

        let err = runner
            .start(&minimal_package())
            .await
            .expect_err("half-alive must not become ready");
        match err {
            RunnerError::StartupFailed { stderr } => {
                assert!(stderr.contains("not ready after 2s"), "{stderr}");
            }
            other => panic!("expected StartupFailed, got {other:?}"),
        }

        // Under pathological host load the stub may have been killed before
        // writing its breadcrumb — then there is no live group to assert
        // against (the group-kill path is also covered by the panic test).
        match fs::read_to_string(temp_root.path().join("supervise.pid")) {
            Ok(raw) => {
                let pid: i32 = raw.trim().parse().expect("pid");
                assert!(
                    wait_until(move || !is_pid_alive(pid)).await,
                    "timed-out supervise group must be killed"
                );
                fs::remove_file(temp_root.path().join("supervise.pid")).expect("rm breadcrumb");
            }
            Err(_) => eprintln!("NOTE: stub killed before writing its pid breadcrumb"),
        }
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    // ---- cancellation safety ----------------------------------------------------------

    #[tokio::test]
    async fn dropping_start_mid_ready_wait_kills_and_reaps_the_spawned_group() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Never goes ready: start() can only leave the ready-wait via the
        // timeout — or via being DROPPED, which is what this test does.
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo $$ > "$FKST_RUNTIME_ROOT/../supervise.pid"
    echo "event runtime running handles=3" >&2
    sleep 30"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));
        let pkg = minimal_package();
        let pid_file = temp_root.path().join("supervise.pid");

        // Race start() against the stub's pid breadcrumb inside select!, so
        // the future is dropped mid-ready-wait (the axum-disconnect shape).
        {
            let fut = runner.start(&pkg);
            tokio::pin!(fut);
            tokio::select! {
                res = &mut fut => panic!("start must still be mid-ready-wait, got {res:?}"),
                _ = async {
                    while !pid_file.exists() {
                        tokio::time::sleep(Duration::from_millis(25)).await;
                    }
                } => {}
            }
        } // <- start() future (armed guard + temp-dir guards) dropped here

        let pid: i32 = fs::read_to_string(&pid_file)
            .expect("pid breadcrumb")
            .trim()
            .parse()
            .expect("pid");
        // is_pid_alive turns false only once the child is REAPED (a zombie
        // still answers kill(pid, 0)), so this asserts kill AND reap.
        assert!(
            wait_until(move || !is_pid_alive(pid)).await,
            "dropped start() must kill and reap the supervise group"
        );
        fs::remove_file(&pid_file).expect("rm breadcrumb");
        assert_eq!(
            fkst_entries(temp_root.path()),
            0,
            "temp dirs cleaned when the start future is dropped"
        );
    }

    // ---- status ---------------------------------------------------------------------

    #[tokio::test]
    async fn clean_self_exit_reports_stopped_and_caches() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // The 2 s lifetime leaves a wide window for the ready-wait to
        // observe the markers BEFORE self-exit, even under test-suite load.
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 2
    exit 0"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");
        assert_eq!(session.status(), LiveStatus::Running);

        assert!(
            wait_until(|| session.status() != LiveStatus::Running).await,
            "stub must self-exit"
        );
        assert_eq!(session.status(), LiveStatus::Stopped);
        // Cached: stable on repeat.
        assert_eq!(session.status(), LiveStatus::Stopped);

        runner
            .stop(&mut session)
            .await
            .expect("stop after self-exit");
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn out_of_band_kill_reports_failed_signal_and_stop_stays_ok() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");
        signal_group(session.pid, Signal::SIGKILL).expect("out-of-band kill");

        assert!(
            wait_until(|| session.status() != LiveStatus::Running).await,
            "killed session must turn terminal"
        );
        assert_eq!(
            session.status(),
            LiveStatus::Failed {
                code: None,
                signal: Some(9)
            }
        );

        // stop() after the out-of-band kill: ESRCH swallowed, Ok, dirs
        // cleaned; the earlier Failed observation is kept.
        runner.stop(&mut session).await.expect("stop must stay Ok");
        assert_eq!(
            session.status(),
            LiveStatus::Failed {
                code: None,
                signal: Some(9)
            }
        );
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    // ---- stop idempotency ------------------------------------------------------------

    #[tokio::test]
    async fn double_stop_is_a_no_op_success() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
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

    // ---- drop semantics ----------------------------------------------------------------

    #[tokio::test]
    async fn dropping_the_handle_never_kills_a_live_engine() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let session = runner.start(&minimal_package()).await.expect("start");
        let pid = session.pid;
        drop(session);

        // kill_on_drop(false): the engine survives the dropped handle...
        assert!(SessionRunner::is_pid_alive(pid), "engine must survive drop");
        // ...while the temp-dir guards were released by the drop.
        assert_eq!(fkst_entries(temp_root.path()), 0, "dirs cleaned on drop");

        // Test hygiene: reap the intentionally-orphaned stub (tokio's
        // orphan reaper collects the zombie).
        signal_group(pid, Signal::SIGKILL).expect("cleanup kill");
        assert!(wait_until(move || !is_pid_alive(pid)).await);
    }

    // ---- helpers -----------------------------------------------------------------------

    #[test]
    fn stderr_tail_keeps_the_newest_bytes_at_char_boundaries() {
        assert_eq!(stderr_tail("abcdef", 4), "cdef");
        assert_eq!(stderr_tail("abc", 8), "abc");
        // 2-byte chars: a cap landing mid-char advances to the boundary.
        assert_eq!(stderr_tail("ααα", 3), "α");
        assert_eq!(stderr_tail("ααα", 4), "αα");
    }

    #[test]
    fn live_status_serializes_with_a_state_tag() {
        assert_eq!(
            serde_json::to_value(LiveStatus::Running).unwrap(),
            serde_json::json!({ "state": "running" })
        );
        assert_eq!(
            serde_json::to_value(LiveStatus::Failed {
                code: Some(2),
                signal: None
            })
            .unwrap(),
            serde_json::json!({ "state": "failed", "code": 2, "signal": null })
        );
    }
}
