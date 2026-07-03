//! wiremock tests for the chrono-storage client: each method builds the right
//! method/path/query and sends the bearer header; the two-step download resolves
//! a presigned URL then fetches the bytes with no auth; non-2xx maps to a
//! status error; and neither the token nor the client secret leaks into an error.

use axum::body::Bytes;
use secrecy::SecretString;
use wiremock::matchers::{body_json, header, header_exists, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::storage::config::ChronoStorageConfig;

const BUCKET: &str = "logs";
const TOKEN_PATH: &str = "/oauth/token";
/// The bearer token the mounted token endpoint always mints, so object-endpoint
/// mocks can assert the exact `Authorization` header.
const SA_TOKEN: &str = "sa-token-xyz";
const SA_SECRET: &str = "sa-client-secret";

/// Start a mock server with the token endpoint mounted, and a client pointed at
/// it. Object-endpoint mocks are added per test.
async fn client_and_server() -> (ChronoStorageClient, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(TOKEN_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": SA_TOKEN,
            "expires_in": 3600,
            "token_type": "Bearer",
        })))
        .mount(&server)
        .await;

    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: BUCKET.to_string(),
        nyxid_token_url: format!("{}{TOKEN_PATH}", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from(SA_SECRET.to_string()),
        writer_client_id: None,
        writer_client_secret: None,
    };
    let client = ChronoStorageClient::new(reqwest::Client::new(), config);
    (client, server)
}

fn bearer() -> String {
    format!("Bearer {SA_TOKEN}")
}

#[tokio::test]
async fn upload_posts_bytes_with_key_content_type_and_bearer() {
    let (client, server) = client_and_server().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/buckets/{BUCKET}/objects")))
        .and(query_param("key", "sessions/42/run.log"))
        .and(query_param("contentType", "text/plain"))
        .and(header("authorization", bearer().as_str()))
        .and(header("content-type", "text/plain"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "url": "https://cdn.example/logs/sessions/42/run.log" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = client
        .upload(
            "sessions/42/run.log",
            Bytes::from_static(b"hello world"),
            "text/plain",
        )
        .await
        .expect("upload succeeds");
    assert_eq!(url, "https://cdn.example/logs/sessions/42/run.log");
}

#[tokio::test]
async fn upload_maps_non_2xx_to_status_error() {
    let (client, server) = client_and_server().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/buckets/{BUCKET}/objects")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = client
        .upload("k", Bytes::from_static(b"x"), "text/plain")
        .await
        .expect_err("500 must error");
    assert!(
        matches!(err, StorageError::Status { status: 500 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn download_resolves_presigned_url_then_fetches_bytes() {
    let (client, server) = client_and_server().await;
    let signed = format!("{}/signed/blob?sig=abc123", server.uri());

    // Step 1: presigned-url resolution (authenticated).
    Mock::given(method("GET"))
        .and(path(format!("/api/buckets/{BUCKET}/presigned-url")))
        .and(query_param("key", "sessions/42/run.log"))
        .and(header("authorization", bearer().as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "presignedUrl": signed, "expiresAt": "2999-01-01T00:00:00Z" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    // Step 2: the direct fetch of the signed URL carries NO auth header.
    Mock::given(method("GET"))
        .and(path("/signed/blob"))
        .and(query_param("sig", "abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"BLOB-DATA".to_vec()))
        .expect(1)
        .mount(&server)
        .await;

    let bytes = client
        .download("sessions/42/run.log")
        .await
        .expect("download succeeds");
    assert_eq!(bytes.as_ref(), b"BLOB-DATA");
}

#[tokio::test]
async fn presigned_get_url_requests_expiry_and_returns_the_signed_url() {
    let (client, server) = client_and_server().await;
    let signed = "https://cdn.example/signed/blob?sig=xyz".to_string();
    Mock::given(method("GET"))
        .and(path(format!("/api/buckets/{BUCKET}/presigned-url")))
        .and(query_param("key", "logs/sess-1/latest.tar.gz"))
        // The requested 900s TTL rides the `expiresIn` query param.
        .and(query_param("expiresIn", "900"))
        .and(header("authorization", bearer().as_str()))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "presignedUrl": signed, "expiresAt": "2999-01-01T00:00:00Z" }
        })))
        .expect(1)
        .mount(&server)
        .await;

    let url = client
        .presigned_get_url("logs/sess-1/latest.tar.gz", 900)
        .await
        .expect("presign succeeds");
    assert_eq!(url, "https://cdn.example/signed/blob?sig=xyz");
}

#[tokio::test]
async fn presigned_get_url_maps_missing_object_to_status_404() {
    let (client, server) = client_and_server().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/buckets/{BUCKET}/presigned-url")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client
        .presigned_get_url("logs/missing/latest.tar.gz", 900)
        .await
        .expect_err("404 must error");
    assert!(
        matches!(err, StorageError::Status { status: 404 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn download_maps_presigned_404_to_status_error() {
    let (client, server) = client_and_server().await;
    Mock::given(method("GET"))
        .and(path(format!("/api/buckets/{BUCKET}/presigned-url")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client
        .download("missing")
        .await
        .expect_err("404 must error");
    assert!(
        matches!(err, StorageError::Status { status: 404 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn download_maps_signed_url_5xx_to_status_error() {
    let (client, server) = client_and_server().await;
    let signed = format!("{}/signed/blob", server.uri());
    Mock::given(method("GET"))
        .and(path(format!("/api/buckets/{BUCKET}/presigned-url")))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "data": { "presignedUrl": signed, "expiresAt": "2999-01-01T00:00:00Z" }
        })))
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path("/signed/blob"))
        .respond_with(ResponseTemplate::new(502))
        .mount(&server)
        .await;

    let err = client
        .download("k")
        .await
        .expect_err("signed 502 must error");
    assert!(
        matches!(err, StorageError::Status { status: 502 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn delete_sends_delete_with_key_and_bearer() {
    let (client, server) = client_and_server().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/buckets/{BUCKET}/objects")))
        .and(query_param("key", "sessions/42/run.log"))
        .and(header("authorization", bearer().as_str()))
        .respond_with(ResponseTemplate::new(204))
        .expect(1)
        .mount(&server)
        .await;

    client
        .delete("sessions/42/run.log")
        .await
        .expect("delete succeeds");
}

#[tokio::test]
async fn delete_maps_non_2xx_to_status_error() {
    let (client, server) = client_and_server().await;
    Mock::given(method("DELETE"))
        .and(path(format!("/api/buckets/{BUCKET}/objects")))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client.delete("k").await.expect_err("404 must error");
    assert!(
        matches!(err, StorageError::Status { status: 404 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn copy_posts_source_and_dest_keys_as_json() {
    let (client, server) = client_and_server().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/buckets/{BUCKET}/objects/copy")))
        .and(header("authorization", bearer().as_str()))
        .and(body_json(serde_json::json!({
            "sourceKey": "a/old.log",
            "destKey": "b/new.log",
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": {} })))
        .expect(1)
        .mount(&server)
        .await;

    client
        .copy("a/old.log", "b/new.log")
        .await
        .expect("copy succeeds");
}

#[tokio::test]
async fn copy_maps_non_2xx_to_status_error() {
    let (client, server) = client_and_server().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/buckets/{BUCKET}/objects/copy")))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = client.copy("a", "b").await.expect_err("500 must error");
    assert!(
        matches!(err, StorageError::Status { status: 500 }),
        "got {err:?}"
    );
}

#[tokio::test]
async fn bucket_ok_is_true_on_2xx_with_bearer() {
    let (client, server) = client_and_server().await;
    Mock::given(method("GET"))
        .and(path("/api/buckets"))
        .and(header_exists("authorization"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({ "data": [] })))
        .mount(&server)
        .await;

    assert!(client.bucket_ok().await);
}

#[tokio::test]
async fn bucket_ok_is_false_on_5xx() {
    let (client, server) = client_and_server().await;
    Mock::given(method("GET"))
        .and(path("/api/buckets"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    assert!(!client.bucket_ok().await);
}

#[tokio::test]
async fn bucket_ok_is_false_when_token_mint_fails() {
    // No token endpoint mounted at this path: the mint 404s, so readiness is
    // false rather than an error.
    let server = MockServer::start().await;
    let config = ChronoStorageConfig {
        base_url: server.uri(),
        bucket: BUCKET.to_string(),
        nyxid_token_url: format!("{}/no-such-token", server.uri()),
        nyxid_client_id: "sa-client".to_string(),
        nyxid_client_secret: SecretString::from(SA_SECRET.to_string()),
        writer_client_id: None,
        writer_client_secret: None,
    };
    let client = ChronoStorageClient::new(reqwest::Client::new(), config);
    assert!(!client.bucket_ok().await);
}

#[tokio::test]
async fn errors_never_leak_the_token_or_client_secret() {
    let (client, server) = client_and_server().await;
    Mock::given(method("POST"))
        .and(path(format!("/api/buckets/{BUCKET}/objects")))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let err = client
        .upload("k", Bytes::from_static(b"x"), "text/plain")
        .await
        .expect_err("403 must error");
    let rendered = format!("{err} {err:?}");
    assert!(
        !rendered.contains(SA_TOKEN),
        "error leaked the token: {rendered}"
    );
    assert!(
        !rendered.contains(SA_SECRET),
        "error leaked the client secret: {rendered}"
    );

    // The whole client Debug must also stay clean.
    let debug = format!("{client:?}");
    assert!(!debug.contains(SA_SECRET), "Debug leaked secret: {debug}");
}
