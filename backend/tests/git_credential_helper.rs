//! Issue #107: end-to-end tests for the git credential-delivery design.
//!
//! These exercise the REAL materialized credential-helper script (invoked via
//! `/bin/sh`, exactly as git invokes it) and the REAL `GIT_CONFIG_*` wiring
//! produced by `engine::git_config_entries`, against the REAL atomic JSON
//! token-file writer (`engine::write_token_file`). No Docker, no engine binary,
//! no network — they assert the credential contract directly.
//!
//! Coverage:
//!   (a) helper invoked as git would (stdin `protocol=https\nhost=github.com\n`)
//!       answers `username=x-access-token` + the file token;
//!   (b) JIT path: a near-expiry file makes the helper drop a nonce-bearing
//!       request; a stand-in "driver" services it (mints + atomic rewrite +
//!       deletes the request) and the helper then serves the NEW token;
//!   (c) env injection: `git_config_entries` produces the expected three keys
//!       and the helper-exec value is the absolute script path;
//!   (d) rotation without re-spawn: after the token file is rewritten, a fresh
//!       helper invocation serves the NEW token (the helper re-reads the file
//!       every time, so no process restart is needed);
//!   (e) `store`/`erase` are silent no-ops.

use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use fkst_control_plane::engine::goal_token::CREDENTIAL_HELPER_SCRIPT;
use fkst_control_plane::engine::{
    git_config_entries, materialize_helper_script, write_token_file, MINT_NONCE_ENV,
    TOKEN_FILE_NAME,
};
use secrecy::SecretString;

/// Materialize the helper into `dir` and write a token file with `token` /
/// `expires_at`. Returns (helper_path, token_path).
fn setup(dir: &Path, token: &str, expires_at: SystemTime) -> (PathBuf, PathBuf) {
    let helper = materialize_helper_script(dir).expect("materialize helper");
    let token_path = dir.join(TOKEN_FILE_NAME);
    write_token_file(
        &token_path,
        &SecretString::from(token.to_string()),
        expires_at,
    )
    .expect("write token file");
    (helper, token_path)
}

/// Invoke the helper exactly as git does: `<helper> <op>` with the git request
/// block on stdin, the token-file path in `FKST_GITHUB_TOKEN_FILE`, and an
/// optional mint nonce + tightened windows. Returns (stdout, exit_ok).
fn run_helper(
    helper: &Path,
    op: &str,
    token_path: &Path,
    nonce: Option<&str>,
    window_secs: Option<u64>,
    wait_secs: Option<u64>,
) -> (String, bool) {
    let mut cmd = Command::new("/bin/sh");
    cmd.arg(helper)
        .arg(op)
        .env("FKST_GITHUB_TOKEN_FILE", token_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    if let Some(n) = nonce {
        cmd.env(MINT_NONCE_ENV, n);
    }
    if let Some(w) = window_secs {
        cmd.env("FKST_GITHUB_MINT_WINDOW_SECS", w.to_string());
    }
    if let Some(w) = wait_secs {
        cmd.env("FKST_GITHUB_MINT_WAIT_SECS", w.to_string());
    }
    let mut child = cmd.spawn().expect("spawn helper");
    // git's request block: key=value lines, blank-line terminated. Tolerate a
    // BrokenPipe here: for `store`/`erase` the helper exits 0 immediately
    // WITHOUT draining stdin, so on a fast platform (Linux) the pipe is already
    // closed when we write (macOS happens to buffer it). The helper reads the
    // token from the file, never stdin, so a dropped request block is harmless;
    // the test asserts the helper's stdout + exit status, which is unaffected.
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
fn helper_get_answers_x_access_token_and_the_file_token() {
    let dir = tempfile::tempdir().expect("dir");
    let (helper, token_path) = setup(dir.path(), "ghs_file_token_aaa", far_future());

    let (stdout, ok) = run_helper(&helper, "get", &token_path, None, None, None);
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
    let (helper, token_path) = setup(dir.path(), "ghs_x", far_future());
    for op in ["store", "erase", "unknown-op"] {
        let (stdout, ok) = run_helper(&helper, op, &token_path, None, None, None);
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
    let (stdout, ok) = run_helper(&helper, "get", &missing, None, None, None);
    assert!(ok, "missing token file must not fail git hard (exit 0)");
    assert!(
        stdout.trim().is_empty(),
        "no blank password may be emitted: {stdout:?}"
    );
}

#[test]
fn jit_path_fetches_a_fresh_token_before_answering() {
    let dir = tempfile::tempdir().expect("dir");
    // A near-expiry token (well inside the helper's window): the helper must
    // request a JIT mint before answering.
    let (helper, token_path) = setup(dir.path(), "ghs_stale_token", soon());
    // Write the per-session nonce file (0600) the stand-in driver authenticates.
    // The nonce is a runtime-random per-session value (not a hard-coded secret),
    // so the test exercises an arbitrary nonce on every run.
    let nonce = rand::random::<u64>().to_string();
    fkst_control_plane::engine::goal_token::write_nonce_file(dir.path(), &nonce).expect("nonce");

    // Stand-in driver: in a thread, wait for the request file, validate the
    // nonce, rewrite the token file with a FRESH token, then delete the request
    // (the "ready" signal the helper waits on). This mirrors the driver poller.
    let request_path = {
        let mut p = token_path.clone().into_os_string();
        p.push(".request");
        PathBuf::from(p)
    };
    let token_path_for_driver = token_path.clone();
    let request_for_driver = request_path.clone();
    let nonce_for_driver = nonce.clone();
    let driver = std::thread::spawn(move || {
        for _ in 0..200 {
            if let Ok(contents) = std::fs::read_to_string(&request_for_driver) {
                assert_eq!(
                    contents.trim(),
                    nonce_for_driver.as_str(),
                    "nonce must match"
                );
                write_token_file(
                    &token_path_for_driver,
                    &SecretString::from("ghs_fresh_token".to_string()),
                    far_future(),
                )
                .expect("rewrite token");
                std::fs::remove_file(&request_for_driver).expect("delete request");
                return true;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        false
    });

    // Tight window so the near-expiry token triggers the JIT path; generous wait.
    let (stdout, ok) = run_helper(
        &helper,
        "get",
        &token_path,
        Some(nonce.as_str()),
        Some(3600),
        Some(10),
    );
    assert!(
        driver.join().expect("driver thread"),
        "driver must service the request"
    );
    assert!(ok);
    assert!(
        stdout.contains("password=ghs_fresh_token"),
        "helper must serve the JIT-refreshed token, not the stale one:\n{stdout}"
    );
}

#[test]
fn jit_path_falls_back_to_current_token_when_no_driver_services_it() {
    let dir = tempfile::tempdir().expect("dir");
    let (helper, token_path) = setup(dir.path(), "ghs_still_valid", soon());
    // Runtime-random per-session nonce (not a hard-coded secret).
    let nonce = rand::random::<u64>().to_string();
    fkst_control_plane::engine::goal_token::write_nonce_file(dir.path(), &nonce).expect("nonce");

    // No driver: the helper waits briefly, then falls back to the current token
    // rather than failing git hard.
    let (stdout, ok) = run_helper(
        &helper,
        "get",
        &token_path,
        Some(nonce.as_str()),
        Some(3600),
        Some(1), // 1s wait so the test is fast
    );
    assert!(ok);
    assert!(
        stdout.contains("password=ghs_still_valid"),
        "fallback to current token:\n{stdout}"
    );
}

#[test]
fn git_config_entries_are_the_expected_three_keys() {
    let dir = tempfile::tempdir().expect("dir");
    let helper = materialize_helper_script(dir.path()).expect("helper");
    let entries = git_config_entries(&helper);
    assert_eq!(entries.len(), 3);
    assert_eq!(entries[0].key, "credential.https://github.com.helper");
    assert_eq!(
        entries[0].value,
        format!("!{}", helper.display()),
        "helper-exec value must be the absolute script path"
    );
    assert_eq!(entries[1].key, "credential.https://github.com.useHttpPath");
    assert_eq!(entries[1].value, "false");
    assert_eq!(entries[2].key, "url.https://github.com/.insteadOf");
    assert_eq!(entries[2].value, "git@github.com:");
}

#[test]
fn rotation_serves_the_new_token_without_respawn() {
    // The "without re-spawn" guarantee at the file level: the helper re-reads
    // the token file on every invocation, so rewriting the file (the rotation a
    // long session performs past the 1h TTL) makes the NEXT credential answer
    // carry the new token — no engine/helper process restart involved.
    let dir = tempfile::tempdir().expect("dir");
    let (helper, token_path) = setup(dir.path(), "ghs_v1_token", far_future());

    let (out1, _) = run_helper(&helper, "get", &token_path, None, None, None);
    assert!(out1.contains("password=ghs_v1_token"), "v1:\n{out1}");

    // Rotate the file in place (simulated re-mint) — same path, atomic rewrite.
    write_token_file(
        &token_path,
        &SecretString::from("ghs_v2_token".to_string()),
        far_future(),
    )
    .expect("rotate");

    let (out2, _) = run_helper(&helper, "get", &token_path, None, None, None);
    assert!(
        out2.contains("password=ghs_v2_token") && !out2.contains("ghs_v1_token"),
        "after rotation the helper must serve the NEW token:\n{out2}"
    );
}

#[test]
fn embedded_helper_script_is_a_posix_sh_script() {
    assert!(CREDENTIAL_HELPER_SCRIPT.starts_with("#!/bin/sh"));
    // Materialized copy must be executable.
    let dir = tempfile::tempdir().expect("dir");
    let helper = materialize_helper_script(dir.path()).expect("helper");
    let mode = std::fs::metadata(&helper)
        .expect("meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700);
}

/// An expiry comfortably in the future (no JIT path).
fn far_future() -> SystemTime {
    SystemTime::now() + Duration::from_secs(3600)
}

/// An expiry inside the helper's default safety window (triggers the JIT path).
fn soon() -> SystemTime {
    SystemTime::now() + Duration::from_secs(60)
}
