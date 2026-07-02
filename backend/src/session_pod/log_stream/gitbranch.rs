//! The per-session log BRANCH: bootstrap a worktree for `fkst-logs/issue-<N>` and,
//! on each flush, commit the instance dir and push it.
//!
//! The git side is hidden behind the [`GitRunner`] trait so the bootstrap →
//! commit → push SEQUENCE is unit-testable with a fake (the real subprocess wiring
//! is validated on-cluster). The real runner reuses the pod's existing git identity
//! and credential helper (the same `GIT_CONFIG_*` plus token-file env the driver
//! wires for clones), so no new credential is introduced. A revived session (new
//! pod, same trigger) reuses the SAME branch and only ADDS a new
//! `instances/<INSTANCE>/` dir, never rewriting an earlier one.

use std::path::{Path, PathBuf};

use super::super::creds_helper::GitConfigEntry;

/// The result of one git invocation (success + captured output). `stdout`/`stderr`
/// are UTF-8-lossy; the caller redacts before logging (git argv never carries the
/// token — it lives in the mounted file the helper reads — but stderr is redacted
/// defensively all the same).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitOutcome {
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

impl GitOutcome {
    fn failure(stderr: impl Into<String>) -> Self {
        Self {
            ok: false,
            stdout: String::new(),
            stderr: stderr.into(),
        }
    }
}

/// Runs one `git` command in the log worktree. The single primitive the branch
/// orchestration is built on, so a fake can record the exact call sequence.
pub trait GitRunner {
    /// Run `git <args>` in the worktree and capture the outcome. Never panics — a
    /// spawn failure returns `ok = false`.
    fn run(&self, args: &[&str]) -> GitOutcome;
}

/// The production [`GitRunner`]: shells out to `git -C <worktree>` with the pod's
/// credential env (the helper + rotating token file) so pushes authenticate exactly
/// as the driver's clones do.
pub struct RealGitRunner {
    worktree: PathBuf,
    env: Vec<(String, String)>,
}

impl RealGitRunner {
    /// Build a runner bound to `worktree`, wiring the same `GIT_CONFIG_*` credential
    /// helper entries + the rotating token-file path the driver uses for clones, plus
    /// the non-interactive guard so a credential miss can never hang the pod.
    pub fn new(worktree: PathBuf, git_entries: &[GitConfigEntry], token_file: &Path) -> Self {
        let mut env = vec![
            ("GIT_TERMINAL_PROMPT".to_string(), "0".to_string()),
            (
                "FKST_GITHUB_TOKEN_FILE".to_string(),
                token_file.to_string_lossy().into_owned(),
            ),
            (
                "GIT_CONFIG_COUNT".to_string(),
                git_entries.len().to_string(),
            ),
        ];
        for (i, entry) in git_entries.iter().enumerate() {
            env.push((format!("GIT_CONFIG_KEY_{i}"), entry.key.clone()));
            env.push((format!("GIT_CONFIG_VALUE_{i}"), entry.value.clone()));
        }
        Self { worktree, env }
    }
}

impl GitRunner for RealGitRunner {
    fn run(&self, args: &[&str]) -> GitOutcome {
        let mut command = std::process::Command::new("git");
        command.arg("-C").arg(&self.worktree).args(args);
        for (key, value) in &self.env {
            command.env(key, value);
        }
        match command.output() {
            Ok(output) => GitOutcome {
                ok: output.status.success(),
                stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
            },
            Err(error) => GitOutcome::failure(error.to_string()),
        }
    }
}

/// Orchestrates the log branch over a [`GitRunner`]. Owns the worktree path, the
/// branch name, and the instance id; drives bootstrap + each flush.
pub struct LogBranch<R: GitRunner> {
    runner: R,
    worktree: PathBuf,
    branch: String,
    instance: String,
    bootstrapped: bool,
}

impl<R: GitRunner> LogBranch<R> {
    pub fn new(runner: R, worktree: PathBuf, branch: String, instance: String) -> Self {
        Self {
            runner,
            worktree,
            branch,
            instance,
            bootstrapped: false,
        }
    }

    /// Whether bootstrap has completed (a flush is a no-op until it has).
    pub fn is_bootstrapped(&self) -> bool {
        self.bootstrapped
    }

    /// The absolute instance dir the collector writes its redacted files into.
    pub fn instance_dir(&self) -> PathBuf {
        self.worktree.join("instances").join(&self.instance)
    }

    /// Bootstrap the worktree for the branch: init the repo, point `origin` at
    /// `remote_url`, then either fetch the existing branch (revival — old instances
    /// are preserved) or create it as an orphan with the `README.md`. Finally create
    /// this pod's `instances/<INSTANCE>/` dir and drop `meta.json` into it. Idempotent
    /// enough to retry on a later flush if the first attempt failed.
    pub fn bootstrap(
        &mut self,
        remote_url: &str,
        readme: &str,
        meta_json: &str,
    ) -> Result<(), String> {
        std::fs::create_dir_all(&self.worktree)
            .map_err(|e| format!("create log worktree {}: {e}", self.worktree.display()))?;

        if !self.worktree.join(".git").is_dir() {
            require(self.runner.run(&["init", "-q"]), "init")?;
        }
        // Point origin at the target repo (set-url when it already exists, else add).
        if !self
            .runner
            .run(&["remote", "set-url", "origin", remote_url])
            .ok
        {
            require(
                self.runner.run(&["remote", "add", "origin", remote_url]),
                "remote add",
            )?;
        }

        if self.remote_branch_exists() {
            require(
                self.runner
                    .run(&["fetch", "--depth=1", "origin", &self.branch]),
                "fetch",
            )?;
            require(
                self.runner
                    .run(&["checkout", "-B", &self.branch, "FETCH_HEAD"]),
                "checkout existing",
            )?;
        } else {
            require(
                self.runner.run(&["checkout", "--orphan", &self.branch]),
                "checkout orphan",
            )?;
            // A fresh orphan has an empty index; a defensive reset keeps it clean.
            let _ = self.runner.run(&["reset", "-q"]);
            std::fs::write(self.worktree.join("README.md"), readme)
                .map_err(|e| format!("write README.md: {e}"))?;
            require(self.runner.run(&["add", "README.md"]), "add README")?;
            require(
                self.runner.run(&[
                    "commit",
                    "-q",
                    "-m",
                    "chore(logs): initialize session log branch",
                ]),
                "commit README",
            )?;
        }

        let instance_dir = self.instance_dir();
        std::fs::create_dir_all(&instance_dir)
            .map_err(|e| format!("create instance dir {}: {e}", instance_dir.display()))?;
        std::fs::write(instance_dir.join("meta.json"), meta_json)
            .map_err(|e| format!("write meta.json: {e}"))?;

        self.bootstrapped = true;
        Ok(())
    }

    /// Stage this instance's dir, commit if there is anything new, and push. On a
    /// non-fast-forward (or transient) push, fetch + rebase this instance's commits
    /// on top and retry once — safe because only this pod writes this instance dir.
    /// Returns `Ok(true)` if a commit was pushed, `Ok(false)` if there was nothing to
    /// commit.
    pub fn flush(&mut self, seq: u64) -> Result<bool, String> {
        if !self.bootstrapped {
            return Err("log branch not bootstrapped".to_string());
        }
        let instance_path = format!("instances/{}", self.instance);
        require(self.runner.run(&["add", &instance_path]), "add instance")?;

        let status = self.runner.run(&["status", "--porcelain"]);
        if status.ok && status.stdout.trim().is_empty() {
            return Ok(false);
        }

        let message = format!("chore(logs): {} flush {}", self.instance, seq);
        require(
            self.runner.run(&["commit", "-q", "-m", &message]),
            "commit flush",
        )?;

        if self.push().ok {
            return Ok(true);
        }
        // Non-ff / transient: reconcile with the remote and retry once.
        let _ = self.runner.run(&["fetch", "origin", &self.branch]);
        let _ = self
            .runner
            .run(&["rebase", &format!("origin/{}", self.branch)]);
        let retry = self.push();
        if retry.ok {
            Ok(true)
        } else {
            Err(format!("push failed: {}", retry.stderr.trim()))
        }
    }

    /// Push the branch to origin, bounding a network hang via git's low-speed timeout
    /// so a stuck push never wedges pod shutdown.
    fn push(&self) -> GitOutcome {
        self.runner.run(&[
            "-c",
            "http.lowSpeedLimit=1000",
            "-c",
            "http.lowSpeedTime=20",
            "push",
            "origin",
            &self.branch,
        ])
    }

    /// Whether the branch already exists on the remote (a non-empty `ls-remote`).
    fn remote_branch_exists(&self) -> bool {
        let ls = self
            .runner
            .run(&["ls-remote", "--heads", "origin", &self.branch]);
        ls.ok && !ls.stdout.trim().is_empty()
    }
}

/// Turn a failed [`GitOutcome`] into an error naming the op; a success passes
/// through so a caller can chain with `?`.
fn require(outcome: GitOutcome, op: &str) -> Result<GitOutcome, String> {
    if outcome.ok {
        Ok(outcome)
    } else {
        Err(format!("git {op} failed: {}", outcome.stderr.trim()))
    }
}

#[cfg(test)]
#[path = "gitbranch_tests.rs"]
mod tests;
