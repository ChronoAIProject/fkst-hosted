//! `POST /api/v1/packages/generate` integration tests against an ephemeral
//! Mongo container (testcontainers), driven via `tower::ServiceExt::oneshot`
//! against the REAL `build_router(AppState)` — full middleware stack.
//!
//! The LLM is a `MockGateway` returning a scripted queue of completions and
//! recording the prompts it was called with (so the retry-prompt feedback can
//! be asserted). The engine is a stub `/bin/sh` script: exit 0 for a passing
//! conformance, `echo ... >&2; exit 1` for a failing one. One test
//! (`generate_real_engine_conformance_smoke`) runs against the real engine when
//! one is resolvable, else self-skips.
//!
//! Every test gets a fresh container and self-skips when Docker is unavailable.

#![allow(clippy::expect_used)]

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use fkst_hosted_api::auth::AuthMode;
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::llm::{LlmError, LlmGateway};
use fkst_hosted_api::packages::{PackageRepository, ShareRepo};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

mod support;
use support::require_engine;

/// True when a Docker daemon answers `docker info`.
fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`.
const MONGO_TAG: &str = "7";

// ---- MockGateway ------------------------------------------------------------

/// One scripted LLM completion: an `Ok(body)` or an `Err(LlmError)`.
type ScriptedReply = Result<String, LlmError>;

/// A scripted, recording `LlmGateway`. `replies` is consumed front-to-back;
/// `calls` records every `(system, user)` pair so retry feedback can be
/// asserted. Running past the script is a hard error (an unexpected extra call).
#[derive(Clone)]
struct MockGateway {
    replies: Arc<Mutex<std::collections::VecDeque<ScriptedReply>>>,
    calls: Arc<Mutex<Vec<(String, String)>>>,
}

impl MockGateway {
    fn new(replies: Vec<ScriptedReply>) -> Self {
        Self {
            replies: Arc::new(Mutex::new(replies.into_iter().collect())),
            calls: Arc::new(Mutex::new(Vec::new())),
        }
    }

    fn calls(&self) -> Vec<(String, String)> {
        self.calls.lock().expect("calls lock").clone()
    }
}

#[async_trait]
impl LlmGateway for MockGateway {
    async fn complete(&self, system: &str, user: &str) -> Result<String, LlmError> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((system.to_string(), user.to_string()));
        self.replies
            .lock()
            .expect("replies lock")
            .pop_front()
            .unwrap_or_else(|| Err(LlmError::Http("mock gateway script exhausted".to_string())))
    }
}

// ---- stub engine ------------------------------------------------------------

/// Conformance branch that passes (exit 0).
const PASS_CONFORMANCE: &str = r#"    echo "PASS graph-scan loaded 1 departments"
    exit 0"#;

/// Conformance branch that fails (exit 1) with a recognizable stderr line.
const FAIL_CONFORMANCE: &str = r#"    echo "CONFORMANCE-STDERR-MARKER department graph invalid" >&2
    exit 1"#;

/// Write a stub engine: `conformance` runs `conformance_body`; any other
/// subcommand is a no-op success (generation never spawns `supervise`).
fn write_stub(dir: &Path, conformance_body: &str) -> PathBuf {
    let path = dir.join("stub-framework.sh");
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  conformance)
{conformance_body}
    ;;
  *)
    exit 0
    ;;
esac
"#
    );
    fs::write(&path, script).expect("write stub");
    fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).expect("chmod stub");
    path
}

// ---- test harness -----------------------------------------------------------

/// Everything a test needs; container + temp dirs are kept alive.
struct TestApp {
    _container: ContainerAsync<Mongo>,
    _stub_dir: Option<tempfile::TempDir>,
    _temp_root: tempfile::TempDir,
    router: axum::Router,
}

/// Build the app with the given LLM gateway (or `None` for the unconfigured
/// case) and engine config.
async fn app_with(
    gateway: Option<Arc<dyn LlmGateway>>,
    engine: EngineConfig,
    stub_dir: Option<tempfile::TempDir>,
    temp_root: tempfile::TempDir,
) -> TestApp {
    let container = Mongo::default()
        .with_tag(MONGO_TAG)
        .start()
        .await
        .expect("start mongo");
    let host = container.get_host().await.expect("container host");
    let port = container
        .get_host_port_ipv4(27017)
        .await
        .expect("container port");
    let config = Config {
        mongodb_uri: format!("mongodb://{host}:{port}"),
        mongodb_server_selection_timeout_ms: 5000,
        ..Config::default()
    };
    let db = Db::connect(&config).await.expect("connect + ping");
    let packages = PackageRepository::new(&db.database);
    let shares = ShareRepo::new(&db.database);
    let goals = GoalRepo::new(&db.database);
    let sessions = SessionService::new(
        SessionRepo::new(&db),
        packages.clone(),
        EngineConfig::default(),
    );
    let vault = support::test_vault(&db);
    let router = build_router(AppState {
        config,
        db: db.clone(),
        packages,
        shares,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        goals,
        engine,
        llm: gateway,
        vault,
    })
    .expect("router");
    TestApp {
        _container: container,
        _stub_dir: stub_dir,
        _temp_root: temp_root,
        router,
    }
}

/// Build an app whose engine is the passing/failing stub and whose LLM is the
/// supplied mock.
async fn app_with_stub_engine(mock: MockGateway, conformance_body: &str) -> TestApp {
    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: write_stub(stub_dir.path(), conformance_body),
        temp_root: temp_root.path().to_path_buf(),
        conformance_timeout_secs: 10,
        ..EngineConfig::default()
    };
    app_with(Some(Arc::new(mock)), engine, Some(stub_dir), temp_root).await
}

/// Drain a response into (status, JSON body). Non-JSON bodies panic — every
/// endpoint here answers JSON.
async fn drain_json(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let json: Value = serde_json::from_slice(&bytes)
        .unwrap_or_else(|_| json!({ "_raw": String::from_utf8_lossy(&bytes) }));
    (status, json)
}

/// POST a JSON value to `/api/v1/packages/generate`.
async fn post_generate(router: &axum::Router, body: &Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/packages/generate")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain_json(response).await
}

/// A valid single-department draft JSON body (raw model output).
fn valid_draft_json() -> String {
    json!({
        "files": [
            { "path": "departments/hello/main.lua", "content": "local M = {}\nM.spec = { consumes = {}, produces = {} }\nfunction pipeline(event) end\nreturn M\n" }
        ],
        "composed_deps": []
    })
    .to_string()
}

// ---- tests ------------------------------------------------------------------

#[tokio::test]
async fn generate_happy_path_returns_valid_draft_and_runs_conformance() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let mock = MockGateway::new(vec![Ok(valid_draft_json())]);
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "say hello", "name": "happy-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["package"]["name"], "happy-pkg");
    assert_eq!(body["validation"]["ok"], true, "body: {body}");
    assert_eq!(body["conformance"]["status"], "ok", "body: {body}");
    assert_eq!(body["attempts"], 1);
    assert_eq!(body["saved"], false);
}

#[tokio::test]
async fn generate_retries_once_on_validation_failure_and_succeeds() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // Attempt 1: a file path with `..` — NewPackage::validate rejects it.
    let bad = json!({
        "files": [
            { "path": "departments/hello/main.lua", "content": "return {}" },
            { "path": "../escape.lua", "content": "x" }
        ]
    })
    .to_string();
    let mock = MockGateway::new(vec![Ok(bad), Ok(valid_draft_json())]);
    let recorder = mock.clone();
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "retry me", "name": "retry-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["validation"]["ok"], true, "body: {body}");
    assert_eq!(body["attempts"], 2, "body: {body}");

    // The 2nd user prompt must carry the first attempt's validation error.
    let calls = recorder.calls();
    assert_eq!(calls.len(), 2, "exactly two gateway calls");
    let second_user = &calls[1].1;
    assert!(
        second_user.contains("unsafe path component"),
        "retry prompt must include the prior error, got: {second_user}"
    );
}

#[tokio::test]
async fn generate_double_failure_reports_errors_without_http_error() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let bad = json!({
        "files": [ { "path": "../escape.lua", "content": "x" } ]
    })
    .to_string();
    let mock = MockGateway::new(vec![Ok(bad.clone()), Ok(bad)]);
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "always bad", "name": "bad-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "draft issues are still a 200");
    assert_eq!(body["validation"]["ok"], false, "body: {body}");
    assert!(
        body["validation"]["errors"]
            .as_array()
            .is_some_and(|e| !e.is_empty()),
        "validation errors must be reported: {body}"
    );
    assert_eq!(body["conformance"]["status"], "skipped", "body: {body}");
    assert_eq!(body["attempts"], 2);
}

#[tokio::test]
async fn generate_strips_markdown_fences() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let fenced = format!("```json\n{}\n```", valid_draft_json());
    let mock = MockGateway::new(vec![Ok(fenced)]);
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "fenced output", "name": "fenced-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(
        body["validation"]["ok"], true,
        "fenced JSON must parse: {body}"
    );
}

#[tokio::test]
async fn generate_rejects_oversize_description_400() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // The gateway would panic if called: assert it never is by scripting no
    // replies (an extra call returns the exhausted-script error, but we assert
    // a 400 BEFORE any call instead).
    let mock = MockGateway::new(vec![]);
    let recorder = mock.clone();
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let big = "x".repeat(8193);
    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": big, "name": "oversize-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {body}");
    assert!(recorder.calls().is_empty(), "gateway must not be called");
}

#[tokio::test]
async fn generate_unconfigured_gateway_is_503() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        temp_root: temp_root.path().to_path_buf(),
        ..EngineConfig::default()
    };
    let app = app_with(None, engine, None, temp_root).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "no gateway", "name": "x" }),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE, "body: {body}");
}

#[tokio::test]
async fn generate_conformance_failed_is_reported_with_stderr_excerpt() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let mock = MockGateway::new(vec![Ok(valid_draft_json())]);
    let app = app_with_stub_engine(mock, FAIL_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "fails conformance", "name": "conf-fail-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["validation"]["ok"], true);
    assert_eq!(body["conformance"]["status"], "failed", "body: {body}");
    let errors = body["conformance"]["errors"]
        .as_array()
        .expect("conformance errors array");
    let joined = errors
        .iter()
        .filter_map(|e| e.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        joined.contains("CONFORMANCE-STDERR-MARKER"),
        "stderr excerpt must be surfaced, got: {joined}"
    );
}

#[tokio::test]
async fn generate_save_persists_with_ownership_and_conflict_is_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // Two successful generations of the SAME name; the second save collides.
    let mock = MockGateway::new(vec![Ok(valid_draft_json()), Ok(valid_draft_json())]);
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "save me", "name": "saved-pkg", "save": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["saved"], true, "body: {body}");
    assert!(body["save_error"].is_null(), "no save error: {body}");

    // The package is now listed for the caller.
    let list = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/v1/packages")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    let (list_status, list_body) = drain_json(list).await;
    assert_eq!(list_status, StatusCode::OK);
    let names = list_body.as_array().expect("names array");
    assert!(
        names.iter().any(|n| n == "saved-pkg"),
        "saved package must be listed: {list_body}"
    );

    // A second save with the same name collides => 409.
    let (status2, body2) = post_generate(
        &app.router,
        &json!({ "description": "save me again", "name": "saved-pkg", "save": true }),
    )
    .await;
    assert_eq!(status2, StatusCode::CONFLICT, "body: {body2}");
}

#[tokio::test]
async fn generate_save_with_invalid_draft_reports_save_error_not_saved() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // Both attempts produce an invalid draft, so save is refused.
    let bad = json!({ "files": [ { "path": "../x.lua", "content": "x" } ] }).to_string();
    let mock = MockGateway::new(vec![Ok(bad.clone()), Ok(bad)]);
    let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "bad save", "name": "bad-save-pkg", "save": true }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["saved"], false, "body: {body}");
    assert!(
        !body["save_error"].is_null(),
        "save_error must be set: {body}"
    );

    // It must NOT have been persisted.
    let fetch = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/v1/packages/bad-save-pkg")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("router responds");
    assert_eq!(
        fetch.status(),
        StatusCode::NOT_FOUND,
        "invalid draft must not be persisted"
    );
}

/// A shared, in-memory tracing sink, dependency-free (no `tracing-test`).
#[derive(Clone, Default)]
struct CaptureBuffer(Arc<Mutex<Vec<u8>>>);

impl CaptureBuffer {
    fn contents(&self) -> Vec<u8> {
        self.0.lock().expect("capture lock").clone()
    }
}

impl std::io::Write for CaptureBuffer {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.lock().expect("capture lock").extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl tracing_subscriber::fmt::MakeWriter<'_> for CaptureBuffer {
    type Writer = CaptureBuffer;
    fn make_writer(&self) -> Self::Writer {
        self.clone()
    }
}

#[test]
fn generate_never_logs_content() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    // Sentinels embedded in BOTH the description and the model's file content;
    // neither may appear in any captured log line.
    const DESC_SENTINEL: &str = "SENTINEL_DESCRIPTION_DO_NOT_LOG";
    const OUTPUT_SENTINEL: &str = "SENTINEL_OUTPUT_DO_NOT_LOG";

    let draft = json!({
        "files": [
            { "path": "departments/hello/main.lua",
              "content": format!("-- {OUTPUT_SENTINEL}\nreturn {{}}\n") }
        ]
    })
    .to_string();

    let capture = CaptureBuffer::default();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(capture.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");

    tracing::subscriber::with_default(subscriber, || {
        runtime.block_on(async {
            let mock = MockGateway::new(vec![Ok(draft)]);
            let app = app_with_stub_engine(mock, PASS_CONFORMANCE).await;
            let (status, body) = post_generate(
                &app.router,
                &json!({ "description": DESC_SENTINEL, "name": "log-pkg" }),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "body: {body}");
        });
    });

    let logged = String::from_utf8_lossy(&capture.contents()).into_owned();
    assert!(
        !logged.contains(DESC_SENTINEL),
        "the description must never be logged"
    );
    assert!(
        !logged.contains(OUTPUT_SENTINEL),
        "model output / file content must never be logged"
    );
}

#[tokio::test]
async fn generate_real_engine_conformance_smoke() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let bin = require_engine!();
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: bin,
        temp_root: temp_root.path().to_path_buf(),
        conformance_timeout_secs: 15,
        ..EngineConfig::default()
    };
    let mock = MockGateway::new(vec![Ok(valid_draft_json())]);
    let app = app_with(Some(Arc::new(mock)), engine, None, temp_root).await;

    let (status, body) = post_generate(
        &app.router,
        &json!({ "description": "real engine smoke", "name": "smoke-pkg" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    assert_eq!(body["validation"]["ok"], true, "body: {body}");
    // The real engine actually ran: the verdict is ok or failed, never skipped.
    let conf = body["conformance"]["status"].as_str().expect("status");
    assert!(
        conf == "ok" || conf == "failed",
        "real-engine conformance must run (not skip), got: {conf} — {body}"
    );
}
