//! Offline unit suite for [`super::execute_dispatch`] (issue #151, increment 4).
//!
//! Kept in its own file (included via `#[path]` from `executor.rs`) so the
//! executor source stays under the 500-line budget. The fake [`Cloner`]
//! materializes the package tree on disk, so every step EXCEPT the real
//! `git clone` runs without a network: codex-home render, ornn ZipB64 install,
//! GoalContext, the 0600 `.mint-nonce` overwrite, the StartSpec, and a real
//! `start_with_spec` driving a fake `sh` `fkst-framework` stub. No secret value
//! is ever asserted or printed.

use std::collections::BTreeMap;
use std::io::Write;
use std::path::PathBuf;

use async_trait::async_trait;
use fkst_shared::models::RepoRef;
use fkst_shared::protocol::{
    CloneSpec, DispatchGoal, OrnnPlan, OrnnSkillRef, OrnnSource, ResolvedDispatch,
};
use secrecy::SecretString;

use super::*;

/// A fake cloner: materializes a `<root>/.fkst/packages/<name>` tree (a minimal
/// runnable package the engine stub's conformance branch accepts) on a `TempDir`
/// and returns it as the handle's drop-guard, so the rest of `execute_dispatch`
/// runs offline. The `TempDir` IS the guard, so the offline test also proves the
/// clone-dir cleanup path.
struct FakeCloner;

#[async_trait]
impl Cloner for FakeCloner {
    async fn clone_packages(
        &self,
        base: &Path,
        _repo: &RepoRef,
        _token: &SecretString,
        package_names: &[String],
        _framework_bin: &Path,
    ) -> Result<ClonedHandle, RunnerError> {
        let guard = tempfile::Builder::new()
            .prefix("fake-clone-")
            .tempdir_in(base)
            .map_err(RunnerError::Io)?;
        let packages_root = guard.path().join(".fkst/packages");
        let mut roots = Vec::new();
        for name in package_names {
            let dir = packages_root.join(name);
            std::fs::create_dir_all(dir.join("departments/hello")).map_err(RunnerError::Io)?;
            std::fs::write(
                dir.join("departments/hello/main.lua"),
                "local M = {}\nM.spec = { consumes = { \"tick\" } }\n\
                 function pipeline(event) end\nreturn M\n",
            )
            .map_err(RunnerError::Io)?;
            std::fs::create_dir_all(dir.join("raisers")).map_err(RunnerError::Io)?;
            std::fs::write(
                dir.join("raisers/tick.lua"),
                "return { type = \"cron\", interval = \"1s\", produces = \"tick\" }\n",
            )
            .map_err(RunnerError::Io)?;
            roots.push(dir.canonicalize().map_err(RunnerError::Io)?);
        }
        let project_root = guard.path().canonicalize().map_err(RunnerError::Io)?;
        Ok(ClonedHandle::new(project_root, roots, Box::new(guard)))
    }
}

/// Write a fake `fkst-framework` shell binary: its `conformance` branch passes
/// and its `supervise` branch emits the real ready markers then sleeps — the
/// same stub pattern the engine's own runner tests use.
fn engine_stub(dir: &Path) -> PathBuf {
    let path = dir.join("stub-framework.sh");
    let script = r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0
    ;;
  supervise)
    echo "event runtime running handles=3"
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
    sleep 30
    ;;
esac
"#;
    std::fs::write(&path, script).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

fn config(bin: &Path, temp_root: &Path) -> EngineConfig {
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

/// A tiny in-memory zip carrying a single `SKILL.md`, for the ZipB64 ornn source
/// (no network in the test).
fn one_file_zip() -> Vec<u8> {
    let mut buf = Vec::new();
    {
        let cursor = std::io::Cursor::new(&mut buf);
        let mut writer = zip::ZipWriter::new(cursor);
        let opts: zip::write::FileOptions<()> =
            zip::write::FileOptions::default().unix_permissions(0o644);
        writer.start_file("SKILL.md", opts).unwrap();
        writer.write_all(b"# Dispatched skill").unwrap();
        writer.finish().unwrap();
    }
    buf
}

fn dispatch_with(env: BTreeMap<String, SecretString>, ornn: Option<OrnnPlan>) -> ResolvedDispatch {
    ResolvedDispatch {
        session_id: "11111111-1111-1111-1111-111111111111".into(),
        worker_id: "worker-0".into(),
        fencing_id: 1,
        goal: DispatchGoal {
            goal_id: "22222222-2222-2222-2222-222222222222".into(),
            title: "Build the thing".into(),
            description: SecretString::from("SECRET-PROMPT".to_string()),
            repo: RepoRef {
                owner: "acme".into(),
                name: "site".into(),
            },
        },
        clone_spec: CloneSpec {
            repo: RepoRef {
                owner: "acme".into(),
                name: "site".into(),
            },
            git_ref: "main".into(),
            package_roots: vec!["demo".into()],
        },
        github_token: SecretString::from("ghs_test_token".to_string()),
        github_token_expires_at_unix_ms: 1_700_000_000_000,
        env_profile: env,
        codex_config_toml: Some("[model_providers.chrono]\nname = \"chrono\"\n".into()),
        ornn,
        mint_nonce: SecretString::from("controller-nonce-abc".to_string()),
    }
}

/// End-to-end (offline): the fake cloner + the engine stub drive every step of
/// `execute_dispatch`. Asserts the start-written files, the overwritten 0600
/// nonce, the rendered config.toml, the installed ornn skill, and that a
/// RunningSession is returned. No secret value is ever asserted.
#[tokio::test]
async fn execute_dispatch_spawns_engine_and_writes_session_files() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let mut env = BTreeMap::new();
    env.insert("MY_VAR".to_string(), SecretString::from("v".to_string()));
    let plan = OrnnPlan {
        agents_md_appends: vec!["Use the dispatched skill.".into()],
        skills: vec![OrnnSkillRef {
            name: "demo-skill".into(),
            // Inline ZipB64 so the test never hits the network.
            source: OrnnSource::ZipB64(
                base64::engine::general_purpose::STANDARD.encode(one_file_zip()),
            ),
        }],
    };
    let dispatch = dispatch_with(env, Some(plan));

    let mut session = execute_dispatch_with(&cfg, &dispatch, &http, &FakeCloner)
        .await
        .expect("dispatch executes");

    let runtime_dir = session.running.runtime_dir.clone();

    // start_with_spec wrote the token file + goal.json under the runtime dir.
    assert!(
        runtime_dir.join("github-token").is_file(),
        "token file must exist"
    );
    let goal_json = std::fs::read_to_string(runtime_dir.join("goal.json")).expect("goal.json");
    // goal.json carries the non-secret identity (never the token); assert
    // structure, not that any secret leaked.
    assert!(goal_json.contains("22222222-2222-2222-2222-222222222222"));
    assert!(goal_json.contains("Build the thing"));

    // The controller's nonce overwrote the runner's, at mode 0600.
    let nonce_path = runtime_dir.join(".mint-nonce");
    assert_eq!(
        std::fs::read_to_string(&nonce_path).expect("nonce"),
        "controller-nonce-abc"
    );
    let nonce_mode = std::fs::metadata(&nonce_path)
        .expect("nonce meta")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(nonce_mode, 0o600, "nonce must be 0600");

    // A genuinely running session was returned.
    assert_eq!(
        session.running.status(),
        fkst_engine::LiveStatus::Running,
        "engine must be live"
    );
    assert!(session.running.pid > 0);

    // The CODEX_HOME guard exists, config.toml was rendered, the ornn skill
    // installed, and the AGENTS.md append landed.
    let codex_dir = session._codex_home.as_ref().expect("codex home").path();
    assert!(
        codex_dir.join("config.toml").is_file(),
        "config.toml present"
    );
    let cfg_mode = std::fs::metadata(codex_dir.join("config.toml"))
        .unwrap()
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(cfg_mode, 0o600, "config.toml must be 0600");
    assert!(
        codex_dir.join("skills/demo-skill/SKILL.md").is_file(),
        "ornn skill installed"
    );
    let agents = std::fs::read_to_string(codex_dir.join("AGENTS.md")).expect("AGENTS.md");
    assert!(agents.contains("Use the dispatched skill."));

    // Stop cleanly so the test never leaks a live engine.
    let runner = SessionRunner::new(cfg.clone());
    runner.stop(&mut session.running).await.expect("stop");
}

/// No codex config and no ornn => no CODEX_HOME is rendered (the spec's
/// `codex_home` stays `None`), and the StartSpec still carries the resolved
/// project root + the env profile.
#[tokio::test]
async fn execute_dispatch_without_codex_or_ornn_skips_codex_home() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let mut dispatch = dispatch_with(BTreeMap::new(), None);
    dispatch.codex_config_toml = None;

    let mut session = execute_dispatch_with(&cfg, &dispatch, &http, &FakeCloner)
        .await
        .expect("dispatch executes");

    assert!(
        session._codex_home.is_none(),
        "no codex home without config or ornn"
    );
    assert_eq!(session.running.status(), fkst_engine::LiveStatus::Running);

    let runner = SessionRunner::new(cfg.clone());
    runner.stop(&mut session.running).await.expect("stop");
}

/// A malformed goal id is rejected at the boundary with `InvalidDispatch`, and
/// no engine is spawned.
#[tokio::test]
async fn execute_dispatch_rejects_a_bad_goal_id() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let mut dispatch = dispatch_with(BTreeMap::new(), None);
    dispatch.goal.goal_id = "not-a-uuid".into();

    let err = execute_dispatch_with(&cfg, &dispatch, &http, &FakeCloner)
        .await
        .expect_err("bad goal id must fail");
    assert!(matches!(err, ExecError::InvalidDispatch(_)), "got {err:?}");
}

/// A malformed inline ZipB64 ornn skill is rejected as `InvalidDispatch` (the
/// boundary validation), and no engine is spawned.
#[tokio::test]
async fn execute_dispatch_rejects_a_bad_base64_skill() {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let bin = engine_stub(stub_dir.path());
    let cfg = config(&bin, temp_root.path());
    let http = reqwest::Client::new();

    let plan = OrnnPlan {
        agents_md_appends: Vec::new(),
        skills: vec![OrnnSkillRef {
            name: "bad".into(),
            source: OrnnSource::ZipB64("@@not base64@@".into()),
        }],
    };
    let dispatch = dispatch_with(BTreeMap::new(), Some(plan));

    let err = execute_dispatch_with(&cfg, &dispatch, &http, &FakeCloner)
        .await
        .expect_err("bad base64 must fail");
    assert!(matches!(err, ExecError::InvalidDispatch(_)), "got {err:?}");
}
