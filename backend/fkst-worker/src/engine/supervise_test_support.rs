//! Shared scaffolding for the supervise-loop suite (issue #151, increment 5):
//! a fake engine (the `sh` `fkst-framework` stub), a fake controller (wiremock),
//! and the agent / refresh-state builders the named tests reuse. Split out of
//! `supervise_tests.rs` so each test file stays under the 500-line budget. No
//! secret value is ever asserted or printed by anything here.

use std::collections::BTreeMap;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use secrecy::SecretString;
use serde_json::json;
use wiremock::matchers::{method, path as wm_path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use fkst_engine::{EngineConfig, RunnerError, RunningSession};
use fkst_shared::models::RepoRef;
use fkst_shared::protocol::{CloneSpec, DispatchGoal, ResolvedDispatch, StatusReport};

use crate::agent::WorkerAgent;
use crate::engine::executor::{execute_dispatch_with, SessionGuards};
use crate::engine::refresh::{FenceConfirmer, RefreshState};
use crate::engine::{ClonedHandle, Cloner};

/// A fake cloner: materializes a minimal runnable package tree the engine stub's
/// conformance branch accepts, returning the `TempDir` as the drop-guard.
pub(crate) struct FakeCloner;

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

/// Write a fake `fkst-framework`: conformance passes; supervise emits the ready
/// markers then sleeps (so `status()` stays Running until stopped). When
/// `exit_after_secs > 0` the supervise branch exits cleanly after that delay so a
/// test can observe a terminal transition.
pub(crate) fn engine_stub(dir: &Path, exit_after_secs: u64) -> PathBuf {
    let path = dir.join("stub-framework.sh");
    let tail = if exit_after_secs == 0 {
        "sleep 30".to_string()
    } else {
        format!("sleep {exit_after_secs}\nexit 0")
    };
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  conformance)
    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0
    ;;
  supervise)
    echo "event runtime running handles=3"
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]"
    {tail}
    ;;
esac
"#
    );
    std::fs::write(&path, script).expect("write stub");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    path
}

pub(crate) fn stub_config(bin: &Path, temp_root: &Path) -> EngineConfig {
    EngineConfig {
        framework_bin: bin.to_path_buf(),
        temp_root: temp_root.to_path_buf(),
        stop_grace_secs: 2,
        conformance_timeout_secs: 30,
        ready_timeout_secs: 30,
        ..EngineConfig::default()
    }
}

pub(crate) fn dispatch() -> ResolvedDispatch {
    ResolvedDispatch {
        session_id: "11111111-1111-1111-1111-111111111111".into(),
        worker_id: "w1".into(),
        fencing_id: 7,
        goal: DispatchGoal {
            goal_id: "22222222-2222-2222-2222-222222222222".into(),
            title: "Build it".into(),
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
        github_token_expires_at_unix_ms: far_future_unix_ms(),
        env_profile: BTreeMap::new(),
        codex_config_toml: None,
        ornn: None,
        journal: None,
        mint_nonce: SecretString::from("controller-nonce-abc".to_string()),
    }
}

/// A token-expiry far in the future so the escalating cooldown uses the NORMAL
/// (60s) window — a test that wants a mint to fire must clear the cooldown
/// explicitly, never race the urgent path.
pub(crate) fn far_future_unix_ms() -> i64 {
    (SystemTime::now() + Duration::from_secs(3 * 3600))
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

pub(crate) fn agent_for(server: &MockServer) -> Arc<WorkerAgent> {
    Arc::new(WorkerAgent::new(
        server.uri(),
        SecretString::from("tok".to_string()),
        "w1".into(),
        4,
        "/tmp/e".into(),
    ))
}

/// Spawn the engine for `dispatch` via the offline executor (fake cloner +
/// stub). Returns the running engine + its guards + the runtime dir.
pub(crate) async fn spawn_engine(
    cfg: &EngineConfig,
    d: &ResolvedDispatch,
) -> (RunningSession, SessionGuards, PathBuf) {
    let http = reqwest::Client::new();
    let session = execute_dispatch_with(cfg, d, &http, &FakeCloner)
        .await
        .expect("dispatch executes");
    let runtime_dir = session.running.runtime_dir.clone();
    let (running, guards) = session.into_parts();
    (running, guards, runtime_dir)
}

/// Build the refresh state for a session, with an explicit confirmer.
pub(crate) fn refresh_state(
    agent: &Arc<WorkerAgent>,
    d: &ResolvedDispatch,
    runtime_dir: &Path,
    confirmer: Arc<dyn FenceConfirmer>,
) -> RefreshState {
    RefreshState::new(
        agent.clone(),
        d.session_id.clone(),
        format!("{}/{}", d.goal.repo.owner, d.goal.repo.name),
        runtime_dir.to_path_buf(),
        SystemTime::now() + Duration::from_secs(3 * 3600),
        confirmer,
    )
}

/// Mount a credential-refresh responder returning the given JSON body.
pub(crate) async fn mount_refresh(server: &MockServer, body: serde_json::Value) {
    Mock::given(method("POST"))
        .and(wm_path("/internal/v1/credential-refresh"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(server)
        .await;
}

/// Mount a status-report responder.
pub(crate) async fn mount_status(server: &MockServer) {
    Mock::given(method("POST"))
        .and(wm_path("/internal/v1/status-report"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({"applied": true})))
        .mount(server)
        .await;
}

/// Mount a released responder.
pub(crate) async fn mount_released(server: &MockServer) {
    Mock::given(method("POST"))
        .and(wm_path("/internal/v1/released"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({})))
        .mount(server)
        .await;
}

/// All status-report request bodies the server received, decoded.
pub(crate) async fn received_statuses(server: &MockServer) -> Vec<StatusReport> {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/internal/v1/status-report")
        .filter_map(|r| serde_json::from_slice(&r.body).ok())
        .collect()
}

/// Count of credential-refresh requests the server received.
pub(crate) async fn refresh_request_count(server: &MockServer) -> usize {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .filter(|r| r.url.path() == "/internal/v1/credential-refresh")
        .count()
}

/// Whether the server received any released request.
pub(crate) async fn saw_released(server: &MockServer) -> bool {
    server
        .received_requests()
        .await
        .unwrap_or_default()
        .into_iter()
        .any(|r| r.url.path() == "/internal/v1/released")
}
