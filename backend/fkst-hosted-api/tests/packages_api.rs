//! Package HTTP API integration tests against an ephemeral Mongo container
//! (testcontainers), driven via `tower::ServiceExt::oneshot` against the
//! REAL `build_router(AppState)` — the full middleware stack, no mock layer.
//!
//! Every test gets a fresh container and self-skips when Docker is
//! unavailable so `cargo test` stays green on runners without a daemon.

use axum::body::Body;
use axum::http::{header, HeaderMap, Request, StatusCode};
use fkst_hosted_api::auth::AuthMode;
use fkst_hosted_api::authz::Authorizer;
use fkst_hosted_api::config::Config;
use fkst_hosted_api::db::Db;
use fkst_hosted_api::engine::EngineConfig;
use fkst_hosted_api::goals::GoalRepo;
use fkst_hosted_api::models::{LeaseDoc, SessionDoc, SessionStatus};
use fkst_hosted_api::packages::{PackageRepository, ShareRepo, MAX_FILE_CONTENT_BYTES};
use fkst_hosted_api::router::build_router;
use fkst_hosted_api::routes::packages::MAX_REQUEST_BODY_BYTES;
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

/// Mongo image tag — pinned to the same major as `backend/docker-compose.yml`
/// so integration tests and local dev exercise the same server line.
const MONGO_TAG: &str = "7";

/// Start an ephemeral Mongo and build the real application router over it.
/// Returns the container (kept alive for the test duration), the router, and
/// the `Db` handle for direct collection manipulation in tests.
struct TestApp {
    _container: ContainerAsync<Mongo>,
    router: axum::Router,
    db: Db,
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
        db: db.clone(),
        packages,
        shares,
        sessions,
        auth_mode: AuthMode::Disabled,
        authz: Authorizer::disabled(),
        github_app: None,
        goals,
    })
    .expect("router");
    TestApp {
        _container: container,
        router,
        db,
    }
}

/// Drain a response into (status, headers, raw body string).
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

/// POST a raw JSON string to /api/v1/packages.
async fn post_raw(router: &axum::Router, body: String) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::post("/api/v1/packages")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// POST a JSON value to /api/v1/packages.
async fn post_json(router: &axum::Router, body: &Value) -> (StatusCode, HeaderMap, String) {
    post_raw(router, body.to_string()).await
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

/// PUT a raw string to an arbitrary path with JSON content type.
async fn put_path(
    router: &axum::Router,
    path: &str,
    body: String,
) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::put(path)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

/// DELETE an arbitrary path.
async fn delete_path(router: &axum::Router, path: &str) -> (StatusCode, HeaderMap, String) {
    let response = router
        .clone()
        .oneshot(
            Request::delete(path)
                .body(Body::empty())
                .expect("request builds"),
        )
        .await
        .expect("router must respond");
    drain(response).await
}

fn parse(body: &str) -> Value {
    serde_json::from_str(body).expect("JSON body")
}

/// A valid multi-file create body: engine entries, a core file, a dotted
/// segment, and one dep — files deliberately NOT in sorted order to prove
/// order round-trips verbatim.
fn valid_body(name: &str) -> Value {
    json!({
        "name": name,
        "files": [
            { "path": "raisers/nightly.lua", "content": "return { { kind = \"cron\" } }\n" },
            { "path": "departments/billing/main.lua", "content": "local M = {} -- héllo 你好 🚀\nreturn M\n" },
            { "path": "core.lua", "content": "-- shared helpers" },
            { "path": "lib/util.v2.lua", "content": "" }
        ],
        "composed_deps": ["util-core"]
    })
}

/// Assert an RFC3339 UTC timestamp: parseable and `Z`-suffixed. The
/// fractional-second width is deliberately not pinned.
fn assert_rfc3339_utc(value: &Value, field: &str) {
    let text = value
        .as_str()
        .unwrap_or_else(|| panic!("{field} is a string"));
    assert!(text.ends_with('Z'), "{field} must end in Z, got {text:?}");
    bson::DateTime::parse_rfc3339_str(text)
        .unwrap_or_else(|error| panic!("{field} must parse as RFC3339 ({text:?}): {error}"));
}

// ---- (1) round-trip ---------------------------------------------------------

#[tokio::test]
async fn post_then_get_round_trips_the_package() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;
    let body = valid_body("billing-pipeline");

    let (status, headers, raw) = post_json(&app.router, &body).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        raw, r#"{"name":"billing-pipeline"}"#,
        "201 body must be exactly the name echo"
    );
    let location = headers
        .get(header::LOCATION)
        .expect("Location header present")
        .to_str()
        .unwrap();
    assert_eq!(location, "/api/v1/packages/billing-pipeline");

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/billing-pipeline").await;
    assert_eq!(status, StatusCode::OK);
    let fetched = parse(&raw);
    assert_eq!(fetched["name"], "billing-pipeline");
    // Identical files: same order, same byte-for-byte content (multibyte
    // UTF-8 and the empty content included).
    assert_eq!(fetched["files"], body["files"]);
    assert_eq!(fetched["composed_deps"], body["composed_deps"]);
    assert_rfc3339_utc(&fetched["created_at"], "created_at");
    assert_rfc3339_utc(&fetched["updated_at"], "updated_at");
    assert_eq!(
        fetched["created_at"], fetched["updated_at"],
        "single clock read on create"
    );
}

#[tokio::test]
async fn omitted_composed_deps_round_trips_as_an_empty_array() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;
    let body = json!({
        "name": "no-deps",
        "files": [{ "path": "departments/x/main.lua", "content": "return {}" }]
    });

    let (status, _headers, _raw) = post_json(&app.router, &body).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/no-deps").await;
    assert_eq!(status, StatusCode::OK);
    let fetched = parse(&raw);
    assert_eq!(fetched["composed_deps"], json!([]));
}

// ---- (2) list ---------------------------------------------------------------

#[tokio::test]
async fn list_is_empty_then_contains_created_names_sorted() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(raw, "[]", "empty store must answer the exact empty array");

    // Create out of order to prove the repository's ascending sort.
    let (status, _h, _b) = post_json(&app.router, &valid_body("beta-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);
    let (status, _h, _b) = post_json(&app.router, &valid_body("alpha-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&raw), json!(["alpha-pkg", "beta-pkg"]));
}

// ---- (3) duplicate ----------------------------------------------------------

#[tokio::test]
async fn duplicate_post_is_409_and_leaves_the_original_untouched() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _b) = post_json(&app.router, &valid_body("dup-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);
    let (_status, _headers, raw) = get_path(&app.router, "/api/v1/packages/dup-pkg").await;
    let original_created_at = parse(&raw)["created_at"].clone();

    let (status, _headers, raw) = post_json(&app.router, &valid_body("dup-pkg")).await;
    assert_eq!(status, StatusCode::CONFLICT);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "conflict");
    assert!(
        envelope["message"].as_str().unwrap().contains("dup-pkg"),
        "conflict message names the package: {envelope}"
    );

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/dup-pkg").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        parse(&raw)["created_at"],
        original_created_at,
        "duplicate POST must not mutate the stored document"
    );
}

// ---- (4) concurrency --------------------------------------------------------

#[tokio::test]
async fn concurrent_same_name_posts_yield_one_201_and_one_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;
    let body = valid_body("race-pkg");

    let (left, right) = tokio::join!(post_json(&app.router, &body), post_json(&app.router, &body));
    let statuses = [left.0, right.0];
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CREATED)
            .count(),
        1,
        "exactly one 201: {statuses:?}"
    );
    assert_eq!(
        statuses
            .iter()
            .filter(|s| **s == StatusCode::CONFLICT)
            .count(),
        1,
        "exactly one 409: {statuses:?}"
    );

    // Exactly one document.
    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(parse(&raw), json!(["race-pkg"]));
}

// ---- (5) missing ------------------------------------------------------------

#[tokio::test]
async fn get_missing_package_is_404_not_found() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages/does-not-exist").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "not_found");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .contains("does-not-exist"),
        "message names the package: {envelope}"
    );
}

// ---- (6) bad path names -----------------------------------------------------

#[tokio::test]
async fn get_with_invalid_name_is_400_and_never_reaches_the_db() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Percent-decoded to "bad name" (space) and "../etc" (traversal): both
    // must fail the anchored name rule, not become Mongo lookups.
    for path in ["/api/v1/packages/bad%20name", "/api/v1/packages/..%2Fetc"] {
        let (status, _headers, raw) = get_path(&app.router, path).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "path {path}");
        let envelope = parse(&raw);
        assert_eq!(envelope["error"], "invalid_request", "path {path}");
        assert!(
            envelope["message"]
                .as_str()
                .unwrap()
                .starts_with("invalid package name"),
            "path {path}: {envelope}"
        );
    }
}

// ---- (7) POST 400 matrix ----------------------------------------------------

#[tokio::test]
async fn post_validation_matrix_answers_400_with_stable_prefixes() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let entry = json!({ "path": "departments/x/main.lua", "content": "return {}" });
    let rows: Vec<(&str, String, &str)> =
        vec![
        (
            "bad name",
            json!({ "name": "bad name", "files": [entry] }).to_string(),
            "invalid package name",
        ),
        (
            "empty files",
            json!({ "name": "ok", "files": [] }).to_string(),
            "package has no files",
        ),
        (
            "traversal path",
            json!({ "name": "ok", "files": [entry, { "path": "a/../b", "content": "x" }] })
                .to_string(),
            "unsafe path component",
        ),
        (
            "absolute path",
            json!({ "name": "ok", "files": [entry, { "path": "/etc/passwd", "content": "x" }] })
                .to_string(),
            "absolute path not allowed",
        ),
        (
            "backslash path",
            json!({ "name": "ok", "files": [entry, { "path": "dir\\file.lua", "content": "x" }] })
                .to_string(),
            "invalid path separator",
        ),
        (
            "control char path",
            json!({ "name": "ok", "files": [entry, { "path": "a\u{1f}b.lua", "content": "x" }] })
                .to_string(),
            "invalid character in path",
        ),
        (
            "duplicate file path",
            json!({ "name": "ok", "files": [entry, entry] }).to_string(),
            "duplicate file path",
        ),
        (
            "composed_dep with newline",
            json!({ "name": "ok", "files": [entry], "composed_deps": ["a\nb"] }).to_string(),
            "invalid composed_dep",
        ),
        (
            "no engine entry",
            json!({ "name": "ok", "files": [{ "path": "core.lua", "content": "x" }] }).to_string(),
            "no engine entry file",
        ),
        (
            "unknown top-level field",
            json!({ "name": "ok", "files": [entry], "bogus": 1 }).to_string(),
            "invalid request body: ",
        ),
        ("malformed JSON", "{not json".to_string(), "invalid request body: "),
    ];

    for (label, body, expected_prefix) in rows {
        let (status, _headers, raw) = post_raw(&app.router, body).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "case {label:?}");
        let envelope = parse(&raw);
        assert_eq!(envelope["error"], "invalid_request", "case {label:?}");
        let message = envelope["message"].as_str().expect("message is a string");
        assert!(
            message.starts_with(expected_prefix),
            "case {label:?}: expected prefix {expected_prefix:?}, got {message:?}"
        );
    }

    // None of the rejected bodies may have been persisted.
    let (status, _headers, raw) = get_path(&app.router, "/api/v1/packages").await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(raw, "[]", "validation failures must never reach the store");
}

// ---- (8) too many files -----------------------------------------------------

#[tokio::test]
async fn post_with_257_files_is_400_too_many_files() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // One over the domain MAX_FILES (256); every path is individually valid.
    let files: Vec<Value> = (0..257)
        .map(|i| json!({ "path": format!("departments/d{i}/main.lua"), "content": "x" }))
        .collect();
    let (status, _headers, raw) =
        post_json(&app.router, &json!({ "name": "ok", "files": files })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .starts_with("too many files"),
        "got: {envelope}"
    );
}

// ---- (9) over-limit body ----------------------------------------------------

#[tokio::test]
async fn over_limit_body_is_400_request_body_too_large_not_413() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // One byte over the route's body cap; the limit fires before
    // deserialization, so the bytes need not be valid JSON.
    let body = "x".repeat(MAX_REQUEST_BODY_BYTES + 1);
    let (status, _headers, raw) = post_raw(&app.router, body).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "must be 400, not 413");
    assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request");
    assert_eq!(envelope["message"], "request body too large");
}

#[tokio::test]
async fn large_legal_package_above_the_axum_default_limit_is_accepted() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Three 1 MiB files of escape-free ASCII (wire size ~ content size) push
    // the request body to ~3 MiB: above axum's built-in 2 MiB body-limit
    // default, well under MAX_REQUEST_BODY_BYTES. A 201 here pins that the
    // raised `DefaultBodyLimit::max(MAX_REQUEST_BODY_BYTES)` layer is
    // actually applied to the packages routes — if that layer were dropped,
    // axum's default would reject this exact legal request.
    let blob = "x".repeat(MAX_FILE_CONTENT_BYTES);
    let body = serde_json::to_string(&json!({
        "name": "big-pkg",
        "files": [
            { "path": "departments/big/main.lua", "content": "return {}" },
            { "path": "blob0.lua", "content": blob },
            { "path": "blob1.lua", "content": blob },
            { "path": "blob2.lua", "content": blob }
        ]
    }))
    .expect("serialize body");
    assert!(
        body.len() > 2 * 1024 * 1024,
        "premise: body ({} bytes) must exceed axum's 2 MiB default",
        body.len()
    );
    assert!(
        body.len() <= MAX_REQUEST_BODY_BYTES,
        "premise: body ({} bytes) must stay under the route cap",
        body.len()
    );

    let (status, _headers, raw) = post_raw(&app.router, body).await;
    assert_eq!(status, StatusCode::CREATED, "body: {raw}");
    assert_eq!(raw, r#"{"name":"big-pkg"}"#);
}

// ---- (10) nest precedence ---------------------------------------------------

#[tokio::test]
async fn api_v1_health_still_answers_next_to_the_packages_nest() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _headers, raw) = get_path(&app.router, "/api/v1/health").await;
    assert_eq!(status, StatusCode::OK);
    let body = parse(&raw);
    assert_eq!(body["status"], "ok");
    assert_eq!(body["mongo"], "up");
}

// ---- (11) PUT replace --------------------------------------------------------

/// PUT /api/v1/packages/{name} replaces files and bumps updated_at while
/// leaving created_at and ownership stable.
#[tokio::test]
async fn put_replaces_files_and_bumps_updated_at_only() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    // Create a package first.
    let (status, _h, _raw) = post_json(&app.router, &valid_body("replace-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Record the original created_at.
    let (_status, _headers, raw) = get_path(&app.router, "/api/v1/packages/replace-pkg").await;
    let original = parse(&raw);
    let original_created_at = original["created_at"].as_str().unwrap().to_string();
    let original_updated_at = original["updated_at"].as_str().unwrap().to_string();

    // Small sleep to ensure updated_at diverges (millisecond precision).
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;

    // PUT with new files.
    let new_body = json!({
        "files": [
            { "path": "departments/updated/main.lua", "content": "return 'v2'" }
        ],
        "composed_deps": ["new-dep"]
    });
    let (status, _headers, raw) = put_path(
        &app.router,
        "/api/v1/packages/replace-pkg",
        new_body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let updated = parse(&raw);
    assert_eq!(updated["name"], "replace-pkg");
    assert_eq!(updated["files"][0]["path"], "departments/updated/main.lua");
    assert_eq!(updated["composed_deps"], json!(["new-dep"]));
    // created_at must be unchanged.
    assert_eq!(updated["created_at"], original_created_at);
    // updated_at must have diverged.
    assert_ne!(updated["updated_at"], original_updated_at);
}

/// PUT to a non-existent package returns 404.
#[tokio::test]
async fn put_unknown_package_is_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let body = json!({
        "files": [{ "path": "departments/x/main.lua", "content": "return {}" }]
    });
    let (status, _headers, raw) =
        put_path(&app.router, "/api/v1/packages/ghost", body.to_string()).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "not_found");
}

/// PUT with invalid body (e.g. bad path) returns 400 with stable prefix.
#[tokio::test]
async fn put_invalid_body_is_400_with_stable_prefix() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _raw) = post_json(&app.router, &valid_body("put-invalid")).await;
    assert_eq!(status, StatusCode::CREATED);

    let body = json!({
        "files": [{ "path": "../evil.lua", "content": "x" }]
    });
    let (status, _headers, raw) = put_path(
        &app.router,
        "/api/v1/packages/put-invalid",
        body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .starts_with("unsafe path component"),
        "got: {envelope}"
    );
}

/// PUT with an unknown body field returns 400 (path is the identity).
#[tokio::test]
async fn put_path_name_is_the_identity() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _raw) = post_json(&app.router, &valid_body("put-identity")).await;
    assert_eq!(status, StatusCode::CREATED);

    let body = json!({
        "files": [{ "path": "departments/x/main.lua", "content": "return {}" }],
        "bogus": 1
    });
    let (status, _headers, raw) = put_path(
        &app.router,
        "/api/v1/packages/put-identity",
        body.to_string(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "invalid_request");
    // Serde's deny_unknown_fields results in a deserialization error.
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .starts_with("invalid request body:"),
        "got: {envelope}"
    );
}

// ---- (12) DELETE -------------------------------------------------------------

/// DELETE removes a package; subsequent GET returns 404.
#[tokio::test]
async fn delete_removes_package_and_answers_204_then_get_is_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _raw) = post_json(&app.router, &valid_body("del-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);

    let (status, _headers, _raw) = delete_path(&app.router, "/api/v1/packages/del-pkg").await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    let (status, _headers, _raw) = get_path(&app.router, "/api/v1/packages/del-pkg").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// DELETE an unknown package returns 404.
#[tokio::test]
async fn delete_unknown_package_is_404() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _headers, _raw) = delete_path(&app.router, "/api/v1/packages/ghost").await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// DELETE when the package has a running session returns 409.
#[tokio::test]
async fn delete_with_running_session_is_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _raw) = post_json(&app.router, &valid_body("del-session-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Insert a running session directly into the sessions collection.
    let session = SessionDoc {
        id: bson::Uuid::new(),
        package_name: "del-session-pkg".to_string(),
        status: SessionStatus::Running,
        pod_id: Some("pod-0".to_string()),
        fencing_token: Some(1),
        pid: None,
        runtime_dir: None,
        error: None,
        run_key: None,
        owner_user_id: None,
        org_id: None,
        created_at: bson::DateTime::now(),
        started_at: None,
        stopped_at: None,
    };
    app.db
        .sessions()
        .insert_one(&session)
        .await
        .expect("insert session");

    let (status, _headers, raw) =
        delete_path(&app.router, "/api/v1/packages/del-session-pkg").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "conflict");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .contains("active session or live lease"),
        "got: {envelope}"
    );

    // Clean up: stop session, then delete should succeed.
    app.db
        .sessions()
        .delete_one(bson::doc! { "_id": session.id })
        .await
        .expect("delete session");
    let (status, _h, _raw) = delete_path(&app.router, "/api/v1/packages/del-session-pkg").await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// DELETE when the package has a live lease returns 409.
#[tokio::test]
async fn delete_with_live_lease_is_409() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    let (status, _h, _raw) = post_json(&app.router, &valid_body("del-lease-pkg")).await;
    assert_eq!(status, StatusCode::CREATED);

    // Insert a live lease directly.
    let lease = LeaseDoc {
        package_name: "del-lease-pkg".to_string(),
        session_id: bson::Uuid::new(),
        holder_pod: "pod-0".to_string(),
        fencing_token: 1,
        expires_at: bson::DateTime::from_millis(bson::DateTime::now().timestamp_millis() + 60_000),
        renewed_at: bson::DateTime::now(),
    };
    app.db
        .leases()
        .insert_one(&lease)
        .await
        .expect("insert lease");

    let (status, _headers, raw) = delete_path(&app.router, "/api/v1/packages/del-lease-pkg").await;
    assert_eq!(status, StatusCode::CONFLICT);
    let envelope = parse(&raw);
    assert_eq!(envelope["error"], "conflict");
    assert!(
        envelope["message"]
            .as_str()
            .unwrap()
            .contains("active session or live lease"),
        "got: {envelope}"
    );
}

// ---- (13) CORS ---------------------------------------------------------------

/// CORS preflight for PUT, DELETE, and PATCH returns the expected methods.
#[tokio::test]
async fn cors_allows_put_and_delete() {
    if !docker_available() {
        eprintln!("skipped: docker unavailable");
        return;
    }
    let app = app().await;

    for method in ["PUT", "DELETE", "PATCH"] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::builder()
                    .method("OPTIONS")
                    .uri("/api/v1/packages/test")
                    .header(header::ACCESS_CONTROL_REQUEST_METHOD, method)
                    .header(header::ORIGIN, "http://localhost:3000")
                    .body(Body::empty())
                    .expect("request builds"),
            )
            .await
            .expect("router must respond");

        let allow_methods = response
            .headers()
            .get(header::ACCESS_CONTROL_ALLOW_METHODS)
            .expect("Access-Control-Allow-Methods header present")
            .to_str()
            .unwrap();
        assert!(
            allow_methods.contains(method),
            "CORS must allow {method}, got: {allow_methods}"
        );
    }
}
