//! Tests for the best-effort recent-output read + the pure RFC3339-prefix strip.
//!
//! The taxonomy cases are the load-bearing ones: a `200` yields the stripped body, a
//! `404` yields the benign empty window `Some("")`, and a `5xx` yields `None` (the
//! WITHHOLD contract — a transient read failure must never clear a degraded flag).

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use super::super::backend_test_support::{
    backend, list_page, osb_config, sandbox_json, SESSION_ID,
};
use super::strip_rfc3339_prefix;

const DIAG_PATH: &str = "/v1/sandboxes/sbx-1/diagnostics/logs";

/// Mount the `resolve_one` list response returning one sandbox `sbx-1` for the session.
async fn mount_resolve(server: &MockServer) {
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
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

#[tokio::test]
async fn recent_output_returns_the_stripped_body_on_200() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    Mock::given(method("GET"))
        .and(path(DIAG_PATH))
        .respond_with(ResponseTemplate::new(200).set_body_string(
            "2026-07-09T00:00:01Z WARN=something happened\nplain line without prefix\n",
        ))
        .mount(&server)
        .await;

    let out = backend(&server.uri(), osb_config())
        .recent_output_impl(SESSION_ID)
        .await;
    // Each line's RFC3339 prefix stripped; the prefix-less line is kept whole.
    assert_eq!(
        out.as_deref(),
        Some("WARN=something happened\nplain line without prefix")
    );
}

#[tokio::test]
async fn recent_output_is_an_empty_window_on_404() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    Mock::given(method("GET"))
        .and(path(DIAG_PATH))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    assert_eq!(
        backend(&server.uri(), osb_config())
            .recent_output_impl(SESSION_ID)
            .await,
        Some(String::new())
    );
}

#[tokio::test]
async fn recent_output_withholds_none_on_a_5xx() {
    let server = MockServer::start().await;
    mount_resolve(&server).await;
    Mock::given(method("GET"))
        .and(path(DIAG_PATH))
        .respond_with(ResponseTemplate::new(503).set_body_string("upstream down"))
        .mount(&server)
        .await;

    // A transport/5xx error MUST be None (WITHHOLD), never a benign `Some("")`.
    assert_eq!(
        backend(&server.uri(), osb_config())
            .recent_output_impl(SESSION_ID)
            .await,
        None
    );
}

#[tokio::test]
async fn recent_output_of_a_gone_session_is_an_empty_window() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    assert_eq!(
        backend(&server.uri(), osb_config())
            .recent_output_impl(SESSION_ID)
            .await,
        Some(String::new())
    );
}

#[test]
fn strip_rfc3339_prefix_only_strips_a_genuine_timestamp() {
    // A valid RFC3339 prefix (with and without fractional seconds / offset) is stripped.
    assert_eq!(
        strip_rfc3339_prefix("2026-07-09T00:00:01Z the message"),
        "the message"
    );
    assert_eq!(
        strip_rfc3339_prefix("2026-07-09T00:00:01.123+00:00 msg here"),
        "msg here"
    );
    // No space at all → the whole line is kept.
    assert_eq!(strip_rfc3339_prefix("no-prefix-line"), "no-prefix-line");
    // A leading token that is NOT an RFC3339 timestamp → the whole line is kept (real
    // engine output is never destroyed).
    assert_eq!(strip_rfc3339_prefix("INFO doing work"), "INFO doing work");
    assert_eq!(
        strip_rfc3339_prefix("garbage-ts more text"),
        "garbage-ts more text"
    );
    // An empty line stays empty.
    assert_eq!(strip_rfc3339_prefix(""), "");
}
