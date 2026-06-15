//! End-to-end happy path (issue #21, the v1 MVP exit criterion) composed
//! fully in-process: testcontainers MongoDB + the REAL `build_router` /
//! `AppState` / `SessionService` + the REAL bundled engine.
//!
//! Self-skipping, honestly: the test engages only when BOTH a Docker daemon
//! (for the ephemeral Mongo) AND a real `fkst-framework` are available. The
//! engine resolves via `tests/support/mod.rs` (`FKST_ENGINE_BIN`, then
//! `/usr/local/bin/fkst-framework`, then — Linux only — Docker extraction
//! from `FKST_ENGINE_IMAGE`); on hosts without one (e.g. macOS without
//! `FKST_ENGINE_BIN`) it prints a SKIP line and returns. NOTE: no CI job
//! currently provides the engine image to `cargo test`, so the green gate
//! here is compile + clean self-skip; the full run engages on engine-capable
//! hosts and via the operator script `scripts/e2e/run-e2e.sh`.
//!
//! The package content is read FROM DISK out of the shared fixture
//! `backend/tests/fixtures/e2e-minimal/departments/hello/main.lua` — the
//! same bytes the shell script POSTs — never inlined here.

mod support;

use std::path::Path;
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::http::{header, Request, StatusCode};
use fkst_hosted_api::auth::AuthMode;
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::packages::{PackageRepository, ShareRepo};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use support::require_engine;
use testcontainers::runners::AsyncRunner;
use testcontainers::ImageExt;
use tower::ServiceExt;

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`.
const MONGO_TAG: &str = "7";

/// Upper bound on each poll phase (start and stop), mirroring the operator
/// script's spirit with a CI-friendly cap.
const POLL_PHASE_TIMEOUT: Duration = Duration::from_secs(60);

/// Interval between status polls.
const POLL_INTERVAL: Duration = Duration::from_millis(250);

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

/// The single source of fixture truth, read from disk (never inlined).
fn fixture_lua() -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../tests/fixtures/e2e-minimal/departments/hello/main.lua");
    std::fs::read_to_string(&path)
        .unwrap_or_else(|err| panic!("read fixture {}: {err}", path.display()))
}

/// Drain a response into (status, parsed JSON body or Null).
async fn drain(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes).expect("JSON body")
    };
    (status, body)
}

async fn post_json(router: &axum::Router, path: &str, body: &Value) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

async fn post_empty(router: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

async fn get_path(router: &axum::Router, path: &str) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::get(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// Rank of a start-phase status on the forward path; `None` = illegal.
fn start_rank(status: &str) -> Option<usize> {
    ["pending", "validating", "running"]
        .iter()
        .position(|legal| *legal == status)
}

/// Rank of a stop-phase status on the forward path; `None` = illegal.
fn stop_rank(status: &str) -> Option<usize> {
    ["stopping", "stopped"]
        .iter()
        .position(|legal| *legal == status)
}

/// Poll `GET /sessions/{id}` until `target`, collecting every observed
/// status. Asserts each observation is legal for the phase (per `rank`) and
/// that the sequence is monotonic (poll-timing skips are fine; regressions
/// are not). Returns the final body.
async fn poll_to(
    router: &axum::Router,
    id: &str,
    target: &str,
    rank: fn(&str) -> Option<usize>,
) -> Value {
    let deadline = Instant::now() + POLL_PHASE_TIMEOUT;
    let mut observed: Vec<String> = Vec::new();
    let mut last_rank = 0usize;
    let mut last = Value::Null;
    while Instant::now() < deadline {
        let (status, body) = get_path(router, &format!("/api/v1/sessions/{id}")).await;
        assert_eq!(status, StatusCode::OK, "GET while polling: {body}");
        let current = body["status"].as_str().expect("status string").to_string();
        let current_rank = rank(&current).unwrap_or_else(|| {
            panic!("illegal status {current:?} while polling to {target:?} (error: {}, observed: {observed:?})", body["error"])
        });
        assert!(
            current_rank >= last_rank,
            "status regressed to {current:?} (observed: {observed:?})"
        );
        last_rank = current_rank;
        observed.push(current.clone());
        if current == target {
            return body;
        }
        last = body;
        tokio::time::sleep(POLL_INTERVAL).await;
    }
    panic!("session {id} never reached {target:?} within {POLL_PHASE_TIMEOUT:?}; observed: {observed:?}; last: {last}");
}

#[tokio::test]
async fn e2e_happy_path_runs_then_stops_against_the_real_engine() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let engine_bin = require_engine!();

    // -- compose the real application over an ephemeral Mongo --------------
    let container = testcontainers_modules::mongo::Mongo::default()
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

    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: engine_bin,
        temp_root: temp_root.path().to_path_buf(),
        ..EngineConfig::default()
    };
    let packages = PackageRepository::new(&db.database);
    let shares = ShareRepo::new(&db.database);
    let goals = GoalRepo::new(&db.database);
    let sessions = SessionService::new(SessionRepo::new(&db), packages.clone(), engine);
    let vault = support::test_vault(&db);
    let router = build_router(AppState {
        config,
        db,
        packages,
        shares,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        goals,
        engine: EngineConfig::default(),
        llm: None,
        vault,
        ornn: None,
    })
    .expect("router");

    // -- 1. create the package from the on-disk fixture --------------------
    let (status, body) = post_json(
        &router,
        "/api/v1/packages",
        &json!({
            "name": "e2e-minimal",
            "files": [
                { "path": "departments/hello/main.lua", "content": fixture_lua() }
            ]
        }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create package: {body}");

    // -- 2. start a session -------------------------------------------------
    let (status, body) = post_json(
        &router,
        "/api/v1/sessions",
        &json!({ "package_name": "e2e-minimal" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create session: {body}");
    let id = body["id"].as_str().expect("id string").to_string();
    assert!(!id.is_empty(), "session id must be non-empty");
    assert_eq!(body["status"], "pending", "fresh session is pending");

    // -- 3. poll to running (pending → validating → running, monotonic) ----
    let running = poll_to(&router, &id, "running", start_rank).await;
    assert!(running["error"].is_null(), "no error while running");

    // -- 4. stop -------------------------------------------------------------
    let (status, body) = post_empty(&router, &format!("/api/v1/sessions/{id}/stop")).await;
    assert_eq!(status, StatusCode::ACCEPTED, "stop: {body}");

    // -- 5. poll to stopped (stopping → stopped, monotonic) ------------------
    let stopped = poll_to(&router, &id, "stopped", stop_rank).await;
    assert!(
        stopped["error"].is_null(),
        "happy path carries no error: {stopped}"
    );
    assert!(
        stopped["stopped_at"].as_str().is_some(),
        "stopped_at set once stopped: {stopped}"
    );
}
