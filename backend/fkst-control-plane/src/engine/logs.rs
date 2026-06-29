//! Best-effort tail of the engine's per-event child logs.
//!
//! `supervise` writes child logs to
//! `<FKST_RUNTIME_ROOT>/logs/framework-child/<dept>-<secs>-<nanos>-<seq>.log`,
//! creating the directory tree itself (`mkdir -p` semantics, spike Q5), and
//! only AFTER the first dispatched event (spike Q9) — so an idle session has
//! ready markers but no child logs, and `None` here is a normal answer.

use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Byte bound on a single tail read: only the last 16 KiB of the newest log
/// file are ever read, so a huge log can never blow memory.
pub const TAIL_BYTE_CAP: u64 = 16 * 1024;

/// Tail the newest regular file under `<runtime_dir>/logs/framework-child/`.
///
/// Returns at most `max_lines` lines (and at most [`TAIL_BYTE_CAP`] bytes,
/// seek-bounded). Best-effort by contract: a missing/empty directory, no
/// regular files, or ANY read error yields `None`, never an `Err`.
pub fn tail_child_logs(runtime_dir: &Path, max_lines: usize) -> Option<String> {
    let dir = runtime_dir.join("logs").join("framework-child");
    let newest = newest_regular_file(&dir)?;
    tracing::debug!(file = %newest.display(), "session.logs: tail file selected");
    let text = read_tail_bytes(&newest)?;
    let lines: Vec<&str> = text.lines().collect();
    let start = lines.len().saturating_sub(max_lines);
    Some(lines[start..].join("\n"))
}

/// Most-recently-modified regular file directly under `dir`, if any.
fn newest_regular_file(dir: &Path) -> Option<PathBuf> {
    let entries = fs::read_dir(dir).ok()?;
    let mut newest: Option<(SystemTime, PathBuf)> = None;
    for entry in entries.flatten() {
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        if !metadata.is_file() {
            continue;
        }
        let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let newer = match &newest {
            Some((best, _)) => modified > *best,
            None => true,
        };
        if newer {
            newest = Some((modified, entry.path()));
        }
    }
    newest.map(|(_, path)| path)
}

/// Read at most the last [`TAIL_BYTE_CAP`] bytes of `path`, lossily decoded.
fn read_tail_bytes(path: &Path) -> Option<String> {
    let mut file = fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    if len > TAIL_BYTE_CAP {
        file.seek(SeekFrom::Start(len - TAIL_BYTE_CAP)).ok()?;
    }
    let mut bytes = Vec::new();
    file.take(TAIL_BYTE_CAP).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runtime_with_log_dir() -> (tempfile::TempDir, PathBuf) {
        let rt = tempfile::tempdir().expect("rt dir");
        let log_dir = rt.path().join("logs").join("framework-child");
        fs::create_dir_all(&log_dir).expect("log dir");
        (rt, log_dir)
    }

    #[test]
    fn missing_log_dir_yields_none() {
        let rt = tempfile::tempdir().expect("rt dir");
        assert_eq!(tail_child_logs(rt.path(), 200), None);
    }

    #[test]
    fn empty_log_dir_yields_none() {
        let (rt, _log_dir) = runtime_with_log_dir();
        assert_eq!(tail_child_logs(rt.path(), 200), None);
    }

    #[test]
    fn directory_only_entries_yield_none() {
        let (rt, log_dir) = runtime_with_log_dir();
        fs::create_dir(log_dir.join("a-subdir")).expect("subdir");
        assert_eq!(tail_child_logs(rt.path(), 200), None);
    }

    #[test]
    fn short_file_is_returned_in_full() {
        let (rt, log_dir) = runtime_with_log_dir();
        fs::write(
            log_dir.join("hello-1-2-0.log"),
            "CMD=/usr/local/bin/x\nLUA=main.lua\nline three",
        )
        .expect("write log");
        assert_eq!(
            tail_child_logs(rt.path(), 200).as_deref(),
            Some("CMD=/usr/local/bin/x\nLUA=main.lua\nline three")
        );
    }

    #[test]
    fn tail_is_capped_at_max_lines_keeping_the_newest() {
        let (rt, log_dir) = runtime_with_log_dir();
        let content: String = (0..50).map(|i| format!("line-{i:02}\n")).collect();
        fs::write(log_dir.join("hello-1-2-0.log"), content).expect("write log");
        let tail = tail_child_logs(rt.path(), 3).expect("tail");
        assert_eq!(tail, "line-47\nline-48\nline-49");
    }

    #[test]
    fn newest_file_by_mtime_wins() {
        let (rt, log_dir) = runtime_with_log_dir();
        let older = log_dir.join("hello-1-1-0.log");
        let newer = log_dir.join("hello-9-9-1.log");
        fs::write(&older, "old content").expect("write old");
        fs::write(&newer, "new content").expect("write new");
        // Deterministic ordering regardless of write timing.
        let now = SystemTime::now();
        fs::File::open(&older)
            .and_then(|f| f.set_modified(now - std::time::Duration::from_secs(60)))
            .expect("age older file");
        fs::File::open(&newer)
            .and_then(|f| f.set_modified(now))
            .expect("touch newer file");

        assert_eq!(
            tail_child_logs(rt.path(), 200).as_deref(),
            Some("new content")
        );
    }

    #[test]
    fn huge_file_read_is_seek_bounded_to_the_byte_cap() {
        let (rt, log_dir) = runtime_with_log_dir();
        // 64 KiB of lines; only the trailing 16 KiB may be read.
        let content: String = (0..8192).map(|i| format!("entry-{i:05}\n")).collect();
        assert!(content.len() as u64 > 4 * TAIL_BYTE_CAP / 2);
        fs::write(log_dir.join("big-1-1-0.log"), &content).expect("write big log");

        let tail = tail_child_logs(rt.path(), usize::MAX).expect("tail");
        assert!(
            tail.len() as u64 <= TAIL_BYTE_CAP,
            "got {} bytes",
            tail.len()
        );
        assert!(tail.ends_with("entry-08191"), "must keep the newest end");
        assert!(!tail.contains("entry-00000"), "oldest must be cut off");
    }

    #[test]
    fn unreadable_file_yields_none_not_err() {
        use std::os::unix::fs::PermissionsExt;
        let (rt, log_dir) = runtime_with_log_dir();
        let path = log_dir.join("secret-1-1-0.log");
        fs::write(&path, "cannot read me").expect("write log");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o000)).expect("chmod 000");

        // Non-root cannot open mode-000 files; best-effort contract => None.
        if fs::File::open(&path).is_ok() {
            return; // running as root bypasses permissions; nothing to assert
        }
        assert_eq!(tail_child_logs(rt.path(), 200), None);
    }

    #[test]
    fn zero_max_lines_yields_an_empty_tail() {
        let (rt, log_dir) = runtime_with_log_dir();
        fs::write(log_dir.join("hello-1-2-0.log"), "a\nb\n").expect("write log");
        assert_eq!(tail_child_logs(rt.path(), 0).as_deref(), Some(""));
    }

    /// Issue #109 log audit (defense in depth): the spawn/refresh paths only ever
    /// log the goal token-file PATH and a `has_goal_env` boolean — never the
    /// token or a struct carrying it. This nails down the load-bearing guarantee
    /// the whole hardening rests on: a `{:?}` of `GoalEnv` (the one struct that
    /// holds the live credential) redacts the token, so a stray debug-log here or
    /// in any spawn path can never surface it.
    #[test]
    fn goal_env_debug_never_leaks_the_token_to_logs() {
        use crate::engine::GoalEnv;
        use secrecy::SecretString;
        use std::path::PathBuf;

        let goal_env = GoalEnv {
            github_token: SecretString::from("ghs_logs_audit_secret".to_string()),
            github_token_file: PathBuf::from("/run/session/github-token"),
            goal_file: PathBuf::from("/run/session/goal.json"),
            helper_path: PathBuf::from("/run/session/git-credential-fkst"),
            mint_nonce: SecretString::from("nonce_audit_secret".to_string()),
        };
        let rendered = format!("{goal_env:?}");
        assert!(
            !rendered.contains("ghs_logs_audit_secret"),
            "no log/debug path may render the token: {rendered}"
        );
        assert!(
            !rendered.contains("nonce_audit_secret"),
            "no log/debug path may render the mint nonce: {rendered}"
        );
        assert!(
            rendered.contains("<redacted>"),
            "token must show <redacted>"
        );
        // The token-file path is the only token-related thing that may be logged.
        assert!(
            rendered.contains("/run/session/github-token"),
            "token-file path remains loggable for diagnostics: {rendered}"
        );
    }
}
