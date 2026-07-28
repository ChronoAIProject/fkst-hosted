//! Tests for [`SelfDispatch`], driving the REAL `build_router` (sibling `#[path]`
//! module).
//!
//! The test vehicle is the **logs** surface, chosen deliberately: it is the only
//! read endpoint that reaches a 200 with no Kubernetes cluster, because its
//! fixtures mock GitHub `/user` and back chrono-storage with wiremock
//! (`routes::logs::test_support`). The environment-profiles handler, by contrast,
//! builds a real `kube::Client` per request and 503s in CI — so it can only prove
//! the error path, never the authorized one.

use secrecy::SecretString;

use super::*;
use crate::routes::logs::test_support::{
    github_user_ok, log_config, registry, state, storage_server, AUTHOR_ID, SESSION_ID,
};
use crate::state::empty_self_router;

/// Build the real router over the logs fixtures and return a dispatcher onto it.
///
/// `build_router` is what populates the handle, so calling it IS the wiring under
/// test — a dispatcher built from a state whose router was never built must fail.
async fn dispatch_over_logs_router(
    bundle_present: bool,
) -> (
    SelfDispatch,
    wiremock::MockServer,
    wiremock::MockServer,
    crate::state::AppState,
) {
    let gh = github_user_ok("alice", AUTHOR_ID).await;
    let (storage, storage_mock) = storage_server(bundle_present).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );
    // The token→identity cache is process-global; reset it so a reused token string
    // cannot carry another test's mocked identity.
    crate::routes::logs::identity::clear_cache();
    let _router = crate::router::build_router(st.clone()).expect("router builds");
    (
        SelfDispatch::new(st.self_router.clone()),
        gh,
        storage_mock,
        st,
    )
}

fn bearer(token: &str) -> SecretString {
    SecretString::from(token.to_string())
}

#[tokio::test]
async fn an_authorized_get_returns_the_endpoint_data() {
    let (dispatch, _gh, _storage, _st) = dispatch_over_logs_router(true).await;
    let response = dispatch
        .get(
            &format!("/api/v1/logs/{SESSION_ID}/runs"),
            &bearer("gho_author"),
            None,
        )
        .await
        .expect("dispatch succeeds");
    assert_eq!(response.status, 200);
    assert!(
        response.body.is_array(),
        "the runs listing is a JSON array: {:?}",
        response.body
    );
    assert!(!response.truncated);
}

#[tokio::test]
async fn an_unauthenticated_get_yields_a_401_envelope_as_data() {
    let gh = crate::routes::logs::test_support::github_user_401().await;
    let (storage, _s) = storage_server(true).await;
    let st = state(
        gh.uri(),
        Some(storage),
        log_config(&[], false),
        registry(&[]),
    );
    crate::routes::logs::identity::clear_cache();
    let _router = crate::router::build_router(st.clone()).expect("router builds");
    let dispatch = SelfDispatch::new(st.self_router.clone());

    let response = dispatch
        .get(
            &format!("/api/v1/logs/{SESSION_ID}/runs"),
            &bearer("gho_rejected"),
            None,
        )
        .await
        .expect("dispatch itself succeeds");
    assert_eq!(response.status, 401);
    assert!(
        response.body.get("error").is_some(),
        "the error envelope must reach the caller as data: {:?}",
        response.body
    );
}

#[tokio::test]
async fn an_oversized_body_is_truncated_and_flagged() {
    // A log tail far above the cap: the truncated payload is no longer valid JSON,
    // so it must arrive wrapped so the model can see it is a fragment.
    let oversized = "x".repeat(MAX_TOOL_RESULT_BYTES * 2);
    let (value, truncated) = decode_body(oversized.as_bytes());
    assert!(truncated);
    let text = value["truncated_text"]
        .as_str()
        .expect("a non-JSON truncation is wrapped as truncated_text");
    assert_eq!(text.len(), MAX_TOOL_RESULT_BYTES);
}

#[tokio::test]
async fn a_complete_json_body_is_parsed_untruncated() {
    let (value, truncated) = decode_body(br#"{"runs":[{"run_id":"r1"}]}"#);
    assert!(!truncated);
    assert_eq!(value["runs"][0]["run_id"], "r1");
}

#[tokio::test]
async fn a_complete_non_json_body_is_wrapped_as_text() {
    let (value, truncated) = decode_body(b"plain text");
    assert!(!truncated);
    assert_eq!(value["text"], "plain text");
}

#[tokio::test]
async fn truncation_never_splits_a_utf8_character() {
    // A body of three-byte characters whose cap lands mid-character.
    let body = "日".repeat(MAX_TOOL_RESULT_BYTES);
    let (value, truncated) = decode_body(body.as_bytes());
    assert!(truncated);
    let text = value["truncated_text"].as_str().expect("truncated text");
    assert!(
        text.chars().all(|c| c == '日'),
        "no replacement character may appear"
    );
    assert!(text.len() <= MAX_TOOL_RESULT_BYTES);
}

#[tokio::test]
async fn dispatching_before_the_router_is_built_is_a_startup_error() {
    let dispatch = SelfDispatch::new(empty_self_router());
    let error = dispatch
        .get("/api/v1/overview", &bearer("gho_x"), None)
        .await
        .expect_err("an unpopulated handle must fail loudly");
    assert!(matches!(error, DispatchError::RouterUnset), "got {error:?}");
}

#[tokio::test]
async fn a_malformed_path_is_rejected_before_dispatch() {
    let (dispatch, _gh, _storage, _st) = dispatch_over_logs_router(true).await;
    let error = dispatch
        .get("http://[invalid", &bearer("gho_author"), None)
        .await
        .expect_err("an unparseable uri must fail");
    assert!(
        matches!(error, DispatchError::BadRequest(_)),
        "got {error:?}"
    );
}

#[test]
fn query_strings_are_kept_out_of_debug_logs() {
    assert_eq!(
        redact_query("/api/v1/logs/s1/file?path=secrets.log&tail_bytes=10"),
        "/api/v1/logs/s1/file"
    );
    assert_eq!(redact_query("/api/v1/overview"), "/api/v1/overview");
}

#[test]
fn only_2xx_counts_as_success() {
    assert!(is_success(200));
    assert!(is_success(204));
    assert!(!is_success(403));
    assert!(!is_success(503));
    assert!(!is_success(999), "an out-of-range status is not a success");
}
