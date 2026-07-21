//! API-mode (Bearer-token) integration tests for the log-download endpoint:
//! identity resolution (valid user → identity, rejected token → 401), the three-tier
//! authorization matrix (author / per-issue / admin allow, non-member deny → 403),
//! and the not-found / unconfigured cases. Fixtures live in [`super::test_support`];
//! the browser-mode + secret-hygiene suites live in [`super::tests_browser`].

use std::sync::Arc;

use axum::http::{header, StatusCode};
use flate2::write::GzEncoder;
use flate2::Compression;
use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::test_support::*;
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

/// A one-file `tar.gz` whose `fkst-hosted/driver.log` carries `driver`, so two
/// bundles with distinct driver lines/sizes can prove a `?run=` read hit the
/// right object.
fn bundle_with_driver(driver: &[u8]) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(driver.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        builder
            .append_data(&mut header, "fkst-hosted/driver.log", driver)
            .expect("append entry");
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

/// A chrono-storage mock serving a distinct `latest` bundle at the latest key AND a
/// `run` bundle at `logs/<sid>/runs/<run_id>.tar.gz`, so a route-level `?run=` read
/// can be shown to route to the per-run object rather than latest.
async fn storage_latest_and_run(
    run_id: &str,
    latest: Vec<u8>,
    run: Vec<u8>,
) -> (Arc<ChronoStorageClient>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "sa-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param(
            "key",
            format!("logs/{SESSION_ID}/latest.tar.gz"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(latest))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param(
            "key",
            format!("logs/{SESSION_ID}/runs/{run_id}.tar.gz"),
        ))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(run))
        .mount(&server)
        .await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    (
        Arc::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

#[tokio::test]
async fn api_mode_author_gets_the_streamed_bundle() {
    let gh = github_user_ok("alice", AUTHOR_ID).await; // id matches the author
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}"),
        Some("gho_author"),
    )
    .await;
    // Streamed through the control plane as a gzip attachment — NO presigned URL is ever
    // handed back to the caller (it stays server-side inside `stream_download`).
    assert_eq!(response.status(), StatusCode::OK);
    let cd = response
        .headers()
        .get(header::CONTENT_DISPOSITION)
        .expect("content-disposition present")
        .to_str()
        .unwrap();
    assert!(cd.starts_with("attachment;"), "must be an attachment: {cd}");
    let ct = response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap();
    assert_eq!(ct, "application/gzip");
    assert_eq!(body_bytes(response).await, BUNDLE_BYTES);
}

#[tokio::test]
async fn api_mode_rejected_token_is_401() {
    let gh = github_user_401().await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), Some("gho_bad")).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    let body = body_json(response).await;
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn api_mode_non_member_is_403() {
    // A valid user who is neither the author, nor listed, nor an admin.
    let gh = github_user_ok("mallory", 4004).await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&["ops"], false),
        registry(&["bob"]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}"),
        Some("gho_mallory"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    let body = body_json(response).await;
    assert_eq!(body["error"], "forbidden");
    assert!(body["message"].as_str().unwrap().contains("not authorized"));
}

#[tokio::test]
async fn api_mode_per_issue_login_grants() {
    // Not the author, but listed in the per-issue `### Log Access Allowlist` allow-list.
    let gh = github_user_ok("Bob", 2002).await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&["bob"]),
    );

    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), Some("gho_bob")).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_mode_legacy_log_admin_grants() {
    let gh = github_user_ok("ops", 3003).await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&["ops"], false),
        registry(&[]),
    );

    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), Some("gho_ops")).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_mode_deployment_global_admin_grants() {
    let gh = github_user_ok("ops", 3003).await;
    let (storage, _s) = storage_server(true).await;
    let mut st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );
    st.config.access = crate::access_policy::AccessPolicy::from_vars(&[(
        "FKST_GLOBAL_ADMINS".to_string(),
        "@OPS".to_string(),
    )])
    .expect("valid global-admin fixture");

    let response = get(st, &format!("/api/v1/logs/{SESSION_ID}"), Some("gho_ops")).await;
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn api_mode_unknown_session_is_404() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(true).await;
    // Registry has SESSION_ID but we request a different one.
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(st, "/api/v1/logs/does-not-exist", Some("gho_author")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn api_mode_missing_object_is_404() {
    // Authorized (author), but the storage object is absent → 404 "no logs yet".
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(false).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let body = body_json(response).await;
    assert!(body["message"]
        .as_str()
        .unwrap()
        .contains("no logs available"));
}

#[tokio::test]
async fn storage_not_configured_is_503() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let st = state(gh.uri(), None, log_config(&[], false), registry(&[]));

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn api_mode_empty_run_serves_the_latest_bundle() {
    // An empty `?run=` must normalize to latest (NOT resolve to
    // `logs/<sid>/runs/.tar.gz`, a guaranteed 404). The storage mock only serves
    // the latest key, so a 200 with the latest bytes proves the normalization.
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}?run="),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, BUNDLE_BYTES);
}

#[tokio::test]
async fn download_routes_run_and_latest_to_distinct_bundles() {
    // "RUN-9\n" (6 bytes) vs "LATEST\n" (7 bytes): distinct objects, so the streamed
    // body proves which key the `?run=` selector resolved to.
    let latest = bundle_with_driver(b"LATEST\n");
    let run = bundle_with_driver(b"RUN-9\n");
    let (storage, _s) = storage_latest_and_run("run-9", latest.clone(), run.clone()).await;
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    // `?run=run-9` streams the per-run object.
    let response = get(
        st.clone(),
        &format!("/api/v1/logs/{SESSION_ID}?run=run-9"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, run);

    // `?run=latest` streams the authoritative latest object.
    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}?run=latest"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(body_bytes(response).await, latest);
}

#[tokio::test]
async fn manifest_routes_run_to_the_per_run_bundle() {
    let latest = bundle_with_driver(b"LATEST\n"); // 7-byte driver
    let run = bundle_with_driver(b"RUN-9\n"); // 6-byte driver
    let (storage, _s) = storage_latest_and_run("run-9", latest, run).await;
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/manifest?run=run-9"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(body["session_id"], SESSION_ID);
    let files = body["files"].as_array().expect("files array");
    let driver = files
        .iter()
        .find(|f| f["path"] == "fkst-hosted/driver.log")
        .expect("the run bundle's driver.log is listed");
    // Size 6 ("RUN-9\n") proves the RUN bundle was read, not latest (7 bytes).
    assert_eq!(driver["size"], 6);
}
