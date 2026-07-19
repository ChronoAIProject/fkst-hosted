//! Handler-level wiremock tests for the outcomes + blob endpoints, plus unit
//! tests of the pure media/kind/content-type helpers.

use axum::http::StatusCode;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::media::{
    content_type_for, disposition_header, extension, guess_kind, validate_blob_sha,
};
use super::*;
use crate::routes::canvas::test_support::{
    auth_headers, mount_app_token, test_app, test_state, viewer_user,
};

const VALID_TRIGGER_BODY: &str = "### Session Name\nsite\n\n### Packages\n\
acme/pkgs@main:packages/devloop\n\n### Work Label\nsite-build\n";

fn issue_json(number: i64, body: &str, labels: &[&str], state: &str) -> serde_json::Value {
    serde_json::json!({
        "number": number,
        "title": format!("issue-{number}"),
        "body": body,
        "state": state,
        "labels": labels.iter().map(|l| serde_json::json!({ "name": l })).collect::<Vec<_>>(),
        "user": { "login": "shining", "id": 9 },
        "html_url": format!("https://github.com/acme/site/issues/{number}"),
        "created_at": "2026-07-01T00:00:00Z",
        "updated_at": "2026-07-02T00:00:00Z",
        "closed_at": serde_json::Value::Null,
    })
}

async fn mount_installation_covering_site(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "installations": [
                { "id": 77, "account": { "login": "acme" }, "repository_selection": "all" }
            ]
        })))
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/user/installations/77/repositories"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 1,
            "repositories": [{ "name": "site", "owner": { "login": "acme" } }]
        })))
        .mount(server)
        .await;
}

async fn mount_trigger_and_work(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                5,
                VALID_TRIGGER_BODY,
                &["fkst-substrate-trigger"],
                "open"
            ),])),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "site-build"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(serde_json::json!([issue_json(
                8,
                "work",
                &["site-build"],
                "open"
            ),])),
        )
        .mount(server)
        .await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {
                "number": 12, "title": "devloop implementation for #8",
                "html_url": "https://github.com/acme/site/pull/12", "state": "closed",
                "merged_at": "2026-07-04T00:00:00Z",
                "user": { "login": "fkst-test[bot]" },
                "head": { "ref": "devloop/issue/acme/site/8/ready-1" }
            }
        ])))
        .mount(server)
        .await;
}

#[tokio::test]
async fn session_outcomes_groups_files_by_pr() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_trigger_and_work(&server).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/12/files"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([
            {"filename": "index.html", "status": "added", "additions": 10, "deletions": 0, "changes": 10, "sha": "shahtml"},
            {"filename": "assets/logo.png", "status": "added", "additions": 0, "deletions": 0, "changes": 0, "sha": "shapng"},
        ])))
        .mount(&server)
        .await;

    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some("fkst-test[bot]".to_string());

    let Json(view) = session_outcomes(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 5)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200");

    assert_eq!(view.owner, "acme");
    assert_eq!(view.trigger_issue, 5);
    assert_eq!(view.prs.len(), 1);
    let pr = &view.prs[0];
    assert_eq!(pr.number, 12);
    assert!(pr.merged);
    assert_eq!(pr.work_issue, Some(8));
    assert!(!pr.files_error);
    assert_eq!(pr.files.len(), 2);
    // A text file carries a size_hint; a binary image does not.
    let html = pr
        .files
        .iter()
        .find(|f| f.filename == "index.html")
        .unwrap();
    assert_eq!(html.kind, "text");
    assert_eq!(html.size_hint, Some(10));
    let png = pr
        .files
        .iter()
        .find(|f| f.filename == "assets/logo.png")
        .unwrap();
    assert_eq!(png.kind, "image");
    assert_eq!(png.size_hint, None);
}

#[tokio::test]
async fn session_outcomes_flags_files_error_but_keeps_the_pr() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    mount_trigger_and_work(&server).await;
    // The file listing fails — the PR still renders, flagged.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/pulls/12/files"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let mut state = test_state(&server.uri(), Some(test_app(&server.uri())));
    state.config.reconcile.github_bot_login = Some("fkst-test[bot]".to_string());

    let Json(view) = session_outcomes(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 5)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect("200 even when a file fetch fails");
    assert_eq!(view.prs.len(), 1);
    assert!(view.prs[0].files_error);
    assert!(view.prs[0].files.is_empty());
}

#[tokio::test]
async fn session_outcomes_unknown_trigger_is_404() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/issues"))
        .and(query_param("labels", "fkst-substrate-trigger"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!([])))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let err = session_outcomes(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 5)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("no such trigger is 404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn session_outcomes_repo_not_visible_is_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0, "installations": []
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let err = session_outcomes(
        State(state),
        Path(("acme".to_string(), "site".to_string(), 5)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("a repo the caller cannot see is 404");
    assert!(matches!(err, AppError::NotFound(_)), "got {err:?}");
}

#[tokio::test]
async fn session_outcomes_rejects_a_malformed_owner() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), None);
    let err = session_outcomes(
        State(state),
        Path(("bad owner".to_string(), "site".to_string(), 5)),
        viewer_user(),
        auth_headers(),
    )
    .await
    .expect_err("malformed owner is a 400");
    assert!(matches!(err, AppError::Validation(_)), "got {err:?}");
}

// ---- blob endpoint ----------------------------------------------------------

#[tokio::test]
async fn outcome_blob_streams_bytes_with_guessed_content_type() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/abc123"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"<svg/>".to_vec()))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let response = outcome_blob(
        State(state),
        Path(("acme".to_string(), "site".to_string(), "abc123".to_string())),
        Query(BlobQuery {
            name: Some("logo.svg".to_string()),
            download: None,
        }),
        viewer_user(),
        auth_headers(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .unwrap(),
        "image/svg+xml"
    );
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_DISPOSITION)
            .unwrap(),
        "inline"
    );
}

#[tokio::test]
async fn outcome_blob_download_sets_attachment_filename() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/def456"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(b"hello".to_vec()))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let response = outcome_blob(
        State(state),
        Path(("acme".to_string(), "site".to_string(), "def456".to_string())),
        Query(BlobQuery {
            name: Some("notes.txt".to_string()),
            download: Some(1),
        }),
        viewer_user(),
        auth_headers(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(axum::http::header::CONTENT_DISPOSITION)
            .unwrap(),
        "attachment; filename=\"notes.txt\""
    );
}

#[tokio::test]
async fn outcome_blob_over_cap_is_413() {
    let server = MockServer::start().await;
    mount_installation_covering_site(&server).await;
    mount_app_token(&server, "acme", "site", 77).await;
    // 200 bytes, over the test cap (64) — the transport rejects it up front.
    Mock::given(method("GET"))
        .and(path("/repos/acme/site/git/blobs/beef99"))
        .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0u8; 200]))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let response = outcome_blob(
        State(state),
        Path(("acme".to_string(), "site".to_string(), "beef99".to_string())),
        Query(BlobQuery {
            name: Some("video.mp4".to_string()),
            download: None,
        }),
        viewer_user(),
        auth_headers(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);
}

#[tokio::test]
async fn outcome_blob_rejects_a_non_hex_sha() {
    let server = MockServer::start().await;
    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let response = outcome_blob(
        State(state),
        Path((
            "acme".to_string(),
            "site".to_string(),
            "../etc/passwd".to_string(),
        )),
        Query(BlobQuery {
            name: None,
            download: None,
        }),
        viewer_user(),
        auth_headers(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn outcome_blob_repo_not_visible_is_404() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/user/installations"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "total_count": 0, "installations": []
        })))
        .mount(&server)
        .await;

    let state = test_state(&server.uri(), Some(test_app(&server.uri())));
    let response = outcome_blob(
        State(state),
        Path(("acme".to_string(), "site".to_string(), "abc123".to_string())),
        Query(BlobQuery {
            name: None,
            download: None,
        }),
        viewer_user(),
        auth_headers(),
    )
    .await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ---- pure helpers -----------------------------------------------------------

#[test]
fn guess_kind_covers_the_media_families() {
    assert_eq!(guess_kind("a/b/c.png"), "image");
    assert_eq!(guess_kind("clip.MP4"), "video");
    assert_eq!(guess_kind("song.mp3"), "audio");
    assert_eq!(guess_kind("src/main.rs"), "text");
    assert_eq!(guess_kind("README"), "text");
    assert_eq!(guess_kind("archive.zip"), "binary");
    assert_eq!(guess_kind("font.woff2"), "binary");
}

#[test]
fn content_type_for_maps_extensions() {
    assert_eq!(content_type_for("x.mp4"), "video/mp4");
    assert_eq!(content_type_for("x.png"), "image/png");
    assert_eq!(content_type_for("x.jpeg"), "image/jpeg");
    assert_eq!(content_type_for("x.svg"), "image/svg+xml");
    assert_eq!(content_type_for("x.mp3"), "audio/mpeg");
    assert_eq!(content_type_for("x.md"), "text/plain; charset=utf-8");
    assert_eq!(content_type_for("README"), "text/plain; charset=utf-8");
    assert_eq!(content_type_for("x.bin"), "application/octet-stream");
}

#[test]
fn disposition_header_sanitizes_and_switches_mode() {
    assert_eq!(disposition_header("a.txt", false), "inline");
    assert_eq!(
        disposition_header("dir/a.txt", true),
        "attachment; filename=\"a.txt\""
    );
    // A quote-injection attempt is stripped, not honored.
    assert_eq!(
        disposition_header("a\".txt", true),
        "attachment; filename=\"a.txt\""
    );
    assert_eq!(disposition_header("", true), "attachment");
}

#[test]
fn validate_blob_sha_accepts_hex_only() {
    validate_blob_sha("deadbeef1234").expect("hex ok");
    validate_blob_sha(&"a".repeat(64)).expect("sha256 length ok");
    for bad in ["", "../x", "zzz", &"a".repeat(65)] {
        assert!(validate_blob_sha(bad).is_err(), "{bad:?} must be rejected");
    }
}

#[test]
fn extensionless_dotfile_has_no_extension() {
    // `.gitignore` is a dotfile whose whole name is the "extension" — treat as
    // extensionless (→ text), never as a `gitignore`-typed binary.
    assert_eq!(extension(".gitignore"), None);
    assert_eq!(guess_kind(".gitignore"), "text");
}
