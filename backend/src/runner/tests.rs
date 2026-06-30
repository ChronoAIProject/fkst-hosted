//! Unit tests for the `run-session` orchestration core.
//!
//! Hermetic by construction: the network `git clone` is the only injected
//! dependency, swapped for [`FixtureRepoSource`] (a local repo tree), and the
//! engine is a stub `sh` script that emits the real readiness markers then exits
//! with a chosen code. Everything else — spec validation, the credential reads,
//! the codex render, the `StartSpec` build, and the supervise loop — runs for
//! real. Both the success and the failure dispositions are covered.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use secrecy::SecretString;
use tempfile::TempDir;

use super::*;
use crate::models::RepoRef;
use crate::session_spec::{derive_session_id, SessionGoal, SessionSpec};

/// A [`RepoSource`] that returns a pre-laid-out local fixture repo, with no
/// network. The fixture dir is owned by the test (held in its own `TempDir`), so
/// the guard here is a unit.
struct FixtureRepoSource {
    repo_dir: PathBuf,
}

#[async_trait::async_trait]
impl RepoSource for FixtureRepoSource {
    async fn prepare(
        &self,
        _repo: &RepoRef,
        _token: &SecretString,
        package_names: &[String],
        _base: &Path,
        _framework_bin: &Path,
    ) -> Result<PreparedRepo, RunnerError> {
        let project_root = self.repo_dir.canonicalize().map_err(RunnerError::Io)?;
        let mut package_roots = Vec::with_capacity(package_names.len());
        for name in package_names {
            let root = project_root
                .join(".fkst/packages")
                .join(name)
                .canonicalize()
                .map_err(RunnerError::Io)?;
            package_roots.push(root);
        }
        Ok(PreparedRepo {
            project_root,
            package_roots,
            _guard: Box::new(()),
        })
    }
}

/// Lay down a minimal valid repo tree with one package under `.fkst/packages/`.
fn fixture_repo() -> TempDir {
    let dir = tempfile::tempdir().expect("repo dir");
    let pkg = dir.path().join(".fkst/packages/demo/departments/demo");
    fs::create_dir_all(&pkg).expect("mkdir");
    fs::write(
        pkg.join("main.lua"),
        "local M = {}\nM.spec = { consumes = { \"tick\" } }\n\
         function pipeline(event) end\nreturn M\n",
    )
    .expect("write package file");
    dir
}

/// A stub engine binary: `conformance` passes; `supervise` runs `supervise_body`
/// (which emits the ready markers then exits with a chosen code). Mirrors the
/// `engine::runner` test stub convention so the real ready-wait reaches a
/// terminal status.
fn framework_stub(dir: &Path, supervise_body: &str) -> PathBuf {
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

/// Emit the readiness markers, stay alive briefly, then exit with `code`. The
/// 2 s lifetime leaves a wide window for the ready-wait to observe the markers
/// before exit, even under saturated test-suite load.
fn ready_then_exit(code: i32) -> String {
    format!(
        r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=demo reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 2
    exit {code}"#
    )
}

/// Build an `EngineConfig` pointing at `bin` with generous timeouts (the stub
/// spawns a real `sh` child whose markers can lag under load).
fn engine_config(bin: &Path, temp_root: &Path) -> EngineConfig {
    EngineConfig {
        framework_bin: bin.to_path_buf(),
        temp_root: temp_root.to_path_buf(),
        candidate_prefix: "candidate/".to_string(),
        candidate_from_sep: "::".to_string(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 30,
        ready_timeout_secs: 30,
        error_capture_bytes: 8192,
        log_tail_lines: 200,
        github_token_refresh_secs: 2400,
    }
}

/// A CODEX-free, Ornn-free SessionSpec for the one fixture package.
fn sample_spec() -> SessionSpec {
    SessionSpec {
        session_id: derive_session_id(1, "acme", "site", 1),
        run_key: "rk-test".into(),
        installation_id: 1,
        repo: RepoRef {
            owner: "acme".into(),
            name: "site".into(),
        },
        owner_login: "acme".into(),
        issue_number: 1,
        goal: SessionGoal {
            title: "Add a thing".into(),
            prompt: "Implement the thing.".into(),
        },
        package_names: vec!["demo".into()],
        ornn_pins: Vec::new(),
        log_branch: "fkst/session-x".into(),
    }
}

/// Write `spec` to a `session-spec.json` under a fresh dir; return (dir, path).
fn write_spec(spec: &SessionSpec) -> (TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("spec dir");
    let path = dir.path().join("session-spec.json");
    fs::write(&path, serde_json::to_vec(spec).expect("serialize spec")).expect("write spec");
    (dir, path)
}

/// A creds dir holding a non-empty `github-token`; return (dir, layout).
fn creds_with_github_token() -> (TempDir, CredsLayout) {
    let dir = tempfile::tempdir().expect("creds dir");
    fs::write(dir.path().join("github-token"), "ghs_test_token\n").expect("write token");
    let layout = CredsLayout::new(dir.path());
    (dir, layout)
}

#[tokio::test]
async fn clean_engine_completion_exits_success() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let repo = fixture_repo();
    let bin = framework_stub(stub_dir.path(), &ready_then_exit(0));

    let spec = sample_spec();
    let (_spec_dir, spec_path) = write_spec(&spec);
    let (_creds_dir, creds) = creds_with_github_token();
    let source = FixtureRepoSource {
        repo_dir: repo.path().to_path_buf(),
    };

    let code = run_session_with(
        &source,
        &spec_path,
        &creds,
        engine_config(&bin, temp_root.path()),
    )
    .await;

    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::SUCCESS));
}

#[tokio::test]
async fn engine_failure_exits_nonzero() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let repo = fixture_repo();
    // Goes ready, runs briefly, then crashes non-zero -> the runner must report
    // failure (never a clean exit).
    let bin = framework_stub(stub_dir.path(), &ready_then_exit(3));

    let spec = sample_spec();
    let (_spec_dir, spec_path) = write_spec(&spec);
    let (_creds_dir, creds) = creds_with_github_token();
    let source = FixtureRepoSource {
        repo_dir: repo.path().to_path_buf(),
    };

    let code = run_session_with(
        &source,
        &spec_path,
        &creds,
        engine_config(&bin, temp_root.path()),
    )
    .await;

    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
}

#[tokio::test]
async fn missing_github_token_exits_nonzero_without_starting() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let repo = fixture_repo();
    let bin = framework_stub(stub_dir.path(), &ready_then_exit(0));

    let spec = sample_spec();
    let (_spec_dir, spec_path) = write_spec(&spec);
    // A creds dir with NO github-token file.
    let creds_dir = tempfile::tempdir().expect("creds dir");
    let creds = CredsLayout::new(creds_dir.path());
    let source = FixtureRepoSource {
        repo_dir: repo.path().to_path_buf(),
    };

    let code = run_session_with(
        &source,
        &spec_path,
        &creds,
        engine_config(&bin, temp_root.path()),
    )
    .await;

    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
    // No engine ran: the temp root holds no fkst-* session dirs.
    let leaked = fs::read_dir(temp_root.path())
        .expect("read temp root")
        .flatten()
        .filter(|entry| entry.file_name().to_string_lossy().starts_with("fkst-"))
        .count();
    assert_eq!(
        leaked, 0,
        "no engine dirs created when the token is missing"
    );
}

#[tokio::test]
async fn invalid_spec_exits_nonzero() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let repo = fixture_repo();
    let bin = framework_stub(stub_dir.path(), &ready_then_exit(0));

    // A spec file that is not valid SessionSpec JSON.
    let spec_dir = tempfile::tempdir().expect("spec dir");
    let spec_path = spec_dir.path().join("session-spec.json");
    fs::write(&spec_path, b"{ not valid json").expect("write bad spec");
    let (_creds_dir, creds) = creds_with_github_token();
    let source = FixtureRepoSource {
        repo_dir: repo.path().to_path_buf(),
    };

    let code = run_session_with(
        &source,
        &spec_path,
        &creds,
        engine_config(&bin, temp_root.path()),
    )
    .await;

    assert_eq!(format!("{code:?}"), format!("{:?}", ExitCode::FAILURE));
}
