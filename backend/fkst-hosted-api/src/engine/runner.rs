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

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use tempfile::TempDir;
use tokio::process::Child;

use crate::engine::config::EngineConfig;
use crate::engine::error::RunnerError;
use crate::engine::logs::tail_child_logs;
use crate::engine::materialize::{materialize_packages, write_fkst_env, PreparedPackage};
use crate::engine::process::{
    is_panicked, is_pid_alive, is_ready, kill_group_quiet, reap_with_grace, run_conformance,
    spawn_supervise, ChildGroupGuard, GoalEnv, OutputBuffer, SpawnedChild,
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

/// Goal context for a session started from a goal. Carries the goal identity
/// and the GitHub token needed by the engine's GitHub integration. The token
/// is `secrecy::SecretString` so callers cannot accidentally log it; it stays a
/// `SecretString` all the way into the process layer's `GoalEnv` (#109), which
/// exposes it only at the `.env(...)` set-site.
#[derive(Debug, Clone)]
pub struct GoalContext {
    pub goal_id: bson::Uuid,
    pub title: String,
    pub description: String,
    pub repo: crate::models::RepoRef,
    pub github_token: secrecy::SecretString,
    /// Expiry of `github_token`, written as RFC3339 into the `0600` token file
    /// so the credential helper can decide whether to force a just-in-time
    /// re-mint (issue #107).
    pub token_expires_at: std::time::SystemTime,
}

/// Specification for starting a session with one or more packages and an
/// optional goal context. Single-package, no-goal specs with an empty
/// `env_profile` produce behavior that is byte-identical (modulo the #101 env
/// isolation) to the existing [`SessionRunner::start`] path.
///
/// `Default` is derived so the per-session injection path (#102) and the
/// goal-token path (#106) can build a spec field-by-field; an empty
/// `env_profile` is the back-compat default the live driver keeps using until
/// the injection issue switches it over.
#[derive(Debug, Default)]
pub struct StartSpec {
    /// Ordered list of prepared packages (at least one).
    pub packages: Vec<PreparedPackage>,
    /// Goal context; `None` for classic (non-goal) sessions.
    pub goal: Option<GoalContext>,
    /// Per-session env applied to the engine child via the isolated-env
    /// mechanism (#101). Values are `SecretString` — never logged. Reserved
    /// keys (`is_reserved_env_key`) are dropped at the spawn seam. Resolved and
    /// populated by the injection path (#102); empty for classic sessions.
    pub env_profile: BTreeMap<String, secrecy::SecretString>,
    /// Per-session `CODEX_HOME` directory holding the rendered codex
    /// `config.toml` (#112). When `Some`, it is set as a platform-managed env
    /// var on the supervise child (layered AFTER the host allow-list, so it
    /// always wins over the allow-listed `CODEX_HOME`). The DRIVER owns this
    /// directory's lifecycle — the runner only points the child at it and never
    /// creates or removes it. `None` for classic / minimal runs, which keeps
    /// the pre-#112 behaviour byte-identical.
    pub codex_home: Option<PathBuf>,
    /// Repo-scoped package source (issue #115). When `Some`, the runner does NOT
    /// materialize `packages` into temp dirs: it points `--project-root` at this
    /// cloned-repo working tree and `--package-root` at each
    /// [`Self::package_roots`] entry (already-existing `<repo>/.fkst/packages/<name>`
    /// dirs the DRIVER owns and cleans). `packages` is then ignored for the
    /// on-disk tree (it may stay empty). `None` keeps the classic materialize
    /// path (used by tests / minimal runs). The driver holds the clone's TempDir
    /// guards for the session lifetime, mirroring the `codex_home` ownership.
    pub project_root: Option<PathBuf>,
    /// Canonicalized `<repo>/.fkst/packages/<name>` dirs, one per package, in
    /// order. Only consulted when [`Self::project_root`] is `Some`.
    pub package_roots: Vec<PathBuf>,
}

/// Live handle for one engine process, held by the caller for the session's
/// lifetime. Owns the spawned [`Child`] and the temp-dir guards; dropping
/// the handle removes the dirs but NEVER kills a live engine
/// (`kill_on_drop(false)` — lifecycle is explicit via [`Self::stop`]).
#[derive(Debug)]
pub struct RunningSession {
    /// PID of the supervise process; == PGID (own process group).
    pub pid: i32,
    /// Absolute `FKST_RUNTIME_ROOT` for this run.
    pub runtime_dir: PathBuf,
    /// Absolute materialized package root for this run (the FIRST / primary
    /// package root). Always equal to `package_dirs[0]`.
    pub package_dir: PathBuf,
    /// ALL materialized package root paths (one per package in the spec).
    /// For single-package sessions this is `[package_dir]`.
    pub package_dirs: Vec<PathBuf>,
    child: Child,
    /// Temp-dir guards for ALL materialized packages (one per package).
    /// Dropped during cleanup to remove the on-disk trees.
    package_guards: Vec<TempDir>,
    runtime_guard: Option<TempDir>,
    /// Merged stdout+stderr ring buffer fed by the spawn drains (issue #50):
    /// ready markers come from stdout, panics/exit reasons from stderr.
    output: OutputBuffer,
    /// Line-framed stdout stream (the journaling layer's `RAISED:` source,
    /// issue #25), fanned out from the same stdout drain that feeds `output`;
    /// taken at most once via [`Self::take_stdout`].
    stdout_lines: Option<tokio::sync::mpsc::Receiver<Vec<u8>>>,
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

    /// Snapshot of the engine's recent output (the bounded ring buffer the
    /// stdout+stderr drain tasks feed) — the place to look when a ready
    /// session later turns `Failed`.
    pub fn engine_stderr(&self) -> String {
        self.output.snapshot()
    }

    /// Take ownership of the engine's line-framed stdout stream (at most
    /// once; subsequent calls return `None`). The journaling layer consumes
    /// this for `RAISED:` parsing; leaving it untaken is safe — the drain
    /// task keeps the pipe flowing and discards lines.
    pub fn take_stdout(&mut self) -> Option<tokio::sync::mpsc::Receiver<Vec<u8>>> {
        self.stdout_lines.take()
    }

    /// Stop the session: SIGTERM the process GROUP, wait up to `grace`,
    /// escalate to SIGKILL, always reap, then clean both temp dirs.
    ///
    /// Idempotent: an already-dead/absent group (`ESRCH`, out-of-band kill,
    /// double stop) is a no-op success; the temp-dir guards are taken
    /// exactly once. A pre-existing terminal `Failed` observation is kept
    /// (stop does not rewrite history). When the reap observed an exit that
    /// PREDATES its own signalling (a crash racing this stop), that exit is
    /// classified — a crash must surface as the crash, never as `Stopped`;
    /// an exit caused by our SIGTERM/SIGKILL is the normal `Stopped`.
    pub async fn stop(&mut self, grace: Duration) -> Result<(), RunnerError> {
        tracing::info!(pid = self.pid, "session.stopping");
        let result = reap_with_grace(&mut self.child, self.pid, grace).await;
        match result {
            Ok(outcome) => {
                if self.terminal_status.is_none() {
                    self.terminal_status = Some(match outcome.pre_signal_exit {
                        Some(exit) => classify_exit(exit),
                        None => LiveStatus::Stopped,
                    });
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
        let pkg_count = self.package_guards.len();
        if pkg_count > 0 {
            self.package_guards.clear();
            dirs_removed += pkg_count;
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

    /// The configured engine temp root — the parent dir for the ephemeral
    /// `fkst-pkg-*` / `fkst-rt-*` (and #112's `fkst-codex-*`) session dirs. The
    /// driver needs it to place a per-session CODEX_HOME on the same filesystem
    /// as the runtime dirs (so it shares their cleanup/reconcile semantics).
    pub fn temp_root(&self) -> &Path {
        &self.config.temp_root
    }

    /// The configured `fkst-framework` binary path. The driver passes it to the
    /// repo-clone step (#115) so the clone's credential helper can be wired with
    /// the same engine context the in-session git uses.
    pub fn framework_bin(&self) -> &Path {
        &self.config.framework_bin
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

    /// Start a single-package session (classic / pre-goal path).
    /// Delegates to [`Self::start_with_spec`] with a single-package, no-goal
    /// spec. The behavior is byte-identical to the pre-`start_with_spec`
    /// implementation.
    pub async fn start(&self, pkg: &PreparedPackage) -> Result<RunningSession, RunnerError> {
        let spec = StartSpec {
            packages: vec![pkg.clone()],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        self.start_with_spec(&spec).await
    }

    /// Start a session from a [`StartSpec`]: validate all packages ->
    /// materialize each into its own `fkst-pkg-<name>-*` dir + `fkst.env` ->
    /// create runtime dir + `durable/` -> write goal files if present ->
    /// conformance pre-flight per root -> spawn `supervise` with multi-root
    /// and optional goal env -> bounded ready-wait.
    ///
    /// For a single-package spec without a goal, the behavior is byte-identical
    /// to the classic [`Self::start`] path (CANON: existing tests stay green).
    ///
    /// Every failure path cleans all temp dirs before returning.
    pub async fn start_with_spec(&self, spec: &StartSpec) -> Result<RunningSession, RunnerError> {
        // Repo-scoped path (#115): the driver cloned the goal repo and resolved
        // `<repo>/.fkst/packages/<name>` dirs, which it owns and cleans — so the
        // runner points the engine at those dirs directly (no copy, no
        // materialize). The classic path materializes `packages` into temp dirs.
        let repo_scoped = spec.project_root.is_some();

        // 1. Determine the package roots + the project root.
        //    - repo-scoped: pre-existing dirs the driver supplied;
        //    - classic: materialize each PreparedPackage into its own temp dir,
        //      with the FIRST materialized dir doubling as the project root.
        let (package_guards, package_dirs, project_root): (Vec<TempDir>, Vec<PathBuf>, PathBuf) =
            if let Some(project_root) = &spec.project_root {
                if spec.package_roots.is_empty() {
                    return Err(RunnerError::InvalidPackage(
                        "repo-scoped spec must contain at least one package root".to_string(),
                    ));
                }
                // The dirs already exist on disk (the clone); canonicalize for the
                // safe argv + a stable project root. No guards: the driver holds
                // the clone's TempDir guards for the session lifetime.
                let project_root = project_root.canonicalize().map_err(RunnerError::Io)?;
                let mut dirs = Vec::with_capacity(spec.package_roots.len());
                for root in &spec.package_roots {
                    dirs.push(root.canonicalize().map_err(RunnerError::Io)?);
                }
                (Vec::new(), dirs, project_root)
            } else {
                // Classic path: validate then materialize. No temp dir is created
                // on the reject path.
                if spec.packages.is_empty() {
                    return Err(RunnerError::InvalidPackage(
                        "spec must contain at least one package".to_string(),
                    ));
                }
                for pkg in &spec.packages {
                    pkg.validate()?;
                }
                let guards = materialize_packages(&spec.packages, &self.config.temp_root)?;
                let mut dirs = Vec::with_capacity(guards.len());
                for guard in &guards {
                    dirs.push(guard.path().canonicalize().map_err(RunnerError::Io)?);
                }
                // The first materialized dir is the project root (classic CANON).
                let project_root = dirs[0].clone();
                (guards, dirs, project_root)
            };

        // 2. Write the host-owned 2-key `fkst.env` into every package root. For
        //    repo-scoped dirs this lands in the throwaway clone (removed with the
        //    session); the engine contract requires it for packages that use the
        //    candidate-git SDK.
        for dir in &package_dirs {
            write_fkst_env(
                dir,
                &self.config.candidate_prefix,
                &self.config.candidate_from_sep,
            )?;
        }

        // 3. Fresh runtime root with a fresh durable root per attempt.
        let runtime_guard = tempfile::Builder::new()
            .prefix("fkst-rt-")
            .tempdir_in(&self.config.temp_root)
            .map_err(RunnerError::Io)?;
        std::fs::create_dir(runtime_guard.path().join("durable")).map_err(RunnerError::Io)?;

        let runtime_dir = runtime_guard
            .path()
            .canonicalize()
            .map_err(RunnerError::Io)?;

        // 4. Write goal files if a goal context is present.
        let goal_env = if let Some(goal) = &spec.goal {
            let goal_json_path = runtime_dir.join("goal.json");
            let goal_json = serde_json::json!({
                "goal_id": goal.goal_id.to_string(),
                "title": goal.title,
                "description": goal.description,
                "repo": goal.repo,
            });
            std::fs::write(&goal_json_path, goal_json.to_string()).map_err(RunnerError::Io)?;
            tracing::debug!(
                goal_id = %goal.goal_id,
                path = %goal_json_path.display(),
                bytes = goal_json.to_string().len(),
                "session.prepare.goal_json"
            );

            // Token file: JSON `{token, expires_at}` at mode 0600, written
            // atomically (#107). The expiry lets the credential helper force a
            // just-in-time re-mint near expiry; the token value is never logged.
            let token_path = runtime_dir.join(crate::engine::TOKEN_FILE_NAME);
            crate::engine::write_token_file(
                &token_path,
                &goal.github_token,
                goal.token_expires_at,
            )?;
            tracing::debug!(
                path = %token_path.display(),
                "session.prepare.github_token"
            );

            // Credential helper (#107): drop the 0700 script into the runtime
            // dir; git is pointed at it via the GIT_CONFIG_* env in spawn.
            let helper_path = crate::engine::materialize_helper_script(&runtime_dir)?;

            // Per-session JIT mint nonce: written 0600 next to the token and
            // handed to the helper via FKST_GITHUB_MINT_NONCE so it can
            // authenticate a re-mint request to the driver's poller.
            let mint_nonce = generate_mint_nonce();
            crate::engine::goal_token::write_nonce_file(&runtime_dir, &mint_nonce)?;

            // The token + nonce stay SecretString into GoalEnv (#109/#107);
            // clone the context's secret rather than expose-then-rewrap a String.
            Some(GoalEnv {
                github_token: goal.github_token.clone(),
                github_token_file: token_path,
                goal_file: goal_json_path,
                helper_path,
                mint_nonce: secrecy::SecretString::from(mint_nonce),
            })
        } else {
            None
        };

        // 5. Conformance pre-flight: run once per package root, sequential,
        //    first failure aborts. For repo-scoped packages this is the only
        //    structural gate (the clone dir is untrusted user content not run
        //    through PreparedPackage::validate); a malformed package fails it
        //    loudly with a clear ConformanceFailed rather than running broken.
        for pkg_dir in &package_dirs {
            run_conformance(
                &self.config.framework_bin,
                pkg_dir,
                &runtime_dir,
                Duration::from_secs(self.config.conformance_timeout_secs),
                self.config.error_capture_bytes,
                &spec.env_profile,
            )
            .await?;
        }

        // 6. Spawn supervise in its own process group. The per-session
        //    CODEX_HOME (when present) is passed as a platform-managed var so
        //    codex discovers the rendered config.toml; the driver owns its
        //    lifecycle, so the runner never creates or cleans it.
        let spawned = spawn_supervise(
            &self.config.framework_bin,
            &project_root,
            &package_dirs,
            &runtime_dir,
            &spec.env_profile,
            goal_env.as_ref(),
            spec.codex_home.as_deref(),
        )?;

        // 7. Bounded ready-wait. Every failure path group-kills, reaps, and
        //    (by dropping the guards still held here) cleans all dirs.
        //    Cancellation safety: the armed ChildGroupGuard ensures that
        //    dropping THIS future mid-ready-wait (client disconnect, outer
        //    select!/timeout) also group-kills and reaps the spawned engine;
        //    it is defused when ownership moves to the RunningSession or to
        //    fail_startup (which kills + reaps itself).
        let SpawnedChild {
            child,
            pid,
            output,
            // The drain tasks run for the child's lifetime; reaping the group
            // EOFs both pipes, ending them. They are not aborted here on the
            // success path because the merged buffer must stay readable via
            // engine_stderr() for the whole session, and the stdout fan-out
            // must keep feeding the journal stream.
            drains: _drains,
            stdout_lines,
        } = spawned;
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
                        &output,
                        &format!("supervise wait failed: {err}"),
                        self.config.error_capture_bytes,
                    )
                    .await);
                }
            };
            if let Some(exit) = exited {
                // Ready-check FIRST: an engine that emitted BOTH ready
                // markers and then exited within one poll tick is a
                // successful start whose exit `status()` will report — not
                // a StartupFailed. Give the drain task one tick to flush
                // the remaining buffered output to EOF before judging.
                tokio::time::sleep(READY_POLL_INTERVAL).await;
                let snapshot = output.snapshot();
                if is_ready(&snapshot) && !is_panicked(&snapshot) {
                    break;
                }
                return Err(fail_startup(
                    guard.defuse(),
                    pid,
                    &output,
                    &format!("supervise exited before ready ({exit})"),
                    self.config.error_capture_bytes,
                )
                .await);
            }

            let snapshot = output.snapshot();
            if is_panicked(&snapshot) {
                return Err(fail_startup(
                    guard.defuse(),
                    pid,
                    &output,
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
                    &output,
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

        let package_dir = package_dirs[0].clone();
        tracing::info!(
            package_count = package_dirs.len(),
            repo_scoped,
            pid,
            runtime_dir = %runtime_dir.display(),
            has_goal = spec.goal.is_some(),
            ready_in_ms = started.elapsed().as_millis() as u64,
            "session.ready"
        );

        Ok(RunningSession {
            pid,
            runtime_dir,
            package_dir,
            package_dirs,
            child: guard.defuse(),
            package_guards,
            runtime_guard: Some(runtime_guard),
            output,
            stdout_lines: Some(stdout_lines),
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
    output: &OutputBuffer,
    reason: &str,
    error_capture_bytes: usize,
) -> RunnerError {
    kill_group_quiet(pid);
    let _ = child.wait().await;

    let tail = stderr_tail(&output.snapshot(), error_capture_bytes);
    tracing::error!(pid, reason, stderr = %tail, "session.start: startup failed");
    RunnerError::StartupFailed {
        stderr: format!("{reason}\n{tail}"),
    }
}

/// A 32-hex-char (128-bit) per-session JIT mint nonce. Random, unguessable, and
/// known only to this session's helper (via env) and its driver poller (via the
/// 0600 nonce file) — so only that session's own git child can trigger its own
/// re-mint (#107). `rand` is already a dependency (vault DEK/nonce).
fn generate_mint_nonce() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 16];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    bytes.iter().map(|b| format!("{b:02x}")).collect()
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
    use crate::engine::materialize::PackageFile;
    use crate::engine::process::signal_group;

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

    /// Issue #50: the REAL `supervise` emits its readiness markers on STDOUT
    /// (not stderr). This is the wiring the runner must handle for production
    /// to work; the stderr variant above is kept only for back-compat.
    const READY_SUPERVISE_STDOUT: &str = r#"    echo "event runtime running handles=3"
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
    sleep 30"#;

    fn config(bin: &Path, temp_root: &Path) -> EngineConfig {
        EngineConfig {
            framework_bin: bin.to_path_buf(),
            temp_root: temp_root.to_path_buf(),
            candidate_prefix: "candidate/".to_string(),
            candidate_from_sep: "::".to_string(),
            stop_grace_secs: 5,
            // Generous engine-level timeouts for the POSITIVE-path tests:
            // these spawn real `sh` engine children whose markers can lag
            // well past a few seconds under a saturated full-workspace run.
            // The negative timeout tests override these locally (and assert
            // their own short windows), so the wider default is safe.
            conformance_timeout_secs: 30,
            ready_timeout_secs: 30,
            error_capture_bytes: 8192,
            log_tail_lines: 200,
            github_token_refresh_secs: 2400,
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

    // ~20 s budget: real-engine-stub tests poll a spawned `sh` child's drained
    // output, which can lag well past a few seconds under a saturated
    // full-workspace run (many parallel spawns + containers).
    async fn wait_until(mut predicate: impl FnMut() -> bool) -> bool {
        for _ in 0..800 {
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

    /// Issue #50 regression: the readiness markers are written to STDOUT by
    /// the real engine. With the original stderr-only piping the markers were
    /// discarded and `start()` always timed out into `StartupFailed`. This
    /// test FAILS before the stdout-drain fix and PASSES after: `start()` must
    /// reach a live `RunningSession`, and `stop()` must reap it cleanly.
    #[tokio::test]
    async fn start_reaches_ready_when_markers_are_on_stdout() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE_STDOUT);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner
            .start(&minimal_package())
            .await
            .expect("start must reach ready from STDOUT markers");

        // A genuinely running session: alive, leading its own group.
        assert_eq!(runner.status(&mut session), LiveStatus::Running);
        assert!(SessionRunner::is_pid_alive(session.pid));
        let pgid = nix::unistd::getpgid(Some(Pid::from_raw(session.pid))).expect("getpgid");
        assert_eq!(pgid.as_raw(), session.pid, "child must lead its own group");

        // The merged buffer surfaces the STDOUT-emitted markers.
        let engine_output = session.engine_stderr();
        assert!(engine_output.contains("event runtime running handles="));
        assert!(engine_output.contains("consumer started dept=hello"));

        let pid = session.pid;
        runner.stop(&mut session).await.expect("stop reaps cleanly");
        assert_eq!(runner.status(&mut session), LiveStatus::Stopped);
        assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
        assert_eq!(fkst_entries(temp_root.path()), 0, "no leaked fkst-* dirs");
    }

    /// A3 half-alive guard at the runner level across the stream split:
    /// `event runtime running` on STDOUT with NO `consumer started` anywhere
    /// must NOT reach ready — `start()` must time out into `StartupFailed`,
    /// not spuriously succeed now that stdout is drained.
    #[tokio::test]
    async fn start_times_out_when_consumer_marker_is_absent_across_streams() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // runtime-running on stdout; a decoy on stderr; consumer line nowhere.
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3"
    echo "WARN consumer thread exited" >&2
    sleep 30"#,
        );
        let mut cfg = config(&bin, temp_root.path());
        cfg.ready_timeout_secs = 1;
        let runner = SessionRunner::new(cfg);

        let err = runner
            .start(&minimal_package())
            .await
            .expect_err("half-alive must not reach ready");
        match err {
            RunnerError::StartupFailed { stderr } => {
                assert!(stderr.contains("not ready after"), "{stderr}");
            }
            other => panic!("expected StartupFailed, got {other:?}"),
        }
        assert_eq!(fkst_entries(temp_root.path()), 0, "dirs cleaned on timeout");
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

    #[tokio::test]
    async fn take_stdout_yields_the_raised_stream_exactly_once() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    echo "RAISED: eyJkZXB0IjoiaGVsbG8ifQ=="
    sleep 30"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");
        let mut stdout = session.take_stdout().expect("stdout taken once");
        assert!(
            session.take_stdout().is_none(),
            "second take must return None"
        );
        let line = tokio::time::timeout(Duration::from_secs(20), stdout.recv())
            .await
            .expect("stdout line within 20s")
            .expect("channel open");
        assert_eq!(line, b"RAISED: eyJkZXB0IjoiaGVsbG8ifQ==".to_vec());

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
    async fn ready_then_immediate_clean_exit_is_a_successful_start() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Both ready markers, then a clean exit — all within one poll tick.
        // The exit must NOT be condemned as StartupFailed: the engine WAS
        // ready; status() reports the subsequent exit.
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    exit 0"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner
            .start(&minimal_package())
            .await
            .expect("ready-then-exit must start successfully");
        assert!(
            wait_until(|| session.status() != LiveStatus::Running).await,
            "stub has already exited"
        );
        assert_eq!(session.status(), LiveStatus::Stopped);

        runner.stop(&mut session).await.expect("stop");
        assert_eq!(session.status(), LiveStatus::Stopped);
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
                    // Wait for the breadcrumb to be present AND written: the
                    // shell creates the file (empty) before `echo $$` fills it,
                    // so gating on existence alone races the write and can read
                    // an empty file under heavy parallel load.
                    loop {
                        if fs::read_to_string(&pid_file)
                            .ok()
                            .is_some_and(|s| !s.trim().is_empty())
                        {
                            break;
                        }
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
    async fn post_ready_nonzero_exit_reports_failed_with_the_code() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Goes ready, runs briefly, then crashes with a non-zero code: the
        // session must turn Failed carrying that exact code (never Stopped).
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 2
    exit 3"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");
        assert!(
            wait_until(|| session.status() != LiveStatus::Running).await,
            "stub must self-exit"
        );
        assert_eq!(
            session.status(),
            LiveStatus::Failed {
                code: Some(3),
                signal: None
            }
        );
        // Cached: stable on repeat.
        assert_eq!(
            session.status(),
            LiveStatus::Failed {
                code: Some(3),
                signal: None
            }
        );

        // stop() keeps the earlier Failed observation (no history rewrite).
        runner.stop(&mut session).await.expect("stop after crash");
        assert_eq!(
            session.status(),
            LiveStatus::Failed {
                code: Some(3),
                signal: None
            }
        );
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

    #[tokio::test]
    async fn stop_racing_a_crash_surfaces_the_crash_not_stopped() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Goes ready, then crashes with code 7 — BEFORE stop() is called and
        // WITHOUT any status() poll observing it first.
        let bin = engine_stub(
            stub_dir.path(),
            r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 2
    exit 7"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut session = runner.start(&minimal_package()).await.expect("start");
        // Let the stub crash; deliberately NO status() call (which would
        // cache the terminal state) — stop() itself must classify the exit
        // it finds already waiting.
        tokio::time::sleep(Duration::from_millis(3500)).await;

        runner.stop(&mut session).await.expect("stop");
        assert_eq!(
            runner.status(&mut session),
            LiveStatus::Failed {
                code: Some(7),
                signal: None
            },
            "a crash racing stop() must surface as the crash, not Stopped"
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

    // ---- start_with_spec: single package, no goal (byte-identical to start) --

    #[tokio::test]
    async fn start_with_spec_single_package_no_goal_matches_start() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));
        let pkg = minimal_package();

        let spec = StartSpec {
            packages: vec![pkg.clone()],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let mut session = runner
            .start_with_spec(&spec)
            .await
            .expect("start_with_spec");

        // Same checks as the classic start test.
        assert_eq!(session.package_dirs.len(), 1);
        assert_eq!(session.package_dir, session.package_dirs[0]);
        assert!(session
            .package_dir
            .join("departments/hello/main.lua")
            .is_file());
        assert!(session.runtime_dir.join("durable").is_dir());
        assert_eq!(runner.status(&mut session), LiveStatus::Running);

        let pid = session.pid;
        runner.stop(&mut session).await.expect("stop");
        assert!(!session.package_dir.exists());
        assert!(!session.runtime_dir.exists());
        assert_eq!(fkst_entries(temp_root.path()), 0);
        assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
    }

    // ---- start_with_spec: multi-package ----------------------------------

    fn second_package() -> PreparedPackage {
        PreparedPackage {
            package_name: "second".to_string(),
            files: vec![
                PackageFile {
                    path: "departments/world/main.lua".to_string(),
                    content: "local M = {}\nM.spec = { consumes = { \"tock\" } }\n\
                              function pipeline(event) end\nreturn M\n"
                        .to_string(),
                },
                PackageFile {
                    path: "raisers/tock.lua".to_string(),
                    content: "return { type = \"cron\", interval = \"1s\", produces = \"tock\" }\n"
                        .to_string(),
                },
            ],
            composed_deps: Vec::new(),
        }
    }

    /// Multi-package stub that accepts multiple --package-root flags.
    /// The conformance branch echoes the flags it received so we can verify
    /// multi-root wiring; the supervise branch emits ready markers.
    fn multi_root_engine_stub(dir: &Path) -> PathBuf {
        let path = dir.join("stub-multi-root.sh");
        let script = r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan"
    exit 0
    ;;
  supervise)
    # Echo all args to stdout so the test can verify the multi-root wiring.
    echo "argv: $*"
    echo "GITHUB_TOKEN=${GITHUB_TOKEN:-UNSET}"
    echo "FKST_GOAL_FILE=${FKST_GOAL_FILE:-UNSET}"
    echo "FKST_GITHUB_TOKEN_FILE=${FKST_GITHUB_TOKEN_FILE:-UNSET}"
    echo "event runtime running handles=3"
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
    sleep 30
    ;;
esac
"#;
        fs::write(&path, script).expect("write stub");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
        path
    }

    #[tokio::test]
    async fn start_with_spec_multi_package_materializes_all_roots() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = multi_root_engine_stub(stub_dir.path());
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let spec = StartSpec {
            packages: vec![minimal_package(), second_package()],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let mut session = runner.start_with_spec(&spec).await.expect("start multi");

        assert_eq!(session.package_dirs.len(), 2);
        assert!(session.package_dirs[0]
            .join("departments/hello/main.lua")
            .is_file());
        assert!(session.package_dirs[1]
            .join("departments/world/main.lua")
            .is_file());
        assert_eq!(session.package_dir, session.package_dirs[0]);

        // The stub echoes its argv to stdout; verify multi-root flags.
        let mut stdout = session.take_stdout().expect("stdout");
        let first_line = tokio::time::timeout(Duration::from_secs(20), stdout.recv())
            .await
            .expect("argv line within 20s")
            .expect("channel open");
        let argv = String::from_utf8(first_line).expect("utf8");
        // Must contain --package-root for both roots.
        assert!(argv.contains("--package-root"), "argv: {argv}");
        // Count --package-root occurrences (should be 2 for 2 packages).
        let count = argv.matches("--package-root").count();
        assert_eq!(count, 2, "expected 2 --package-root flags, got: {argv}");

        // No goal env set.
        let token_line = tokio::time::timeout(Duration::from_secs(5), stdout.recv())
            .await
            .expect("token line")
            .expect("channel open");
        assert_eq!(token_line, b"GITHUB_TOKEN=UNSET");

        let pid = session.pid;
        runner.stop(&mut session).await.expect("stop");
        assert_eq!(fkst_entries(temp_root.path()), 0);
        assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
    }

    #[tokio::test]
    async fn start_with_spec_multi_package_cleans_all_dirs_on_conformance_fail() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        // Conformance always fails; we never reach supervise.
        let bin = conformance_stub(
            stub_dir.path(),
            r#"    echo "FAIL broken" >&2
    exit 1"#,
        );
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let spec = StartSpec {
            packages: vec![minimal_package(), second_package()],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let err = runner.start_with_spec(&spec).await.expect_err("must fail");
        assert!(matches!(
            err,
            RunnerError::ConformanceFailed { code: 1, .. }
        ));
        assert_eq!(fkst_entries(temp_root.path()), 0, "all dirs cleaned");
    }

    // ---- start_with_spec: goal context ------------------------------------

    fn goal_context() -> GoalContext {
        GoalContext {
            goal_id: bson::Uuid::parse_str("a1b2c3d4-e5f6-7890-abcd-ef1234567890").unwrap(),
            title: "Test goal".to_string(),
            description: "A test goal description.".to_string(),
            repo: crate::models::RepoRef {
                owner: "acme".to_string(),
                name: "test-repo".to_string(),
            },
            github_token: secrecy::SecretString::new("ghp_test_token_12345".to_string().into()),
            token_expires_at: std::time::SystemTime::now() + Duration::from_secs(3600),
        }
    }

    #[tokio::test]
    async fn start_with_spec_goal_session_writes_goal_files_and_sets_env() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = multi_root_engine_stub(stub_dir.path());
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let goal = goal_context();
        let spec = StartSpec {
            packages: vec![minimal_package()],
            goal: Some(goal),
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let mut session = runner
            .start_with_spec(&spec)
            .await
            .expect("start goal session");

        // goal.json written to runtime dir.
        let goal_json_path = session.runtime_dir.join("goal.json");
        assert!(goal_json_path.is_file(), "goal.json must exist");
        let goal_content = fs::read_to_string(&goal_json_path).expect("read goal.json");
        let parsed: serde_json::Value = serde_json::from_str(&goal_content).expect("parse json");
        assert_eq!(parsed["goal_id"], "a1b2c3d4-e5f6-7890-abcd-ef1234567890");
        assert_eq!(parsed["title"], "Test goal");
        assert_eq!(parsed["description"], "A test goal description.");
        assert_eq!(parsed["repo"]["owner"], "acme");
        assert_eq!(parsed["repo"]["name"], "test-repo");

        // github-token written as JSON {token, expires_at} with mode 0600 (#107).
        let token_path = session.runtime_dir.join("github-token");
        assert!(token_path.is_file(), "github-token must exist");
        let token_content = fs::read_to_string(&token_path).expect("read token");
        let token_json: serde_json::Value =
            serde_json::from_str(&token_content).expect("token file must be JSON");
        assert_eq!(token_json["token"], "ghp_test_token_12345");
        assert!(
            token_json["expires_at"].as_str().is_some(),
            "token file must carry an RFC3339 expires_at"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&token_path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600, "token file must be mode 0600");
        }

        // Credential helper materialized 0700 + the 0600 mint nonce (#107).
        let helper_path = session.runtime_dir.join(crate::engine::HELPER_SCRIPT_NAME);
        assert!(helper_path.is_file(), "credential helper must exist");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let hmode = fs::metadata(&helper_path)
                .expect("metadata")
                .permissions()
                .mode();
            assert_eq!(hmode & 0o777, 0o700, "helper must be mode 0700");
        }
        assert!(
            session
                .runtime_dir
                .join(crate::engine::NONCE_FILE_NAME)
                .is_file(),
            "mint nonce file must exist"
        );

        // The stub echoes env vars; verify goal env is set.
        let mut stdout = session.take_stdout().expect("stdout");
        let _argv_line = tokio::time::timeout(Duration::from_secs(20), stdout.recv())
            .await
            .expect("argv line")
            .expect("channel open");
        let token_line = tokio::time::timeout(Duration::from_secs(5), stdout.recv())
            .await
            .expect("token line")
            .expect("channel open");
        let token_str = String::from_utf8(token_line).expect("utf8");
        assert_eq!(token_str, "GITHUB_TOKEN=ghp_test_token_12345");

        let goal_file_line = tokio::time::timeout(Duration::from_secs(5), stdout.recv())
            .await
            .expect("goal file line")
            .expect("channel open");
        let goal_file_str = String::from_utf8(goal_file_line).expect("utf8");
        assert!(
            goal_file_str.starts_with("FKST_GOAL_FILE="),
            "got: {goal_file_str}"
        );
        assert!(goal_file_str.contains("goal.json"));

        let token_file_line = tokio::time::timeout(Duration::from_secs(5), stdout.recv())
            .await
            .expect("token file line")
            .expect("channel open");
        let token_file_str = String::from_utf8(token_file_line).expect("utf8");
        assert!(
            token_file_str.starts_with("FKST_GITHUB_TOKEN_FILE="),
            "got: {token_file_str}"
        );
        assert!(token_file_str.contains("github-token"));

        let pid = session.pid;
        runner.stop(&mut session).await.expect("stop");
        assert_eq!(fkst_entries(temp_root.path()), 0);
        assert_eq!(signal_group(pid, Signal::SIGTERM), Err(nix::Error::ESRCH));
    }

    #[tokio::test]
    async fn start_with_spec_empty_packages_is_rejected() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let spec = StartSpec {
            packages: vec![],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let err = runner.start_with_spec(&spec).await.expect_err("must fail");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert_eq!(fkst_entries(temp_root.path()), 0);
    }

    #[tokio::test]
    async fn start_with_spec_invalid_second_package_is_rejected_before_dirs() {
        let stub_dir = tempfile::tempdir().expect("stub dir");
        let temp_root = tempfile::tempdir().expect("temp root");
        let bin = engine_stub(stub_dir.path(), READY_SUPERVISE);
        let runner = SessionRunner::new(config(&bin, temp_root.path()));

        let mut bad_second = second_package();
        bad_second.files.clear(); // invalid

        let spec = StartSpec {
            packages: vec![minimal_package(), bad_second],
            goal: None,
            env_profile: BTreeMap::new(),
            codex_home: None,
            project_root: None,
            package_roots: Vec::new(),
        };
        let err = runner.start_with_spec(&spec).await.expect_err("must fail");
        assert!(matches!(err, RunnerError::InvalidPackage(_)));
        assert_eq!(
            fkst_entries(temp_root.path()),
            0,
            "no dirs on validation fail"
        );
    }
}
