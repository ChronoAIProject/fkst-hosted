//! Handler tests for the in-bundle log viewer, driving the handlers directly
//! (a constructed [`GithubUser`], no `/user` round-trip) against a wiremock
//! chrono-storage that serves an in-memory `tar.gz` fixture. Covers the manifest
//! listing + labels, full + tailed file reads, the traversal/unknown-path 404,
//! and the shared authz (unauthorized → 403, unknown session → 404).

use std::sync::Arc;

use axum::extract::{Path, Query, State};
use flate2::write::GzEncoder;
use flate2::Compression;
use secrecy::SecretString;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::*;
use crate::github_identity::GithubUser;
use crate::routes::logs::test_support::{log_config, registry, state, AUTHOR_ID, SESSION_ID};
use crate::storage::{ChronoStorageClient, ChronoStorageConfig};

/// A codex log with several lines so the tail can snap to a line boundary.
const CODEX_CONTENT: &[u8] = b"L1\nL2\nL3\nL4\n";

fn append(builder: &mut tar::Builder<&mut GzEncoder<Vec<u8>>>, name: &str, data: &[u8]) {
    let mut header = tar::Header::new_gnu();
    header.set_size(data.len() as u64);
    header.set_mode(0o644);
    header.set_cksum();
    builder
        .append_data(&mut header, name, data)
        .expect("append entry");
}

/// A minimal redacted-bundle fixture in the fixed collector layout.
fn make_bundle() -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut builder = tar::Builder::new(&mut encoder);
        append(&mut builder, "fkst-hosted/driver.log", b"driver line\n");
        append(
            &mut builder,
            "fkst-substrate/codex/codex.log",
            CODEX_CONTENT,
        );
        append(&mut builder, "README.md", b"# readme\n");
        append(&mut builder, "meta.json", b"{}\n");
        builder.finish().expect("finish tar");
    }
    encoder.finish().expect("finish gzip")
}

/// A chrono-storage mock that serves `bundle` for SESSION_ID's log key.
async fn storage_serving(bundle: Vec<u8>) -> (Arc<ChronoStorageClient>, MockServer) {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/oauth/token"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "access_token": "sa-token",
            "expires_in": 3600,
        })))
        .mount(&server)
        .await;
    let key = format!("logs/{SESSION_ID}/latest.tar.gz");
    Mock::given(method("GET"))
        .and(path("/api/buckets/logs/objects/download"))
        .and(query_param("key", key))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(bundle))
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

fn author() -> GithubUser {
    GithubUser {
        login: "alice".to_string(),
        id: AUTHOR_ID,
    }
}

#[tokio::test]
async fn manifest_lists_files_with_labels() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let Json(manifest) = log_manifest(State(st), Path(SESSION_ID.to_string()), author())
        .await
        .expect("200");
    assert_eq!(manifest.session_id, SESSION_ID);
    assert!(!manifest.generated_at.is_empty());
    let by_path = |p: &str| manifest.files.iter().find(|f| f.path == p).cloned();

    let driver = by_path("fkst-hosted/driver.log").expect("driver present");
    assert_eq!(driver.label, "Driver");
    assert_eq!(driver.size, "driver line\n".len() as i64);

    assert_eq!(
        by_path("fkst-substrate/codex/codex.log").unwrap().label,
        "Codex"
    );
    assert_eq!(by_path("README.md").unwrap().label, "README");
    assert_eq!(by_path("meta.json").unwrap().label, "Meta");
    // Path-sorted for a stable render.
    let paths: Vec<&str> = manifest.files.iter().map(|f| f.path.as_str()).collect();
    let mut sorted = paths.clone();
    sorted.sort_unstable();
    assert_eq!(paths, sorted);
}

#[tokio::test]
async fn file_returns_full_content_untruncated() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let Json(file) = log_file(
        State(st),
        Path(SESSION_ID.to_string()),
        Query(LogFileQuery {
            path: "fkst-substrate/codex/codex.log".to_string(),
            tail_bytes: None,
        }),
        author(),
    )
    .await
    .expect("200");
    assert_eq!(file.content, "L1\nL2\nL3\nL4\n");
    assert_eq!(file.total_bytes, CODEX_CONTENT.len() as i64);
    assert_eq!(file.returned_bytes, CODEX_CONTENT.len() as i64);
    assert!(!file.truncated);
}

#[tokio::test]
async fn file_tail_snaps_to_a_line_boundary() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    // Last 7 bytes of "L1\nL2\nL3\nL4\n" starts mid-line; snapping forward drops
    // the partial leading line, leaving "L3\nL4\n".
    let Json(file) = log_file(
        State(st),
        Path(SESSION_ID.to_string()),
        Query(LogFileQuery {
            path: "fkst-substrate/codex/codex.log".to_string(),
            tail_bytes: Some(7),
        }),
        author(),
    )
    .await
    .expect("200");
    assert_eq!(file.content, "L3\nL4\n");
    assert_eq!(file.returned_bytes, 6);
    assert_eq!(file.total_bytes, 12);
    assert!(file.truncated);
}

#[tokio::test]
async fn file_tail_larger_than_file_returns_all() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let Json(file) = log_file(
        State(st),
        Path(SESSION_ID.to_string()),
        Query(LogFileQuery {
            path: "fkst-substrate/codex/codex.log".to_string(),
            tail_bytes: Some(9999),
        }),
        author(),
    )
    .await
    .expect("200");
    assert_eq!(file.content, "L1\nL2\nL3\nL4\n");
    assert!(!file.truncated);
}

#[tokio::test]
async fn file_unknown_path_is_404() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let err = log_file(
        State(st),
        Path(SESSION_ID.to_string()),
        Query(LogFileQuery {
            path: "../../etc/passwd".to_string(),
            tail_bytes: None,
        }),
        author(),
    )
    .await
    .expect_err("a traversal / unknown path matches no entry → 404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn manifest_unauthorized_is_403() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    // author is AUTHOR_ID; this caller is someone else, not listed, not admin.
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&["bob"]),
    );

    let stranger = GithubUser {
        login: "mallory".to_string(),
        id: 4004,
    };
    let err = log_manifest(State(st), Path(SESSION_ID.to_string()), stranger)
        .await
        .expect_err("unauthorized caller → 403");
    assert!(matches!(err, AppError::Forbidden(_)), "got {err:?}");
}

#[tokio::test]
async fn manifest_unknown_session_is_404() {
    let (storage, _s) = storage_serving(make_bundle()).await;
    let st = state(
        "https://unused".to_string(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );

    let err = log_manifest(State(st), Path("does-not-exist".to_string()), author())
        .await
        .expect_err("unknown session → 404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn manifest_storage_not_configured_is_503() {
    let st = state(
        "https://unused".to_string(),
        None,
        log_config(&[], false),
        registry(&[]),
    );
    let err = log_manifest(State(st), Path(SESSION_ID.to_string()), author())
        .await
        .expect_err("no storage → 503");
    assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
}

// ---- pure helpers -----------------------------------------------------------

#[test]
fn classify_bundle_path_maps_the_layout() {
    assert_eq!(classify_bundle_path("fkst-hosted/driver.log"), "Driver");
    assert_eq!(
        classify_bundle_path("fkst-substrate/framework/supervise.log"),
        "Supervise"
    );
    assert_eq!(
        classify_bundle_path("fkst-substrate/codex/codex.log"),
        "Codex"
    );
    assert_eq!(classify_bundle_path("fkst-substrate/etc/misc.log"), "Misc");
    assert_eq!(classify_bundle_path("README.md"), "README");
    assert_eq!(classify_bundle_path("meta.json"), "Meta");
}

#[test]
fn tail_returns_whole_file_when_unbounded() {
    let (out, truncated) = tail(b"abc\ndef\n", None);
    assert_eq!(out, b"abc\ndef\n");
    assert!(!truncated);
}
