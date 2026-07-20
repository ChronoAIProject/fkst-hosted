//! Tests for the log-bundle sink: the fake records puts, `from_creds` fails closed
//! without the storage SA cred files, and the chrono-storage sink maps an upload
//! failure to a leak-free error. Split into a sibling file so `sink.rs` stays under
//! the module-size cap.

use super::*;

use crate::session_spec::creds::CredsLayout;
use crate::storage::ChronoStorageClient;

use secrecy::SecretString;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SA_TOKEN: &str = "sa-token-xyz";
const SA_SECRET: &str = "writer-client-secret";

#[tokio::test]
async fn fake_sink_records_every_put() {
    let fake = FakeSink::default();
    fake.put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz-a"))
        .await
        .expect("ok");
    fake.put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz-b"))
        .await
        .expect("ok");
    let calls = fake.calls();
    assert_eq!(calls.len(), 2);
    assert_eq!(calls[0].0, "logs/s1/latest.tar.gz");
    assert_eq!(calls[1].1.as_ref(), b"gz-b");
}

#[tokio::test]
async fn fake_sink_can_be_programmed_to_fail() {
    let fake = FakeSink {
        fail: true,
        ..Default::default()
    };
    let err = fake
        .put("k", Bytes::from_static(b"x"))
        .await
        .expect_err("programmed failure");
    assert!(matches!(err, SinkError::Upload(_)));
    // Even a failing put is still recorded (the collector saw the attempt).
    assert_eq!(fake.calls().len(), 1);
    // A programmed failure also fails `get`.
    let err = fake.get("k").await.expect_err("programmed get failure");
    assert!(matches!(err, SinkError::Upload(_)));
}

#[tokio::test]
async fn fake_sink_fail_key_contains_scopes_failure_to_matching_keys() {
    // Permanent: only keys containing "runs.json" fail; other keys still succeed.
    let fake = FakeSink {
        fail_key_contains: Some("runs.json".to_string()),
        ..Default::default()
    };
    // A non-matching key succeeds and records normally.
    fake.put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz"))
        .await
        .expect("non-matching put ok");
    assert_eq!(
        fake.get("logs/s1/latest.tar.gz").await.expect("ok"),
        Some(Bytes::from_static(b"gz"))
    );
    // The matching index key fails on both put and get, every time.
    assert!(fake
        .put("logs/s1/runs.json", Bytes::from_static(b"[]"))
        .await
        .is_err());
    assert!(fake.get("logs/s1/runs.json").await.is_err());
    assert!(fake.get("logs/s1/runs.json").await.is_err());
}

#[tokio::test]
async fn fake_sink_fail_key_remaining_is_transient() {
    // Transient: fail only the first matching op, then let matching ops succeed —
    // modelling an outage that clears (so a lost write can later be recovered).
    let fake = FakeSink {
        fail_key_contains: Some("runs.json".to_string()),
        fail_key_remaining: std::sync::Arc::new(std::sync::Mutex::new(Some(1))),
        ..Default::default()
    };
    assert!(
        fake.get("logs/s1/runs.json").await.is_err(),
        "first matching op fails"
    );
    // Budget exhausted → matching ops now succeed.
    fake.put("logs/s1/runs.json", Bytes::from_static(b"[]"))
        .await
        .expect("second matching op succeeds");
    assert_eq!(
        fake.get("logs/s1/runs.json").await.expect("ok"),
        Some(Bytes::from_static(b"[]"))
    );
}

#[tokio::test]
async fn fake_sink_get_returns_the_last_put_value_and_none_for_absent() {
    let fake = FakeSink::default();
    // Absent key → None.
    assert!(fake.get("logs/s1/runs.json").await.expect("ok").is_none());
    // After a put, get returns the last-put value for that key.
    fake.put("logs/s1/runs.json", Bytes::from_static(b"v1"))
        .await
        .expect("ok");
    fake.put("logs/s1/runs.json", Bytes::from_static(b"v2"))
        .await
        .expect("ok");
    assert_eq!(
        fake.get("logs/s1/runs.json").await.expect("ok"),
        Some(Bytes::from_static(b"v2"))
    );
    // A different key is still absent.
    assert!(fake.get("logs/s1/other").await.expect("ok").is_none());
}

/// Write the five storage SA cred files into a fresh creds dir.
fn write_storage_creds(dir: &std::path::Path) {
    std::fs::write(
        dir.join("storage-base-url"),
        "https://storage.example/proxy\n",
    )
    .expect("base");
    std::fs::write(dir.join("storage-bucket"), "logs\n").expect("bucket");
    std::fs::write(
        dir.join("storage-token-url"),
        "https://nyx.example/oauth/token",
    )
    .expect("tok");
    std::fs::write(dir.join("storage-client-id"), "writer-client").expect("id");
    std::fs::write(dir.join("storage-client-secret"), SA_SECRET).expect("secret");
}

#[test]
fn from_creds_is_none_when_the_storage_sa_is_not_mounted() {
    let dir = tempfile::tempdir().expect("dir");
    // Only some of the files present → fail closed (None), no uploader.
    std::fs::write(
        dir.path().join("storage-base-url"),
        "https://storage.example",
    )
    .expect("base");
    let layout = CredsLayout::new(dir.path());
    assert!(ChronoStorageSink::from_creds(&layout).is_none());
}

#[test]
fn from_creds_builds_a_sink_when_all_files_are_present() {
    let dir = tempfile::tempdir().expect("dir");
    write_storage_creds(dir.path());
    let layout = CredsLayout::new(dir.path());
    assert!(ChronoStorageSink::from_creds(&layout).is_some());
}

/// Build a chrono-storage sink whose token endpoint mints `SA_TOKEN` and whose
/// object endpoint returns `status`, so the error mapping is exercised end to end.
async fn sink_over_status(status: u16) -> (ChronoStorageSink, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": SA_TOKEN,
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/buckets/logs/objects"))
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;

    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "writer-client".to_string(),
        nyxid_client_secret: SecretString::from(SA_SECRET.to_string()),
    };
    (
        ChronoStorageSink::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

#[tokio::test]
async fn put_maps_a_non_2xx_upload_to_a_sink_error_without_leaking_secrets() {
    let (sink, _server) = sink_over_status(403).await;
    let err = sink
        .put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz"))
        .await
        .expect_err("403 must error");
    assert!(matches!(err, SinkError::Upload(_)));
    let rendered = format!("{err} {err:?}");
    // The storage SA secret + the minted token never ride the error.
    assert!(!rendered.contains(SA_SECRET), "leaked secret: {rendered}");
    assert!(!rendered.contains(SA_TOKEN), "leaked token: {rendered}");
    // What DOES survive is the leak-free status code.
    assert!(rendered.contains("403"), "status carried: {rendered}");
}

#[tokio::test]
async fn put_succeeds_on_a_2xx_upload() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": SA_TOKEN,
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path("/api/buckets/logs/objects"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "url": "https://cdn.example/logs/s1/latest.tar.gz" }
        })))
        .mount(&server)
        .await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "writer-client".to_string(),
        nyxid_client_secret: SecretString::from(SA_SECRET.to_string()),
    };
    let sink = ChronoStorageSink::new(ChronoStorageClient::new(reqwest::Client::new(), config));
    sink.put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz"))
        .await
        .expect("upload succeeds");
}

/// Build a chrono-storage sink whose token endpoint mints `SA_TOKEN` and whose
/// object DOWNLOAD endpoint responds with `status` (+ optional body), so the `get`
/// mapping is exercised end to end.
async fn sink_get_over(status: u16, body: Option<&[u8]>) -> (ChronoStorageSink, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": SA_TOKEN,
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    let mut template = ResponseTemplate::new(status);
    if let Some(bytes) = body {
        template = template.set_body_bytes(bytes.to_vec());
    }
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .respond_with(template)
        .mount(&server)
        .await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: "logs".to_string(),
        nyxid_token_url: format!("{}/oauth/token", server.uri()),
        nyxid_client_id: "writer-client".to_string(),
        nyxid_client_secret: SecretString::from(SA_SECRET.to_string()),
    };
    (
        ChronoStorageSink::new(ChronoStorageClient::new(reqwest::Client::new(), config)),
        server,
    )
}

#[tokio::test]
async fn get_maps_404_to_none() {
    let (sink, _server) = sink_get_over(404, None).await;
    let got = sink
        .get("logs/s1/runs.json")
        .await
        .expect("404 is not an error");
    assert!(got.is_none(), "a missing object reads as None");
}

#[tokio::test]
async fn get_returns_the_object_bytes_on_200() {
    let (sink, _server) = sink_get_over(200, Some(b"[\n]\n")).await;
    let got = sink.get("logs/s1/runs.json").await.expect("ok");
    assert_eq!(got.as_deref(), Some(&b"[\n]\n"[..]));
}

#[tokio::test]
async fn get_maps_other_status_to_a_leak_free_error() {
    let (sink, _server) = sink_get_over(500, None).await;
    let err = sink
        .get("logs/s1/runs.json")
        .await
        .expect_err("500 must error");
    assert!(matches!(err, SinkError::Upload(_)));
    let rendered = format!("{err} {err:?}");
    assert!(!rendered.contains(SA_SECRET), "leaked secret: {rendered}");
    assert!(!rendered.contains(SA_TOKEN), "leaked token: {rendered}");
    assert!(rendered.contains("500"), "status carried: {rendered}");
}
