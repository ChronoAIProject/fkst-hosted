//! Wiremock tests for `deliver_credential`: the intact single-file rewrite, the
//! container-restart full-bundle re-push (every file + the sentinel LAST), the
//! benign session-gone no-op, the surfaced upload failure, and the both-restarted
//! cache-miss single-file fallback.

use std::collections::BTreeMap;

use secrecy::{ExposeSecret, SecretString};
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::session_backend::{BackendError, DeliveryOutcome};

use super::super::backend_test_support::{
    backend, list_page, osb_config, sandbox_json, SESSION_ID,
};

const RESOLVE_PATH: &str = "/v1/sandboxes";
const FILE_INFO_PATH: &str = "/v1/sandboxes/sbx-1/proxy/44772/files/info";
const UPLOAD_PATH: &str = "/v1/sandboxes/sbx-1/proxy/44772/files/upload";

/// The `file_info` 200 body for a present github-token file (a path-keyed map).
fn token_present_body() -> serde_json::Value {
    json!({
        "/var/lib/fkst/creds/github-token": {
            "path": "/var/lib/fkst/creds/github-token", "size": 10, "mode": 400
        }
    })
}

/// Mount the `resolve_one` list response returning one sandbox `sbx-1` for the session.
async fn mount_resolve(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path(RESOLVE_PATH))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(list_page(json!([sandbox_json(
                "sbx-1",
                "Running",
                "2026-07-09T00:00:00Z",
                json!({ "fkst-session-id": SESSION_ID }),
            )]))),
        )
        .mount(server)
        .await;
}

/// The multipart upload bodies received, in arrival order (each embeds its target path).
async fn upload_bodies(server: &MockServer) -> Vec<String> {
    let reqs = server.received_requests().await.expect("requests");
    reqs.iter()
        .filter(|r| r.url.path() == UPLOAD_PATH)
        .map(|r| String::from_utf8_lossy(&r.body).into_owned())
        .collect()
}

fn full_bundle() -> BTreeMap<String, SecretString> {
    BTreeMap::from([
        (
            "github-token".to_string(),
            SecretString::from("ghs_old".to_string()),
        ),
        (
            "llm-api-key".to_string(),
            SecretString::from("sk-key".to_string()),
        ),
    ])
}

#[tokio::test]
async fn deliver_credential_rewrites_a_single_file_when_creds_are_intact() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    // The canary probe finds the token file present → single-file rewrite.
    Mock::given(method("GET"))
        .and(path(FILE_INFO_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_present_body()))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let outcome = backend(&server.uri(), osb_config())
        .deliver_credential_impl(
            SESSION_ID,
            "github-token",
            SecretString::from("ghs_new".to_string()),
        )
        .await
        .expect("delivered");
    assert_eq!(outcome, DeliveryOutcome::Delivered);

    let uploads = upload_bodies(&server).await;
    assert_eq!(uploads.len(), 1, "only the rotated file is rewritten");
    assert!(
        uploads[0].contains("/var/lib/fkst/creds/github-token") && uploads[0].contains("ghs_new")
    );
    assert!(uploads[0].contains(r#""mode":600"#));
    assert!(
        !uploads[0].contains(".creds-complete"),
        "no sentinel on the single-file path"
    );
}

#[tokio::test]
async fn deliver_credential_repushes_the_full_bundle_when_creds_were_wiped() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    // The canary probe 404s → the container restarted and lost its creds.
    Mock::given(method("GET"))
        .and(path(FILE_INFO_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let b = backend(&server.uri(), osb_config());
    // Pre-populate the cached bundle, as `ensure_session` would have.
    b.creds
        .lock()
        .unwrap()
        .insert(SESSION_ID.to_string(), full_bundle());

    let outcome = b
        .deliver_credential_impl(
            SESSION_ID,
            "github-token",
            SecretString::from("ghs_new".to_string()),
        )
        .await
        .expect("delivered");
    assert_eq!(outcome, DeliveryOutcome::Delivered);

    let uploads = upload_bodies(&server).await;
    // Every file re-pushed + the completeness sentinel LAST.
    assert_eq!(uploads.len(), 3, "two creds + the sentinel");
    assert!(
        uploads
            .iter()
            .any(|u| u.contains("/var/lib/fkst/creds/github-token") && u.contains("ghs_new")),
        "the rotated token is overlaid onto the re-pushed bundle"
    );
    assert!(
        uploads
            .iter()
            .any(|u| u.contains("/var/lib/fkst/creds/llm-api-key")),
        "the other cred is re-pushed too"
    );
    assert!(
        uploads[2].contains("/var/lib/fkst/creds/.creds-complete"),
        "the sentinel is uploaded LAST"
    );
    assert!(
        !uploads[0].contains(".creds-complete") && !uploads[1].contains(".creds-complete"),
        "the sentinel is not uploaded before the creds"
    );
    // The overlaid bundle is returned to the cache with the fresh token.
    let cached = b.creds.lock().unwrap();
    assert_eq!(
        cached
            .get(SESSION_ID)
            .and_then(|b| b.get("github-token"))
            .map(|s| s.expose_secret().to_string()),
        Some("ghs_new".to_string())
    );
}

#[tokio::test]
async fn deliver_credential_to_a_gone_session_is_benign() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(RESOLVE_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let outcome = backend(&server.uri(), osb_config())
        .deliver_credential_impl(
            SESSION_ID,
            "github-token",
            SecretString::from("ghs_new".to_string()),
        )
        .await
        .expect("benign");
    assert_eq!(outcome, DeliveryOutcome::SessionGone);
}

#[tokio::test]
async fn deliver_credential_surfaces_an_upload_failure() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    Mock::given(method("GET"))
        .and(path(FILE_INFO_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(token_present_body()))
        .mount(&server)
        .await;
    // The rewrite upload fails hard → the error is surfaced (the loop retries next tick).
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .deliver_credential_impl(
            SESSION_ID,
            "github-token",
            SecretString::from("ghs_new".to_string()),
        )
        .await
        .expect_err("upload failed");
    assert!(matches!(err, BackendError::Other(_)), "got {err:?}");
}

#[tokio::test]
async fn deliver_credential_delivers_only_the_single_file_when_the_cache_is_empty() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    // Creds wiped (404) AND the cache is empty (control plane also restarted).
    Mock::given(method("GET"))
        .and(path(FILE_INFO_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path(UPLOAD_PATH))
        .respond_with(ResponseTemplate::new(200))
        .mount(&server)
        .await;

    let outcome = backend(&server.uri(), osb_config())
        .deliver_credential_impl(
            SESSION_ID,
            "github-token",
            SecretString::from("ghs_new".to_string()),
        )
        .await
        .expect("delivered");
    assert_eq!(outcome, DeliveryOutcome::Delivered);

    let uploads = upload_bodies(&server).await;
    // Only the single rotated file — no full bundle (empty cache), no sentinel.
    assert_eq!(uploads.len(), 1);
    assert!(uploads[0].contains("/var/lib/fkst/creds/github-token"));
    assert!(uploads[0].contains(r#""mode":600"#));
    assert!(!uploads[0].contains(".creds-complete"));
}
