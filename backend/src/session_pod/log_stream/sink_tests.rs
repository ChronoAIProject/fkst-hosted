//! Tests for the log-bundle sink: the fake records puts, `from_creds` fails closed
//! without the write-only SA files, and the chrono-storage sink maps an upload
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
}

/// Write the five write-only SA cred files into a fresh creds dir.
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
fn from_creds_is_none_when_the_write_only_sa_is_not_mounted() {
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
        writer_client_id: None,
        writer_client_secret: None,
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
    // The write-only SA secret + the minted token never ride the error.
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
        writer_client_id: None,
        writer_client_secret: None,
    };
    let sink = ChronoStorageSink::new(ChronoStorageClient::new(reqwest::Client::new(), config));
    sink.put("logs/s1/latest.tar.gz", Bytes::from_static(b"gz"))
        .await
        .expect("upload succeeds");
}
