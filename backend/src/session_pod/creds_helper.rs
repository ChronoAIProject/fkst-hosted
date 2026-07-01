//! The in-pod git credential-helper wiring for a substrate session (Model B,
//! issue #359 §5.2).
//!
//! Plain `git push`/`clone` over HTTPS does not read `GITHUB_TOKEN`; it consults a
//! credential helper. The `run-substrate` driver materializes the [embedded
//! script](CREDENTIAL_HELPER_SCRIPT) into a writable runtime dir (the creds mount
//! is read-only 0400) and points `git` at it via the `GIT_CONFIG_*` env
//! ([`git_config_entries`]). The helper reads the mounted, control-plane-rotated
//! `{token, expires_at}` JSON token file on every op and answers
//! `username=x-access-token` / `password=<token>`, so a token rotation is picked
//! up with no process restart.
//!
//! Relocated verbatim (minus the deleted Model-A just-in-time mint-nonce path)
//! out of `engine::{goal_token, materialize}` so the session pod keeps its only
//! caller after `engine/` is removed.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// File name of the materialized credential-helper script.
pub const HELPER_SCRIPT_NAME: &str = "git-credential-fkst";

/// Owner-only rwx for the executable credential-helper script. It holds no
/// secret — it reads the rotatable `0400`/`0600` token file at credential time —
/// so `0700` is safe.
const HELPER_SCRIPT_MODE: u32 = 0o700;

/// The git credential-helper script source. Materialized verbatim at
/// `<runtime_dir>/git-credential-fkst` (mode `0700`).
///
/// Contract (validated against git's credential-helper protocol):
/// - git invokes it as `<script> get|store|erase`; only `get` emits output, the
///   others (and any unknown op) exit 0 silently.
/// - stdin for `get` is `key=value` lines terminated by a blank line; we ignore
///   the request fields (the per-host config already scopes us to github.com).
/// - on `get` it prints EXACTLY `username=x-access-token` and `password=<token>`
///   to stdout; ALL diagnostics go to stderr (stdout is parsed by git).
/// - JSON is parsed with anchored POSIX `grep`/`sed` (no `jq`/`python3`
///   dependency): the token charset (`ghs_` + word chars) and the RFC3339 stamp
///   contain no quotes, so a non-greedy `"key":"<chars>"` extraction is safe.
pub const CREDENTIAL_HELPER_SCRIPT: &str = include_str!("git-credential-fkst.sh");

/// One `GIT_CONFIG` key/value entry to inject on the substrate child.
pub struct GitConfigEntry {
    pub key: String,
    pub value: String,
}

/// Build the platform-set `git config` entries that wire the substrate session's
/// `git` to the credential helper WITHOUT touching any on-disk `.git/config` or
/// embedding the token in a remote URL (leak surface). Returned as ordered
/// key/value pairs; the caller renders them into
/// `GIT_CONFIG_COUNT`/`GIT_CONFIG_KEY_i`/`GIT_CONFIG_VALUE_i` on the child env.
///
/// - `credential.https://github.com.helper = !<abs helper path>` — git's
///   shell-exec helper form (git appends the `get`/`store`/`erase` operation).
/// - `credential.https://github.com.useHttpPath = false` — one credential serves
///   every path under the host (so the token is not path-scoped).
/// - `url.https://github.com/.insteadOf = git@github.com:` — coerce scp-style SSH
///   remotes to HTTPS so the helper (HTTPS-only) applies to them too.
pub fn git_config_entries(helper_path: &Path) -> Vec<GitConfigEntry> {
    let helper = helper_path.display();
    vec![
        GitConfigEntry {
            key: "credential.https://github.com.helper".to_string(),
            value: format!("!{helper}"),
        },
        GitConfigEntry {
            key: "credential.https://github.com.useHttpPath".to_string(),
            value: "false".to_string(),
        },
        GitConfigEntry {
            key: "url.https://github.com/.insteadOf".to_string(),
            value: "git@github.com:".to_string(),
        },
    ]
}

/// Materialize the credential-helper script into `dir`
/// (`<dir>/git-credential-fkst`, mode `0700`) and return its canonical absolute
/// path. The script holds no secret — it reads the rotatable token file at
/// credential time — so it is safe at `0700`. The path is canonicalized so the
/// `GIT_CONFIG` helper entry is the absolute path git executes.
pub fn materialize_helper_script(dir: &Path) -> Result<PathBuf, std::io::Error> {
    let path = dir.join(HELPER_SCRIPT_NAME);
    std::fs::write(&path, CREDENTIAL_HELPER_SCRIPT.as_bytes())?;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(HELPER_SCRIPT_MODE))?;
    let canonical = path.canonicalize()?;
    tracing::debug!(
        path = %canonical.display(),
        "run-substrate: materialized git credential helper"
    );
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    use std::process::{Command, Stdio};

    use super::*;

    /// Write a `{token, expires_at}` JSON token file the slim helper parses.
    fn write_token_json(path: &Path, token: &str) {
        std::fs::write(
            path,
            format!("{{\"token\":\"{token}\",\"expires_at\":\"2999-01-01T00:00:00Z\"}}"),
        )
        .expect("write token json");
    }

    /// Invoke the helper exactly as git does: `<helper> <op>` with the git request
    /// block on stdin and the token-file path in `FKST_GITHUB_TOKEN_FILE`. Returns
    /// (stdout, exit_ok).
    fn run_helper(helper: &Path, op: &str, token_path: &Path) -> (String, bool) {
        let mut child = Command::new("/bin/sh")
            .arg(helper)
            .arg(op)
            .env("FKST_GITHUB_TOKEN_FILE", token_path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn helper");
        // git's request block: key=value lines, blank-line terminated. A dropped
        // request block is harmless (the helper reads the token from the file, not
        // stdin), so a BrokenPipe on the silent-no-op ops is tolerated.
        let _ = child
            .stdin
            .take()
            .expect("stdin")
            .write_all(b"protocol=https\nhost=github.com\n\n");
        let out = child.wait_with_output().expect("wait helper");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            out.status.success(),
        )
    }

    #[test]
    fn git_config_entries_wire_helper_host_scope_and_insteadof() {
        let helper = PathBuf::from("/run/session/git-credential-fkst");
        let entries = git_config_entries(&helper);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "credential.https://github.com.helper");
        assert_eq!(entries[0].value, "!/run/session/git-credential-fkst");
        assert_eq!(entries[1].key, "credential.https://github.com.useHttpPath");
        assert_eq!(entries[1].value, "false");
        assert_eq!(entries[2].key, "url.https://github.com/.insteadOf");
        assert_eq!(entries[2].value, "git@github.com:");
    }

    #[test]
    fn helper_is_materialized_executable_absolute_and_posix_sh() {
        let dir = tempfile::tempdir().expect("dir");
        let path = materialize_helper_script(dir.path()).expect("materialize helper");
        assert!(path.is_absolute(), "helper path must be absolute");
        assert!(path.ends_with(HELPER_SCRIPT_NAME));
        let mode = std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777;
        assert_eq!(mode, HELPER_SCRIPT_MODE, "helper must be 0700");
        assert!(CREDENTIAL_HELPER_SCRIPT.starts_with("#!/bin/sh"));
        assert!(CREDENTIAL_HELPER_SCRIPT.contains("username=x-access-token"));
        assert!(CREDENTIAL_HELPER_SCRIPT.contains("password="));
    }

    #[test]
    fn helper_get_answers_x_access_token_and_the_file_token() {
        let dir = tempfile::tempdir().expect("dir");
        let helper = materialize_helper_script(dir.path()).expect("helper");
        let token_path = dir.path().join("github-token");
        write_token_json(&token_path, "ghs_file_token_aaa");

        let (stdout, ok) = run_helper(&helper, "get", &token_path);
        assert!(ok, "helper must exit 0");
        assert!(stdout.contains("username=x-access-token"), "got:\n{stdout}");
        assert!(
            stdout.contains("password=ghs_file_token_aaa"),
            "must serve the file token, got:\n{stdout}"
        );
        // ONLY the two credential lines on stdout.
        let lines: Vec<&str> = stdout.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 2, "exactly two lines on stdout: {lines:?}");
    }

    #[test]
    fn helper_store_and_erase_are_silent_noops() {
        let dir = tempfile::tempdir().expect("dir");
        let helper = materialize_helper_script(dir.path()).expect("helper");
        let token_path = dir.path().join("github-token");
        write_token_json(&token_path, "ghs_x");
        for op in ["store", "erase", "unknown-op"] {
            let (stdout, ok) = run_helper(&helper, op, &token_path);
            assert!(ok, "{op} must exit 0");
            assert!(
                stdout.trim().is_empty(),
                "{op} must emit nothing: {stdout:?}"
            );
        }
    }

    #[test]
    fn helper_with_unreadable_token_file_exits_clean_with_no_output() {
        let dir = tempfile::tempdir().expect("dir");
        let helper = materialize_helper_script(dir.path()).expect("helper");
        let missing = dir.path().join("does-not-exist");
        let (stdout, ok) = run_helper(&helper, "get", &missing);
        assert!(ok, "missing token file must not fail git hard (exit 0)");
        assert!(
            stdout.trim().is_empty(),
            "no blank password may be emitted: {stdout:?}"
        );
    }

    #[test]
    fn rotation_serves_the_new_token_without_respawn() {
        // The helper re-reads the token file on every invocation, so rewriting the
        // file in place (the rotation the control plane performs past the ~1h TTL)
        // makes the NEXT credential answer carry the new token — no restart.
        let dir = tempfile::tempdir().expect("dir");
        let helper = materialize_helper_script(dir.path()).expect("helper");
        let token_path = dir.path().join("github-token");
        write_token_json(&token_path, "ghs_v1_token");

        let (out1, _) = run_helper(&helper, "get", &token_path);
        assert!(out1.contains("password=ghs_v1_token"), "v1:\n{out1}");

        write_token_json(&token_path, "ghs_v2_token");
        let (out2, _) = run_helper(&helper, "get", &token_path);
        assert!(
            out2.contains("password=ghs_v2_token") && !out2.contains("ghs_v1_token"),
            "after rotation the helper must serve the NEW token:\n{out2}"
        );
    }
}
