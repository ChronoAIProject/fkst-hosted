//! Zip archive upload integration tests against an ephemeral Mongo container
//! (testcontainers). Tests POST and PUT `/api/v1/packages/{name}/archive`
//! with real zip payloads built in-memory.

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use fkst_hosted_api::auth::AuthMode;
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::packages::{PackageRepository, ShareRepo, MAX_FILES, MAX_FILE_CONTENT_BYTES};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::sessions::{SessionRepo, SessionService};
use fkst_hosted_api::state::AppState;
use http_body_util::BodyExt;
use serde_json::{json, Value};
use std::io::{Cursor, Write};
use testcontainers::runners::AsyncRunner;
use testcontainers::{ContainerAsync, ImageExt};
use testcontainers_modules::mongo::Mongo;
use tower::ServiceExt;

fn docker_available() -> bool {
    std::process::Command::new("docker")
        .args(["info"])
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

const MONGO_TAG: &str = "7";

struct TestApp {
    _container: ContainerAsync<Mongo>,
    router: axum::Router,
}

async fn app() -> TestApp {
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
    })
    .expect("router");
    TestApp {
        _container: container,
        router,
    }
}

async fn drain(response: axum::response::Response) -> (StatusCode, HeaderMap, String) {
    let status = response.status();
    let headers = response.headers().clone();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("collect body")
        .to_bytes();
    let body = String::from_utf8(bytes.to_vec()).expect("utf-8 body");
    (status, headers, body)
}

fn parse(body: &str) -> Value {
    serde_json::from_str(body).expect("JSON body")
}

/// Build a zip archive in memory from owned entries.
fn build_zip(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    let mut writer = zip::ZipWriter::new(&mut buf);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    for (name, data) in entries {
        writer
            .start_file(name.as_str(), options)
            .expect("start_file");
        writer.write_all(data).expect("write");
    }
    writer.finish().expect("finish");
    buf.into_inner()
}

/// POST a zip archive to `/api/v1/packages/{name}/archive`.
async fn post_archive(
    router: &axum::Router,
    name: &str,
    zip_bytes: Vec<u8>,
) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::post(format!("/api/v1/packages/{name}/archive"))
                .header(header::CONTENT_TYPE, "application/zip")
                .body(Body::from(zip_bytes))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// PUT a zip archive to `/api/v1/packages/{name}/archive`.
async fn put_archive(
    router: &axum::Router,
    name: &str,
    zip_bytes: Vec<u8>,
) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::put(format!("/api/v1/packages/{name}/archive"))
                .header(header::CONTENT_TYPE, "application/zip")
                .body(Body::from(zip_bytes))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// GET an arbitrary path.
async fn get_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, String) {
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

/// POST a JSON value to /api/v1/packages (for creating packages via JSON first).
async fn post_json(router: &axum::Router, body: &Value) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/packages")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

fn valid_body(name: &str) -> Value {
    json!({
        "name": name,
        "files": [
            { "path": "departments/x/main.lua", "content": "return {}" }
        ]
    })
}

// ---- POST archive create -----------------------------------------------------

#[tokio::test]
async fn archive_post_creates_package_from_zip() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("composed.deps".to_string(), b"dep-a\ndep-b\n".to_vec()),
    ]);

    let (status, headers, raw) = post_archive(&app.router, "zip-pkg", zip_bytes).await;
    assert_eq!(status, StatusCode::CREATED, "body: {raw}");
    let body = parse(&raw);
    assert_eq!(body["name"], "zip-pkg");

    // Location header.
    let location = headers
        .get(header::LOCATION)
        .expect("Location header present")
        .to_str()
        .unwrap();
    assert_eq!(location, "/api/v1/packages/zip-pkg");

    // GET to verify composed_deps were parsed (not stored as a file).
    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/zip-pkg").await;
    assert_eq!(status, StatusCode::OK);
    let fetched = parse(&raw);
    assert_eq!(fetched["name"], "zip-pkg");
    assert_eq!(fetched["composed_deps"], json!(["dep-a", "dep-b"]));
    // composed.deps must NOT be in files.
    let files = fetched["files"].as_array().unwrap();
    assert_eq!(files.len(), 1);
    assert_eq!(files[0]["path"], "departments/x/main.lua");
}

#[tokio::test]
async fn archive_post_duplicate_is_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[("departments/x/main.lua".to_string(), b"return {}".to_vec())]);

    let (status, _h, _raw) = post_archive(&app.router, "dup-zip", zip_bytes.clone()).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _headers, raw) = post_archive(&app.router, "dup-zip", zip_bytes).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "conflict");
}

// ---- POST archive reject matrix -----------------------------------------------

#[tokio::test]
async fn archive_rejects_zip_slip_and_absolute_paths() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Case 1: parent traversal
    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("../evil.lua".to_string(), b"x".to_vec()),
    ]);
    let (status, _headers, raw) = post_archive(&app.router, "slip-parent", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "case parent traversal");
    let envelope = parse(&raw);
    assert_eq!(
        envelope["error"], "invalid_request",
        "case parent traversal"
    );
    let msg = envelope["message"].as_str().unwrap();
    assert!(
        msg.contains("unsafe path component"),
        "case parent traversal: got {msg}"
    );

    // Case 2: absolute path
    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("/etc/passwd".to_string(), b"x".to_vec()),
    ]);
    let (status, _headers, raw) = post_archive(&app.router, "slip-abs", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "case absolute path");
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request", "case absolute path");
    let msg = envelope["message"].as_str().unwrap();
    assert!(
        msg.contains("absolute path"),
        "case absolute path: got {msg}"
    );
}

#[tokio::test]
async fn archive_rejects_entry_count_over_256() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    for i in 0..=MAX_FILES + 1 {
        entries.push((format!("departments/d{i}/main.lua"), vec![b'x']));
    }
    let zip_bytes = build_zip(&entries);
    let (status, _headers, raw) = post_archive(&app.router, "toomany", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .starts_with("too many entries"),
        "got: {envelope}"
    );
}

#[tokio::test]
async fn archive_rejects_decoded_size_over_caps() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Per-file cap: one file over MAX_FILE_CONTENT_BYTES.
    let big = vec![b'x'; MAX_FILE_CONTENT_BYTES + 1];
    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("big.lua".to_string(), big),
    ]);
    let (status, _headers, raw) = post_archive(&app.router, "perfile-cap", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        parse(&raw)["message"]
            .as_str()
            .unwrap()
            .contains("file content too large"),
        "got: {}",
        raw
    );
}

#[tokio::test]
async fn archive_rejects_non_utf8() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("bad.lua".to_string(), vec![0xff, 0xfe]),
    ]);
    let (status, _headers, raw) = post_archive(&app.router, "nonutf8", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    let msg = envelope["message"].as_str().unwrap();
    assert!(msg.contains("not valid UTF-8"), "got: {msg}");
}

#[tokio::test]
async fn archive_rejects_root_fkst_env() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[
        ("departments/x/main.lua".to_string(), b"return {}".to_vec()),
        ("fkst.env".to_string(), b"HOST_VAR=x".to_vec()),
    ]);
    let (status, _headers, raw) = post_archive(&app.router, "fkstenv", zip_bytes).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    let msg = envelope["message"].as_str().unwrap();
    assert!(msg.contains("fkst.env"), "got: {msg}");
}

#[tokio::test]
async fn archive_post_rejects_wrong_content_type() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[("departments/x/main.lua".to_string(), b"return {}".to_vec())]);
    // Send with wrong content type.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/v1/packages/wrong-ct/archive")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(zip_bytes))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    let (status, _headers, raw) = drain(response).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert!(
        parse(&raw)["message"]
            .as_str()
            .unwrap()
            .contains("application/zip"),
        "got: {raw}"
    );
}

// ---- PUT archive replace ------------------------------------------------------

#[tokio::test]
async fn archive_put_replaces_existing() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Create via JSON first.
    let (status, _h, _raw) = post_json(&app.router, &valid_body("replace-zip")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Replace via zip archive.
    let zip_bytes = build_zip(&[
        (
            "departments/replaced/main.lua".to_string(),
            b"return 'v2'".to_vec(),
        ),
        ("lib/util.lua".to_string(), b"-- util v2".to_vec()),
    ]);
    let (status, _headers, raw) = put_archive(&app.router, "replace-zip", zip_bytes).await;
    assert_eq!(status, StatusCode::OK, "body: {raw}");
    let body = parse(&raw);
    assert_eq!(body["name"], "replace-zip");
    let files = body["files"].as_array().unwrap();
    assert_eq!(files.len(), 2);
    assert_eq!(files[0]["path"], "departments/replaced/main.lua");

    // GET to confirm replacement.
    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/replace-zip").await;
    assert_eq!(status, StatusCode::OK);
    let fetched = parse(&raw);
    assert_eq!(fetched["files"].as_array().unwrap().len(), 2);
}

#[tokio::test]
async fn archive_put_unknown_is_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let zip_bytes = build_zip(&[("departments/x/main.lua".to_string(), b"return {}".to_vec())]);
    let (status, _headers, raw) = put_archive(&app.router, "ghost-zip", zip_bytes).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    assert_eq!(parse(&raw)["error"], "not_found");
}
