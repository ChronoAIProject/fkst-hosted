//! Tests for the struct-level helpers: `resolve_one` (0/1/many) and the `OsbError` ->
//! `BackendError` mapping. The five fleet verbs #419 completes are covered by the
//! sibling `rotation_tests` / `health_tests` / `logs_tests` / `validation_tests`.

use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::session_backend::opensandbox::dto::OsbError;
use crate::session_backend::BackendError;

use super::backend_test_support::{backend, list_page, osb_config, sandbox_json, SESSION_ID};

/// An `acme/site` sandbox at `created_at` (minimal metadata for resolve).
fn sbx(id: &str, created_at: &str) -> serde_json::Value {
    sandbox_json(
        id,
        "Running",
        created_at,
        json!({ "fkst-session-id": SESSION_ID }),
    )
}

#[tokio::test]
async fn resolve_one_is_not_found_on_zero_matches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([]))))
        .mount(&server)
        .await;

    let err = backend(&server.uri(), osb_config())
        .resolve_one(SESSION_ID)
        .await
        .expect_err("no match");
    assert!(matches!(err, BackendError::NotFound), "got {err:?}");
}

#[tokio::test]
async fn resolve_one_returns_the_single_match() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_json(list_page(json!([sbx("sbx-1", "2026-07-09T00:00:00Z")]))),
        )
        .mount(&server)
        .await;

    let view = backend(&server.uri(), osb_config())
        .resolve_one(SESSION_ID)
        .await
        .expect("resolved");
    assert_eq!(view.id, "sbx-1");
}

#[tokio::test]
async fn resolve_one_picks_the_oldest_on_multiple_matches() {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/v1/sandboxes"))
        .respond_with(ResponseTemplate::new(200).set_body_json(list_page(json!([
            sbx("s-new", "2026-07-09T01:00:00Z"),
            sbx("s-old", "2026-07-09T00:00:00Z"),
        ]))))
        .mount(&server)
        .await;

    let view = backend(&server.uri(), osb_config())
        .resolve_one(SESSION_ID)
        .await
        .expect("resolved");
    assert_eq!(view.id, "s-old", "the oldest by (created_at, id) is chosen");
}

#[test]
fn osb_error_maps_not_found_and_folds_the_rest() {
    assert!(matches!(
        BackendError::from(OsbError::NotFound),
        BackendError::NotFound
    ));
    assert!(matches!(
        BackendError::from(OsbError::Api {
            status: 500,
            message: "boom".to_string()
        }),
        BackendError::Other(_)
    ));
}
