//! Integration tests for `GET /api/v1/logs/{session_id}/runs` (`list_session_runs`):
//! the run-index listing (newest first), the legacy 404 → synthetic-`latest`
//! fallback, and the shared authz / storage-config cases. Fixtures live in
//! [`super::test_support`]; the whole-bundle download suites live in [`super::tests`]
//! / [`super::tests_browser`].

use std::sync::Arc;

use axum::http::StatusCode;
use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::test_support::*;
use crate::session_pod::log_stream::runs::{self, runs_index_key, LogRun};
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

/// A chrono-storage mock serving `body` (Some → 200) or a 404 (None) for the
/// session's run-index key, and answering the `latest.tar.gz` existence probe
/// per `bundle`: `Some(true)` → present, `Some(false)` → 404, `None` → left
/// unmounted so the probe hits wiremock's own no-match 404.
async fn storage_index_bundle(
    body: Option<Vec<u8>>,
    bundle: Option<bool>,
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
    let template = match &body {
        Some(bytes) => ResponseTemplate::new(200).set_body_bytes(bytes.clone()),
        None => ResponseTemplate::new(404),
    };
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param("key", runs_index_key(SESSION_ID)))
        .respond_with(template)
        .mount(&server)
        .await;
    if let Some(present) = bundle {
        Mock::given(method("HEAD"))
            .and(path("/api/buckets/logs/objects/download"))
            .and(query_param(
                "key",
                format!("logs/{SESSION_ID}/latest.tar.gz"),
            ))
            .respond_with(ResponseTemplate::new(if present { 200 } else { 404 }))
            .mount(&server)
            .await;
    }
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

/// Build a two-run index (`r1` older, `r2` newer) as the stored bytes.
fn two_run_index() -> Vec<u8> {
    let first = runs::upsert_run(
        None,
        &LogRun {
            run_id: "r1".to_string(),
            started_at: "2026-07-20T10:00:00Z".to_string(),
            ended_at: Some("2026-07-20T10:30:00Z".to_string()),
        },
    );
    runs::upsert_run(
        Some(first.as_bytes()),
        &LogRun {
            run_id: "r2".to_string(),
            started_at: "2026-07-20T11:00:00Z".to_string(),
            ended_at: None,
        },
    )
    .into_bytes()
}

#[tokio::test]
async fn runs_are_listed_newest_first() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_index_bundle(Some(two_run_index()), None).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let arr = body.as_array().expect("runs array");
    assert_eq!(arr.len(), 2);
    // Newest (r2, 11:00) first, then r1.
    assert_eq!(arr[0]["run_id"], "r2");
    assert_eq!(arr[1]["run_id"], "r1");
    assert_eq!(arr[1]["ended_at"], "2026-07-20T10:30:00Z");
    // A live run omits `ended_at` (skip_serializing_if) — absent, not null.
    assert!(arr[0].get("ended_at").is_none(), "live run has no ended_at");
}

#[tokio::test]
async fn legacy_session_without_index_reports_a_synthetic_latest_run() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    // No run index (404) but the bundle EXISTS — a session bundled before per-run
    // separation. Both conditions are required for the synthetic run (#5765).
    let (storage, _s) = storage_index_bundle(None, Some(true)).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    let arr = body.as_array().expect("runs array");
    assert_eq!(arr.len(), 1, "one synthetic run for the legacy bundle");
    assert_eq!(arr[0]["run_id"], "latest");
}

#[tokio::test]
async fn runs_unauthorized_is_403() {
    // A valid user who is neither the author, nor listed, nor an admin.
    let gh = github_user_ok("mallory", 4004).await;
    let (storage, _s) = storage_index_bundle(Some(two_run_index()), None).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_mallory"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn runs_unknown_session_is_404() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_index_bundle(Some(two_run_index()), None).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(st, "/api/v1/logs/does-not-exist/runs", Some("gho_author")).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn runs_storage_not_configured_is_503() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let st = state(gh.uri(), None, log_config(&[], false), registry(&[]));

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

/// The #5765 regression: no run index AND no bundle is a session that has never
/// flushed a log. Advertising a synthetic `latest` there made the viewer request a
/// manifest that could not exist and render its 404 as a hard error.
#[tokio::test]
async fn a_session_with_no_index_and_no_bundle_reports_no_runs() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, _s) = storage_index_bundle(None, Some(false)).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = body_json(response).await;
    assert_eq!(
        body.as_array().expect("runs array").len(),
        0,
        "no phantom run when nothing has been flushed"
    );
}

/// A storage failure on the probe must NOT be reported as "no logs": that would
/// turn a transient outage into a confident, wrong empty state.
#[tokio::test]
async fn an_unreadable_bundle_probe_is_an_error_not_an_empty_list() {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
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
        .and(query_param("key", runs_index_key(SESSION_ID)))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("HEAD"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param(
            "key",
            format!("logs/{SESSION_ID}/latest.tar.gz"),
        ))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from("sa-secret".to_string()),
    };
    let storage = Arc::new(ChronoStorageClient::new(reqwest::Client::new(), config));
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let response = get(
        st,
        &format!("/api/v1/logs/{SESSION_ID}/runs"),
        Some("gho_author"),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
}
