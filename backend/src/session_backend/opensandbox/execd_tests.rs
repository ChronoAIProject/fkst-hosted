//! Transport-layer tests for [`ExecdClient`] (sibling `#[path]` module, mirrors the
//! `lifecycle_tests.rs` wiremock style). EVERY mock asserts BOTH auth headers
//! (`OPEN-SANDBOX-API-KEY` + `X-EXECD-ACCESS-TOKEN`) AND that the request path rides
//! the `/v1/sandboxes/{id}/proxy/44772` lifecycle-proxy prefix, so no verb can drop
//! a header or bypass the proxy.

use wiremock::matchers::{body_string_contains, header, method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::lifecycle::API_KEY_HEADER;
use super::*;
use super::{EXECD_TOKEN_HEADER, TAIL_CURSOR_HEADER};

const API_KEY: &str = "osb_secret_key_abc123";
const EXECD_TOKEN: &str = "execd_tok_deadbeef42";
const SANDBOX_ID: &str = "sbx-1";

fn client(base: &str) -> ExecdClient {
    ExecdClient::new(
        reqwest::Url::parse(base).expect("base url"),
        SecretString::from(API_KEY.to_string()),
        SANDBOX_ID.to_string(),
        SecretString::from(EXECD_TOKEN.to_string()),
        reqwest::Client::new(),
    )
}

/// The exact proxied path for a daemon-relative `execd_path`, asserting the
/// `/v1/sandboxes/{id}/proxy/44772` prefix inline.
fn proxy(execd_path: &str) -> String {
    format!("/v1/sandboxes/{SANDBOX_ID}/proxy/44772{execd_path}")
}

#[tokio::test]
async fn upload_file_sends_both_headers_octal_digit_mode_and_named_parts() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/files/upload")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        // Real bits 0o400 must reach the wire as the octal-DIGITS integer 400.
        .and(body_string_contains(r#""mode":400"#))
        .and(body_string_contains(r#""path":"/app/secret.txt""#))
        // Both multipart parts carry filenames (the server reads each as a file part).
        .and(body_string_contains("filename=\"metadata.json\""))
        .and(body_string_contains("filename=\"secret.txt\""))
        .respond_with(ResponseTemplate::new(200))
        .expect(1)
        .mount(&server)
        .await;

    client(&server.uri())
        .upload_file("/app/secret.txt", b"top-secret-bytes", 0o400)
        .await
        .expect("uploaded");
}

#[tokio::test]
async fn file_info_returns_the_entry_for_the_queried_path() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/files/info")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .and(query_param("path", "/app/x.txt"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "/app/x.txt": {
                "path": "/app/x.txt",
                "type": "file",
                "size": 10,
                "modified_at": "2026-07-09T00:00:00Z",
                "mode": 420
            }
        })))
        .mount(&server)
        .await;

    let info = client(&server.uri())
        .file_info("/app/x.txt")
        .await
        .expect("file info");
    assert_eq!(info.path, "/app/x.txt");
    assert_eq!(info.size, 10);
    // `mode` is the RAW wire octal-digits integer (420 = 0o644); fkst does NOT convert it.
    assert_eq!(info.mode, 420);
    assert_eq!(info.r#type.as_deref(), Some("file"));
}

#[tokio::test]
async fn file_info_empty_map_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/files/info")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({})))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .file_info("/app/gone.txt")
        .await
        .expect_err("empty map");
    assert!(matches!(err, OsbError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn file_info_404_is_not_found() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/files/info")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .file_info("/app/missing.txt")
        .await
        .expect_err("404");
    assert!(matches!(err, OsbError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn run_command_extracts_id_from_the_init_frame() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/command")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(
            // A raw event-stream frame (JSON + a blank line), exactly as execd emits.
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("{\"type\":\"init\",\"text\":\"cmd-abc123\",\"timestamp\":1}\n\n"),
        )
        .expect(1)
        .mount(&server)
        .await;

    let cmd = client(&server.uri())
        .run_command("echo hi", Some(5_000), false)
        .await
        .expect("launched");
    assert_eq!(cmd.id, "cmd-abc123");
}

#[tokio::test]
async fn run_command_without_init_frame_is_api_error() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/command")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string("{\"type\":\"stdout\",\"text\":\"hello\"}\n\n"),
        )
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .run_command("echo hi", None, true)
        .await
        .expect_err("no init frame");
    match err {
        OsbError::Api { status, message } => {
            assert_eq!(status, 200);
            // The raw stream body must NOT be echoed into the error message.
            assert!(!message.contains("hello"), "message leaked body: {message}");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[tokio::test]
async fn command_status_deserializes_running_and_finished_shapes() {
    let server = MockServer::start().await;
    // Running: exit_code is null.
    Mock::given(method("GET"))
        .and(path(proxy("/command/status/cmd-run")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "cmd-run",
            "content": null,
            "running": true,
            "exit_code": null,
            "started_at": "2026-07-09T00:00:00Z"
        })))
        .mount(&server)
        .await;
    // Finished: exit_code is set.
    Mock::given(method("GET"))
        .and(path(proxy("/command/status/cmd-done")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "cmd-done",
            "content": "ok",
            "running": false,
            "exit_code": 0,
            "started_at": "2026-07-09T00:00:00Z",
            "finished_at": "2026-07-09T00:00:05Z"
        })))
        .mount(&server)
        .await;

    let running = client(&server.uri())
        .command_status("cmd-run")
        .await
        .expect("running status");
    assert!(running.running);
    assert_eq!(running.exit_code, None);
    assert!(!running.is_finished());

    let done = client(&server.uri())
        .command_status("cmd-done")
        .await
        .expect("finished status");
    assert!(!done.running);
    assert_eq!(done.exit_code, Some(0));
    assert!(done.is_finished());
}

#[tokio::test]
async fn command_logs_reads_the_tail_cursor_header_when_present() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/command/cmd-1/logs")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .and(query_param("cursor", "0"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header(TAIL_CURSOR_HEADER, "42")
                .set_body_string("line1\nline2\n"),
        )
        .mount(&server)
        .await;

    let (body, next) = client(&server.uri())
        .command_logs("cmd-1", 0)
        .await
        .expect("logs");
    assert_eq!(body, "line1\nline2\n");
    assert_eq!(next, 42);
}

#[tokio::test]
async fn command_logs_reuses_input_cursor_when_header_absent() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/command/cmd-2/logs")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .and(query_param("cursor", "7"))
        .respond_with(ResponseTemplate::new(200).set_body_string("more output"))
        .mount(&server)
        .await;

    let (body, next) = client(&server.uri())
        .command_logs("cmd-2", 7)
        .await
        .expect("logs");
    assert_eq!(body, "more output");
    // Missing header -> the passed cursor is reused unchanged.
    assert_eq!(next, 7);
}

#[tokio::test]
async fn non_2xx_maps_to_api_error_with_status_and_body() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path(proxy("/command/status/boom")))
        .and(header(API_KEY_HEADER, API_KEY))
        .and(header(EXECD_TOKEN_HEADER, EXECD_TOKEN))
        .respond_with(ResponseTemplate::new(500).set_body_string("kaboom"))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .command_status("boom")
        .await
        .expect_err("500");
    match err {
        OsbError::Api { status, message } => {
            assert_eq!(status, 500);
            assert!(message.contains("kaboom"), "message was {message}");
        }
        other => panic!("expected Api, got {other:?}"),
    }
}

#[test]
fn client_debug_never_leaks_either_secret() {
    let debug = format!("{:?}", client("http://localhost:8080"));
    assert!(
        !debug.contains(API_KEY),
        "api key leaked in Debug output: {debug}"
    );
    assert!(
        !debug.contains(EXECD_TOKEN),
        "execd token leaked in Debug output: {debug}"
    );
}

/// Milliseconds-scale budgets for the stall tests below.
fn tiny_timeouts() -> ExecdTimeouts {
    ExecdTimeouts {
        upload: std::time::Duration::from_millis(200),
        query: std::time::Duration::from_millis(200),
        command_slack: std::time::Duration::from_millis(100),
    }
}

/// A stalled `/files/upload` elapses the UPLOAD budget and surfaces as a timeout
/// `OsbError::Transport` instead of hanging the reconciler's creds push.
#[tokio::test]
async fn stalled_upload_times_out_as_transport() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/files/upload")))
        .respond_with(ResponseTemplate::new(200).set_delay(std::time::Duration::from_secs(5)))
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .with_timeouts(tiny_timeouts())
        .upload_file("/var/run/fkst/creds/github-token", b"tok", 0o400)
        .await
        .expect_err("stalled upload must time out");
    assert!(
        matches!(&err, OsbError::Transport(e) if e.is_timeout()),
        "expected a timeout Transport error, got: {err:?}"
    );
}

/// A BACKGROUND command launch rides the quick-query budget (execd answers with
/// just the init frame and closes): a stalled response times out.
#[tokio::test]
async fn stalled_background_command_launch_times_out() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/command")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"type\":\"init\",\"text\":\"cmd-1\"}\n\n")
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    let err = client(&server.uri())
        .with_timeouts(tiny_timeouts())
        .run_command("echo hi", None, true)
        .await
        .expect_err("stalled background launch must time out");
    assert!(
        matches!(&err, OsbError::Transport(e) if e.is_timeout()),
        "expected a timeout Transport error, got: {err:?}"
    );
}

/// A FOREGROUND command's request budget is its own execd-side timeout + slack —
/// the buffered SSE body lasts the command's lifetime, so the budget must cover
/// it. A response stalled past (timeout_ms + slack) times out...
#[tokio::test]
async fn stalled_foreground_command_times_out_at_its_own_timeout_plus_slack() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/command")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"type\":\"init\",\"text\":\"cmd-1\"}\n\n")
                .set_delay(std::time::Duration::from_secs(5)),
        )
        .mount(&server)
        .await;

    // budget = 100ms (execd timeout) + 100ms (slack) = 200ms << the 5s stall.
    let err = client(&server.uri())
        .with_timeouts(tiny_timeouts())
        .run_command("sleep 99", Some(100), false)
        .await
        .expect_err("a stall past timeout+slack must time out");
    assert!(
        matches!(&err, OsbError::Transport(e) if e.is_timeout()),
        "expected a timeout Transport error, got: {err:?}"
    );
}

/// ...while a foreground command whose stream lasts LONGER than the quick-query
/// budget but WITHIN its own timeout+slack budget is NOT severed — proving the
/// foreground budget derives from the command's timeout, not the query budget.
#[tokio::test]
async fn slow_foreground_command_within_its_own_budget_is_not_severed() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path(proxy("/command")))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_string("data: {\"type\":\"init\",\"text\":\"cmd-42\"}\n\n")
                .set_delay(std::time::Duration::from_millis(500)),
        )
        .mount(&server)
        .await;

    // query budget = 200ms < 500ms stall < 2000ms (execd timeout) + 100ms slack.
    let cmd = client(&server.uri())
        .with_timeouts(tiny_timeouts())
        .run_command("make build", Some(2_000), false)
        .await
        .expect("a foreground command within its own budget must not be severed");
    assert_eq!(cmd.id, "cmd-42");
}
