//! Session HTTP API + driver integration tests against an ephemeral Mongo
//! container (testcontainers), driven via `tower::ServiceExt::oneshot`
//! against the REAL `build_router(AppState)` and the REAL driver state
//! machine — only the engine binary is a stub shell script.
//!
//! Every test gets a fresh container and self-skips when Docker is
//! unavailable so `cargo test` stays green on runners without a daemon.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use bson::doc;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::models::{SessionDoc, SessionStatus};
use fkst_hosted_api::packages::PackageRepository;
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

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

/// Supervise branch that goes ready and idles. The SIGTERM the runner sends
/// to the process group default-terminates both the `sh` and the `sleep`.
const READY_SUPERVISE: &str = r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 300"#;

/// Conformance branch that passes.
const PASS_CONFORMANCE: &str = r#"    echo "PASS graph-scan loaded 1 departments, 1 raisers, 1 queues"
    exit 0"#;

/// Write the stub engine script with the given branch bodies.
fn write_stub(dir: &Path, conformance_body: &str, supervise_body: &str) -> PathBuf {
    let path = dir.join("stub-framework.sh");
    let script = format!(
        r#"#!/bin/sh
case "$1" in
  conformance)
{conformance_body}
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

/// Everything a test needs, with the temp dirs and container kept alive.
struct TestApp {
    _container: ContainerAsync<Mongo>,
    _stub_dir: tempfile::TempDir,
    _temp_root: tempfile::TempDir,
    router: axum::Router,
    db: Db,
    sessions: SessionService,
}

/// Start an ephemeral Mongo, write the stub engine, and build the real
/// application router over both.
async fn app(conformance_body: &str, supervise_body: &str) -> TestApp {
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

    let stub_dir = tempfile::tempdir().expect("stub dir");
    let temp_root = tempfile::tempdir().expect("temp root");
    let engine = EngineConfig {
        framework_bin: write_stub(stub_dir.path(), conformance_body, supervise_body),
        temp_root: temp_root.path().to_path_buf(),
        stop_grace_secs: 5,
        conformance_timeout_secs: 10,
        ready_timeout_secs: 10,
        ..EngineConfig::default()
    };

    let packages = PackageRepository::new(&db.database);
    let sessions = SessionService::new(SessionRepo::new(&db), packages.clone(), engine);
    let router = build_router(AppState {
        config,
        db: db.clone(),
        packages,
        sessions: sessions.clone(),
    });
    TestApp {
        _container: container,
        _stub_dir: stub_dir,
        _temp_root: temp_root,
        router,
        db,
        sessions,
    }
}

/// Drain a response into (status, headers, parsed JSON body or Null).
async fn drain(response: axum::response::Response) -> (StatusCode, HeaderMap, Value) {
    let status = response.status();
    let headers = response.headers().clone();
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
    (status, headers, body)
}

/// POST raw text to a path.
async fn post_raw(
    router: &axum::Router,
    path: &str,
    body: String,
) -> (StatusCode, HeaderMap, Value) {
    let response = router
        .clone()
        .oneshot(
            Request::post(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// POST a JSON value to a path.
async fn post_json(
    router: &axum::Router,
    path: &str,
    body: &Value,
) -> (StatusCode, HeaderMap, Value) {
    post_raw(router, path, body.to_string()).await
}

/// POST with an empty body (the stop endpoint).
async fn post_empty(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
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

/// GET a path.
async fn get_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, Value) {
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

/// Seed a valid stored package the stub engine accepts.
async fn seed_package(router: &axum::Router, name: &str) {
    let body = json!({
        "name": name,
        "files": [
            {
                "path": "departments/hello/main.lua",
                "content": "local M = {}\nM.spec = { consumes = { \"tick\" } }\nfunction pipeline(event) end\nreturn M\n"
            },
            {
                "path": "raisers/tick.lua",
                "content": "return { type = \"cron\", interval = \"1s\", produces = \"tick\" }\n"
            }
        ]
    });
    let (status, _headers, _body) = post_json(router, "/api/v1/packages", &body).await;
    assert_eq!(status, StatusCode::CREATED, "seed package {name}");
}

/// Create a session and return its id.
async fn create_session(router: &axum::Router, package: &str) -> String {
    let (status, _headers, body) = post_json(
        router,
        "/api/v1/sessions",
        &json!({ "package_name": package }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create session: {body}");
    body["id"].as_str().expect("id string").to_string()
}

/// Poll `GET /sessions/{id}` until its status matches, or panic after ~20s.
async fn poll_until(router: &axum::Router, id: &str, expected: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..200 {
        let (status, _headers, body) = get_path(router, &format!("/api/v1/sessions/{id}")).await;
        assert_eq!(status, StatusCode::OK, "GET while polling: {body}");
        if body["status"] == expected {
            return body;
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session {id} never reached {expected:?}; last: {last}");
}

/// Poll until the session reaches ANY terminal status; return the body.
async fn poll_until_terminal(router: &axum::Router, id: &str) -> Value {
    let mut last = Value::Null;
    for _ in 0..200 {
        let (status, _headers, body) = get_path(router, &format!("/api/v1/sessions/{id}")).await;
        assert_eq!(status, StatusCode::OK, "GET while polling: {body}");
        if body["status"] == "stopped" || body["status"] == "failed" {
            return body;
        }
        last = body;
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("session {id} never reached a terminal status; last: {last}");
}

// ---- (1) create + projection -------------------------------------------------

#[tokio::test]
async fn create_answers_201_with_location_and_get_projects_the_document() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;

    let (status, headers, body) = post_json(
        &app.router,
        "/api/v1/sessions",
        &json!({ "package_name": "demo" }),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED);
    let id = body["id"].as_str().expect("id string");
    assert_eq!(
        body,
        json!({ "id": id, "status": "pending" }),
        "201 body is exactly id + pending"
    );
    let location = headers
        .get(header::LOCATION)
        .expect("Location header present")
        .to_str()
        .unwrap();
    assert_eq!(location, format!("/api/v1/sessions/{id}"));

    let (status, _headers, view) = get_path(&app.router, &format!("/api/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(view["id"], id);
    assert_eq!(view["package_name"], "demo");
    // The driver may already have advanced the status; any lifecycle value
    // is legal here — the exact progression is pinned by the lifecycle test.
    let projected = view["status"].as_str().expect("status string");
    assert!(
        ["pending", "validating", "running"].contains(&projected),
        "unexpected early status {projected}"
    );
    // Full CANON projection: every advisory field present (null or set).
    for field in [
        "pod_id",
        "fencing_token",
        "pid",
        "runtime_dir",
        "error",
        "created_at",
        "started_at",
        "stopped_at",
    ] {
        assert!(
            view.get(field).is_some(),
            "projection must carry {field}: {view}"
        );
    }
    assert!(view["created_at"]
        .as_str()
        .expect("created_at string")
        .ends_with('Z'));
    assert!(view["fencing_token"].is_null(), "no lease in v1");
}

// ---- (2) lifecycle happy path --------------------------------------------------

#[tokio::test]
async fn lifecycle_runs_then_stops_cleanly() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;

    let running = poll_until(&app.router, &id, "running").await;
    assert!(running["pid"].as_i64().expect("pid set") > 0);
    assert!(!running["runtime_dir"]
        .as_str()
        .expect("runtime_dir set")
        .is_empty());
    assert!(running["started_at"]
        .as_str()
        .expect("started_at set")
        .ends_with('Z'));
    // pod_id mirrors HOSTNAME at service construction (advisory; may be
    // absent on a local runner).
    match std::env::var("HOSTNAME").ok() {
        Some(host) => assert_eq!(running["pod_id"], json!(host)),
        None => assert!(running["pod_id"].is_null()),
    }

    let (status, _headers, body) =
        post_empty(&app.router, &format!("/api/v1/sessions/{id}/stop")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, json!({ "status": "stopping" }));

    let stopped = poll_until(&app.router, &id, "stopped").await;
    assert!(stopped["stopped_at"]
        .as_str()
        .expect("stopped_at set")
        .ends_with('Z'));
    assert!(stopped["error"].is_null(), "clean stop carries no error");
}

// ---- (3) conformance failure ---------------------------------------------------

#[tokio::test]
async fn conformance_failure_drives_the_session_to_failed() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(
        r#"    echo "FAIL graph-scan department broken missing M.spec" >&2
    exit 1"#,
        READY_SUPERVISE,
    )
    .await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;

    let failed = poll_until(&app.router, &id, "failed").await;
    let error = failed["error"].as_str().expect("error set");
    assert!(
        error.starts_with("conformance failed (exit 1)"),
        "got {error:?}"
    );
    assert!(
        error.contains("FAIL graph-scan"),
        "stderr surfaced: {error:?}"
    );
    assert!(failed["stopped_at"].as_str().is_some(), "stopped_at set");
}

// ---- (4) uncommanded exit ------------------------------------------------------

#[tokio::test]
async fn uncommanded_engine_exit_drives_the_session_to_failed() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(
        PASS_CONFORMANCE,
        r#"    echo "event runtime running handles=3" >&2
    echo "consumer started dept=hello reliable_queues=[] ephemeral_queues=[]" >&2
    sleep 2
    exit 3"#,
    )
    .await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;

    let failed = poll_until(&app.router, &id, "failed").await;
    let error = failed["error"].as_str().expect("error set");
    assert!(
        error.contains("engine exited unexpectedly"),
        "got {error:?}"
    );
    assert!(failed["stopped_at"].as_str().is_some());
}

// ---- (5) stop-vs-create race ---------------------------------------------------

#[tokio::test]
async fn stop_right_after_create_converges_to_stopped_never_failed() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;

    // Immediately request a stop, racing the driver wherever it is.
    let (status, _headers, body) =
        post_empty(&app.router, &format!("/api/v1/sessions/{id}/stop")).await;
    assert_eq!(status, StatusCode::ACCEPTED);
    assert_eq!(body, json!({ "status": "stopping" }));

    let terminal = poll_until_terminal(&app.router, &id).await;
    assert_eq!(
        terminal["status"], "stopped",
        "a commanded stop must never surface as failed: {terminal}"
    );
}

// ---- (6) stop idempotency --------------------------------------------------------

#[tokio::test]
async fn stop_is_idempotent_including_on_a_terminal_session() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;
    poll_until(&app.router, &id, "running").await;

    let stop_path = format!("/api/v1/sessions/{id}/stop");
    let (s1, _h, b1) = post_empty(&app.router, &stop_path).await;
    let (s2, _h, b2) = post_empty(&app.router, &stop_path).await;
    assert_eq!((s1, s2), (StatusCode::ACCEPTED, StatusCode::ACCEPTED));
    assert_eq!(b1, json!({ "status": "stopping" }));
    assert_eq!(b2, b1, "identical body on the idempotent repeat");

    let stopped = poll_until(&app.router, &id, "stopped").await;
    assert!(stopped["error"].is_null());

    // Stop on an already-stopped session: still 202, state unchanged.
    let (s3, _h, b3) = post_empty(&app.router, &stop_path).await;
    assert_eq!(s3, StatusCode::ACCEPTED);
    assert_eq!(b3, json!({ "status": "stopping" }));
    let (status, _headers, after) = get_path(&app.router, &format!("/api/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(after["status"], "stopped", "terminal state unchanged");
}

#[tokio::test]
async fn two_concurrent_stops_both_answer_202_and_converge() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;
    poll_until(&app.router, &id, "running").await;

    let stop_path = format!("/api/v1/sessions/{id}/stop");
    let (left, right) = tokio::join!(
        post_empty(&app.router, &stop_path),
        post_empty(&app.router, &stop_path)
    );
    assert_eq!(left.0, StatusCode::ACCEPTED);
    assert_eq!(right.0, StatusCode::ACCEPTED);
    assert_eq!(left.2, json!({ "status": "stopping" }));
    assert_eq!(right.2, json!({ "status": "stopping" }));

    let terminal = poll_until_terminal(&app.router, &id).await;
    assert_eq!(terminal["status"], "stopped", "{terminal}");
}

// ---- (7) create validation -------------------------------------------------------

#[tokio::test]
async fn create_with_unknown_package_is_404_and_writes_no_document() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;

    let (status, _headers, body) = post_json(
        &app.router,
        "/api/v1/sessions",
        &json!({ "package_name": "ghost" }),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
    assert!(
        body["message"].as_str().unwrap().contains("ghost"),
        "message names the package: {body}"
    );

    let count = app
        .db
        .sessions()
        .count_documents(doc! {})
        .await
        .expect("count");
    assert_eq!(count, 0, "no session document on 404");
}

#[tokio::test]
async fn create_validation_matrix_answers_400_and_writes_no_document() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;

    let rows: Vec<(&str, String)> = vec![
        (
            "bad chars",
            json!({ "package_name": "bad name" }).to_string(),
        ),
        ("empty", json!({ "package_name": "" }).to_string()),
        (
            "129 bytes",
            json!({ "package_name": "a".repeat(129) }).to_string(),
        ),
        ("path-like", json!({ "package_name": "a/b" }).to_string()),
        ("dotted", json!({ "package_name": "." }).to_string()),
        ("missing field", "{}".to_string()),
        (
            "unknown field",
            json!({ "package_name": "ok", "bogus": 1 }).to_string(),
        ),
        ("malformed JSON", "{not json".to_string()),
    ];
    for (label, body) in rows {
        let (status, _headers, envelope) = post_raw(&app.router, "/api/v1/sessions", body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "case {label:?}");
        assert_eq!(envelope["error"], "invalid_request", "case {label:?}");
        assert!(
            envelope["message"].as_str().is_some(),
            "case {label:?}: {envelope}"
        );
    }

    let count = app
        .db
        .sessions()
        .count_documents(doc! {})
        .await
        .expect("count");
    assert_eq!(count, 0, "validation failures must never reach the store");
}

// ---- (8) id parsing / lookup -------------------------------------------------------

#[tokio::test]
async fn malformed_and_unknown_ids_map_to_400_and_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;

    for path in [
        "/api/v1/sessions/not-a-uuid",
        "/api/v1/sessions/not-a-uuid/stop",
    ] {
        let (status, _headers, body) = if path.ends_with("/stop") {
            post_empty(&app.router, path).await
        } else {
            get_path(&app.router, path).await
        };
        assert_eq!(status, StatusCode::BAD_REQUEST, "path {path}");
        assert_eq!(body["error"], "invalid_request", "path {path}");
        assert_eq!(body["message"], "invalid session id: must be a UUID");
    }

    let ghost = "f4e2c0a1-9b3d-4d2e-8c11-3a6b5e0d7f12";
    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{ghost}")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");

    let (status, _headers, body) =
        post_empty(&app.router, &format!("/api/v1/sessions/{ghost}/stop")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(body["error"], "not_found");
}

#[tokio::test]
async fn uppercase_uuid_resolves_the_stored_session() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;

    let upper = id.to_uppercase();
    assert_ne!(upper, id, "premise: the canonical id is lowercase");
    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{upper}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["id"], id, "projection echoes the canonical id");
}

// ---- (9) orphan sweep ----------------------------------------------------------------

#[tokio::test]
async fn orphan_sweep_fails_only_pre_terminal_sessions_and_is_idempotent() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    let repo = SessionRepo::new(&app.db);

    let mk = |status: SessionStatus| SessionDoc {
        id: bson::Uuid::new(),
        package_name: "demo".to_string(),
        status,
        pod_id: Some("dead-pod".to_string()),
        fencing_token: None,
        pid: Some(4242),
        runtime_dir: None,
        error: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    };
    // Every pre-terminal status must be swept. The `stopping` row is the
    // durable backstop for a stop request that was acknowledged but whose
    // driver died (with the pod) before completing it.
    let pre_terminal = [
        mk(SessionStatus::Pending),
        mk(SessionStatus::Validating),
        mk(SessionStatus::Running),
        mk(SessionStatus::Stopping),
    ];
    let stopped = mk(SessionStatus::Stopped);
    let failed = mk(SessionStatus::Failed);
    for doc in &pre_terminal {
        repo.insert(doc).await.expect("insert pre-terminal");
    }
    repo.insert(&stopped).await.expect("insert stopped");
    repo.insert(&failed).await.expect("insert failed");

    let swept = repo.fail_orphans().await.expect("sweep");
    assert_eq!(swept, 4, "exactly the pre-terminal sessions are swept");

    for doc in &pre_terminal {
        let (status, _headers, body) =
            get_path(&app.router, &format!("/api/v1/sessions/{}", doc.id)).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(body["status"], "failed", "seeded {:?}: {body}", doc.status);
        assert_eq!(body["error"], "orphaned by pod restart");
        assert!(body["stopped_at"].as_str().is_some());
    }

    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{}", stopped.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "stopped", "terminal sessions untouched");

    let (status, _headers, body) =
        get_path(&app.router, &format!("/api/v1/sessions/{}", failed.id)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "failed", "already-failed left alone");
    assert!(
        body["error"].is_null(),
        "an already-failed session must not be re-stamped: {body}"
    );

    let again = repo.fail_orphans().await.expect("second sweep");
    assert_eq!(again, 0, "sweep is idempotent");
}

// ---- (10) graceful shutdown ------------------------------------------------------

#[tokio::test]
async fn graceful_shutdown_records_running_sessions_as_stopped() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app(PASS_CONFORMANCE, READY_SUPERVISE).await;
    seed_package(&app.router, "demo").await;
    let id = create_session(&app.router, "demo").await;
    poll_until(&app.router, &id, "running").await;

    // SIGTERM-driven pod shutdown signals the drivers directly, WITHOUT any
    // HTTP stop having CAS'd the document to `stopping`. The stop-success
    // CAS must therefore accept `running` too — otherwise the document
    // lingers `running` and the next boot's orphan sweep mislabels a clean
    // shutdown as "orphaned by pod restart".
    app.sessions.shutdown().await;

    let (status, _headers, body) = get_path(&app.router, &format!("/api/v1/sessions/{id}")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        body["status"], "stopped",
        "a shutdown stop must persist as stopped: {body}"
    );
    assert!(body["stopped_at"].as_str().is_some(), "stopped_at set");
    assert!(body["error"].is_null(), "clean shutdown carries no error");
}
