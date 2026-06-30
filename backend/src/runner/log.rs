//! In-repo session logging (issue #291).
//!
//! The run-session pod writes its session log to `.fkst/log/<run_key>.log` in
//! the session's own cloned repo and commits checkpoints to a DEDICATED branch
//! `fkst/session-<session_id>`. Commits are built with git PLUMBING (a throwaway
//! index, `hash-object` / `write-tree` / `commit-tree` / `update-ref`) so the
//! engine's working tree is NEVER staged and the branch holds ONLY the log —
//! the eventual code PR stays clean.
//!
//! Logging is best-effort: every failure here is surfaced as a warning by the
//! caller and NEVER changes the session disposition (that is the engine exit
//! code alone).

use std::path::{Path, PathBuf};
use std::process::Stdio;

use secrecy::SecretString;
use tempfile::TempDir;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::engine::goal_token::{git_config_entries, write_token_file};
use crate::engine::materialize::materialize_helper_script;

/// Longest single log line kept; longer engine output is truncated with a
/// marker so one pathological line cannot blow up the committed blob.
const MAX_LINE_BYTES: usize = 8 * 1024;
/// Identity stamped on the log commits (never a human / AI author).
const COMMIT_NAME: &str = "fkst session log";
const COMMIT_EMAIL: &str = "noreply@chrono-ai.fun";

/// A typed empty env slice (disambiguates the generic git helpers).
const NO_ENV: &[(&str, &str)] = &[];

/// Errors from a log checkpoint. All are non-fatal at the call site.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// A `git` plumbing command exited non-zero.
    #[error("git {op} failed: {detail}")]
    Git { op: String, detail: String },
    /// An I/O error running git or writing the throwaway index.
    #[error("log io: {0}")]
    Io(#[from] std::io::Error),
}

/// Push credentials: the 0600 token file + the credential-helper script, wired
/// to git via `GIT_CONFIG_*` exactly as `engine::clone` does for the clone.
/// Absent for a local-remote push (tests) that needs no authentication.
struct PushCred {
    _dir: TempDir,
    token_file: PathBuf,
    helper: PathBuf,
}

/// Accumulates a session's log and checkpoints it to the dedicated branch.
pub struct SessionLog {
    /// The cloned repo working-tree root (where `git -C` runs).
    root: PathBuf,
    /// The dedicated branch name, `fkst/session-<session_id>`.
    branch: String,
    /// The in-repo log path, `.fkst/log/<run_key>.log`.
    log_path: String,
    /// The accumulated log bytes (the full file content each checkpoint).
    buf: Vec<u8>,
    /// Push auth; `None` for a credential-free (local) remote.
    cred: Option<PushCred>,
}

impl SessionLog {
    /// Build a logger for `root`. When `token` is `Some`, the push authenticates
    /// against `https://github.com` via the engine's credential helper (written
    /// under a fresh temp dir in `cred_base`); when `None`, the push runs with no
    /// credentials (a local/file remote).
    pub fn new(
        root: PathBuf,
        session_id: &str,
        run_key: &str,
        token: Option<SecretString>,
        cred_base: &Path,
    ) -> Result<Self, LogError> {
        let cred = match token {
            Some(token) => {
                let dir = tempfile::Builder::new()
                    .prefix("fkst-log-cred-")
                    .tempdir_in(cred_base)?;
                let token_file = dir.path().join("log-token");
                // The helper's JIT-refresh window never trips for a short push;
                // a +1h expiry mirrors the clone's one-shot token file.
                write_token_file(
                    &token_file,
                    &token,
                    std::time::SystemTime::now() + std::time::Duration::from_secs(3600),
                )
                .map_err(|e| LogError::Git {
                    op: "write-token".to_string(),
                    detail: e.to_string(),
                })?;
                let helper = materialize_helper_script(dir.path()).map_err(|e| LogError::Git {
                    op: "materialize-helper".to_string(),
                    detail: e.to_string(),
                })?;
                Some(PushCred {
                    _dir: dir,
                    token_file,
                    helper,
                })
            }
            None => None,
        };
        Ok(Self {
            root,
            branch: format!("fkst/session-{session_id}"),
            log_path: format!(".fkst/log/{run_key}.log"),
            buf: Vec::new(),
            cred,
        })
    }

    /// Append one raw engine stdout line (newline added), lossy-UTF-8 and capped.
    pub fn append_line(&mut self, line: &[u8]) {
        let trimmed: &[u8] = if line.len() > MAX_LINE_BYTES {
            &line[..MAX_LINE_BYTES]
        } else {
            line
        };
        let text = String::from_utf8_lossy(trimmed);
        let text = text.trim_end_matches(['\n', '\r']);
        self.buf.extend_from_slice(text.as_bytes());
        if line.len() > MAX_LINE_BYTES {
            self.buf.extend_from_slice(b" ...[truncated]");
        }
        self.buf.push(b'\n');
    }

    /// Append a non-engine lifecycle marker line.
    pub fn record(&mut self, msg: &str) {
        self.buf.extend_from_slice(b"## ");
        self.buf.extend_from_slice(msg.as_bytes());
        self.buf.push(b'\n');
    }

    /// Commit the current log to the dedicated branch and push it. Isolated from
    /// the working tree via a throwaway index + plumbing. A no-op when the buffer
    /// is empty.
    pub async fn checkpoint(&self) -> Result<(), LogError> {
        if self.buf.is_empty() {
            return Ok(());
        }
        let branch_ref = format!("refs/heads/{}", self.branch);

        // 1. Write the log bytes as a blob in the repo's object db (no working
        //    tree, no index touched).
        let blob = self
            .git_with_stdin(&["hash-object", "-w", "--stdin"], &self.buf, NO_ENV)
            .await?;
        let blob = String::from_utf8_lossy(&blob).trim().to_string();

        // 2. Stage ONLY the log path into a throwaway index, then snapshot it.
        let index = tempfile::Builder::new()
            .prefix("fkst-log-index-")
            .tempfile_in(&self.root)?;
        // git wants the index file to not pre-exist as a non-index; remove the
        // empty tempfile so update-index creates a fresh index there.
        let index_path = index.path().to_path_buf();
        drop(index);
        let index_env = [("GIT_INDEX_FILE", index_path.as_os_str())];
        self.git(
            &[
                "update-index",
                "--add",
                "--cacheinfo",
                &format!("100644,{blob},{}", self.log_path),
            ],
            &index_env,
        )
        .await?;
        let tree = self.git(&["write-tree"], &index_env).await?;
        let tree = String::from_utf8_lossy(&tree).trim().to_string();
        let _ = std::fs::remove_file(&index_path);

        // 3. Commit the tree onto the branch tip (root commit on first checkpoint).
        let parent = self.current_branch_tip(&branch_ref).await;
        let mut commit_args = vec!["commit-tree", &tree];
        if let Some(parent) = &parent {
            commit_args.push("-p");
            commit_args.push(parent);
        }
        let msg = "fkst: session log checkpoint";
        commit_args.push("-m");
        commit_args.push(msg);
        let commit = self
            .git_with_stdin(&commit_args, &[], &Self::identity_env())
            .await?;
        let commit = String::from_utf8_lossy(&commit).trim().to_string();

        // 4. Move the branch ref and push it.
        self.git(&["update-ref", &branch_ref, &commit], NO_ENV)
            .await?;
        self.push(&branch_ref).await
    }

    /// Final checkpoint at session end.
    pub async fn finish(&self) -> Result<(), LogError> {
        self.checkpoint().await
    }

    /// The branch's current tip sha, or `None` when the branch does not exist yet.
    async fn current_branch_tip(&self, branch_ref: &str) -> Option<String> {
        let out = Command::new("git")
            .arg("-C")
            .arg(&self.root)
            .args(["rev-parse", "--verify", "--quiet", branch_ref])
            .stdin(Stdio::null())
            .output()
            .await
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let sha = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if sha.is_empty() {
            None
        } else {
            Some(sha)
        }
    }

    /// Push the branch to `origin`, authenticating via the credential helper when
    /// present (a local/file remote needs none).
    async fn push(&self, branch_ref: &str) -> Result<(), LogError> {
        let refspec = format!("{branch_ref}:{branch_ref}");
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.root)
            .args(["push", "origin", &refspec])
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if let Some(cred) = &self.cred {
            command.env("FKST_GITHUB_TOKEN_FILE", &cred.token_file);
            let entries = git_config_entries(&cred.helper);
            command.env("GIT_CONFIG_COUNT", entries.len().to_string());
            for (i, entry) in entries.iter().enumerate() {
                command.env(format!("GIT_CONFIG_KEY_{i}"), &entry.key);
                command.env(format!("GIT_CONFIG_VALUE_{i}"), &entry.value);
            }
        }
        let out = command.output().await?;
        if out.status.success() {
            Ok(())
        } else {
            Err(LogError::Git {
                op: "push".to_string(),
                detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            })
        }
    }

    /// Author + committer identity env for the log commits.
    fn identity_env() -> [(&'static str, &'static str); 4] {
        [
            ("GIT_AUTHOR_NAME", COMMIT_NAME),
            ("GIT_AUTHOR_EMAIL", COMMIT_EMAIL),
            ("GIT_COMMITTER_NAME", COMMIT_NAME),
            ("GIT_COMMITTER_EMAIL", COMMIT_EMAIL),
        ]
    }

    /// Run `git -C <root> <args>` with extra env, capturing stdout; error on a
    /// non-zero exit.
    async fn git<K, V>(&self, args: &[&str], env: &[(K, V)]) -> Result<Vec<u8>, LogError>
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        self.git_with_stdin(args, &[], env).await
    }

    /// As [`Self::git`] but feeds `stdin` to the command.
    async fn git_with_stdin<K, V>(
        &self,
        args: &[&str],
        stdin: &[u8],
        env: &[(K, V)],
    ) -> Result<Vec<u8>, LogError>
    where
        K: AsRef<std::ffi::OsStr>,
        V: AsRef<std::ffi::OsStr>,
    {
        let mut command = Command::new("git");
        command
            .arg("-C")
            .arg(&self.root)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for (k, v) in env {
            command.env(k, v);
        }
        let mut child = command.spawn()?;
        if let Some(mut handle) = child.stdin.take() {
            handle.write_all(stdin).await?;
            handle.shutdown().await?;
        }
        let out = child.wait_with_output().await?;
        if out.status.success() {
            Ok(out.stdout)
        } else {
            Err(LogError::Git {
                op: args.first().copied().unwrap_or("git").to_string(),
                detail: String::from_utf8_lossy(&out.stderr).trim().to_string(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command as SyncCommand;

    /// Run a git command synchronously in `dir`, asserting success.
    fn git(dir: &Path, args: &[&str]) -> String {
        let out = SyncCommand::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .output()
            .expect("spawn git");
        assert!(
            out.status.success(),
            "git {args:?} failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_string()
    }

    /// A bare `origin` + a working clone with a committed `.fkst/packages/demo`
    /// tree on `main`, pushed to origin. Returns (tempdir, bare_path, work_path).
    fn fixture() -> (TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().expect("tmp");
        let bare = tmp.path().join("origin.git");
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        SyncCommand::new("git")
            .args(["init", "--bare", "-b", "main"])
            .arg(&bare)
            .output()
            .unwrap();
        git(&work, &["init", "-b", "main"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "t"]);
        std::fs::create_dir_all(work.join(".fkst/packages/demo")).unwrap();
        std::fs::write(work.join(".fkst/packages/demo/main.lua"), "-- demo\n").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-m", "init"]);
        git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        git(&work, &["push", "origin", "main"]);
        (tmp, bare, work)
    }

    fn log_for(work: &Path, tmp: &Path) -> SessionLog {
        // token=None: local filesystem remote needs no credentials.
        SessionLog::new(work.to_path_buf(), "sess1", "runkey1", None, tmp).unwrap()
    }

    #[tokio::test]
    async fn checkpoint_lands_on_the_dedicated_branch_only() {
        let (tmp, bare, work) = fixture();
        let main_before = git(&bare, &["rev-parse", "main"]);

        let mut log = log_for(&work, tmp.path());
        log.record("session started");
        log.append_line(b"RAISED: hello");
        log.append_line(b"working...\n");
        log.checkpoint().await.expect("checkpoint");

        // The dedicated branch exists on the remote and carries the log file.
        let blob = git(&bare, &["show", "fkst/session-sess1:.fkst/log/runkey1.log"]);
        assert!(blob.contains("session started"));
        assert!(blob.contains("RAISED: hello"));
        assert!(blob.contains("working..."));

        // The default branch is untouched, and the dedicated branch tree holds
        // ONLY the log (the engine working tree was never staged).
        assert_eq!(git(&bare, &["rev-parse", "main"]), main_before);
        let tree = git(
            &bare,
            &["ls-tree", "-r", "--name-only", "fkst/session-sess1"],
        );
        assert_eq!(tree, ".fkst/log/runkey1.log");

        // The working tree itself has no staged/dirty state from the log commit.
        assert!(git(&work, &["status", "--porcelain"]).is_empty());
    }

    #[tokio::test]
    async fn second_checkpoint_appends_a_commit_to_the_branch() {
        let (tmp, bare, work) = fixture();
        let mut log = log_for(&work, tmp.path());
        log.append_line(b"first");
        log.checkpoint().await.expect("first checkpoint");
        log.append_line(b"second");
        log.checkpoint().await.expect("second checkpoint");

        let count = git(&bare, &["rev-list", "--count", "fkst/session-sess1"]);
        assert_eq!(count, "2", "two checkpoints => two commits on the branch");
        let blob = git(&bare, &["show", "fkst/session-sess1:.fkst/log/runkey1.log"]);
        assert!(blob.contains("first") && blob.contains("second"));
    }

    #[tokio::test]
    async fn empty_buffer_checkpoint_is_a_noop() {
        let (tmp, _bare, work) = fixture();
        let log = log_for(&work, tmp.path());
        log.checkpoint()
            .await
            .expect("empty checkpoint is Ok and pushes nothing");
    }

    #[tokio::test]
    async fn push_failure_is_a_nonfatal_error() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path().join("work");
        std::fs::create_dir_all(&work).unwrap();
        git(&work, &["init", "-b", "main"]);
        git(&work, &["config", "user.email", "t@example.com"]);
        git(&work, &["config", "user.name", "t"]);
        std::fs::write(work.join("f"), "x").unwrap();
        git(&work, &["add", "-A"]);
        git(&work, &["commit", "-m", "init"]);
        // origin points at a path that does not exist -> push fails, but the
        // local plumbing succeeds, so checkpoint returns Err (never panics).
        git(&work, &["remote", "add", "origin", "/nonexistent/repo.git"]);

        let mut log = log_for(&work, tmp.path());
        log.append_line(b"line");
        let result = log.checkpoint().await;
        assert!(result.is_err(), "a failed push surfaces as Err");
        // The branch ref WAS created locally even though the push failed.
        assert_eq!(
            git(
                &work,
                &["rev-list", "--count", "refs/heads/fkst/session-sess1"]
            ),
            "1"
        );
    }
}
