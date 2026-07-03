//! API-mode (Bearer-token) integration tests for the log-download endpoint:
//! identity resolution (valid user → identity, rejected token → 401), the three-tier
//! authorization matrix (author / per-issue / admin allow, non-member deny → 403),
//! and the not-found / unconfigured cases. Fixtures live in [`super::test_support`];
//! the browser-mode + secret-hygiene suites live in [`super::tests_browser`].

use axum::http::{header, StatusCode};

use super::test_support::*;

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
async fn api_mode_global_admin_grants() {
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
