//! Shared HTTP fixtures for the relay's protocol tests, plus the router-level
//! assertions that hold for every endpoint.

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use crate::audit_relay::test_support::{relay, READ_TOKEN, WRITE_TOKEN};

/// Issue one request against the relay router and return `(status, body)`.
pub(super) async fn call(router: &axum::Router, request: Request<Body>) -> (StatusCode, Value) {
    let response = router
        .clone()
        .oneshot(request)
        .await
        .expect("the router answers");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("the body collects")
        .to_bytes();
    let body = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    (status, body)
}

/// A JSON request with the given bearer token.
pub(super) fn json_request(
    method: &str,
    uri: &str,
    token: Option<&str>,
    body: &impl serde::Serialize,
) -> Request<Body> {
    let mut builder = Request::builder()
        .method(method)
        .uri(uri)
        .header("content-type", "application/json");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder
        .body(Body::from(
            serde_json::to_vec(body).expect("the body encodes"),
        ))
        .expect("the request builds")
}

/// A GET with the given bearer token.
pub(super) fn get_request(uri: &str, token: Option<&str>) -> Request<Body> {
    let mut builder = Request::builder().method("GET").uri(uri);
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    builder.body(Body::empty()).expect("the request builds")
}

#[tokio::test]
async fn every_write_endpoint_refuses_the_read_token() {
    let (_dir, _state, router) = relay();
    let start = crate::audit_relay::test_support::start("11111111-1111-4111-8111-111111111111");
    let cases: Vec<Request<Body>> = vec![
        json_request(
            "POST",
            "/internal/v1/audit/request-starts",
            Some(READ_TOKEN),
            &start,
        ),
        json_request(
            "PUT",
            "/internal/v1/audit/requests/11111111-1111-4111-8111-111111111111/completion",
            Some(READ_TOKEN),
            &crate::audit_relay::test_support::completion(
                "11111111-1111-4111-8111-111111111111",
                Some(101),
            ),
        ),
        json_request(
            "POST",
            "/internal/v1/audit/events",
            Some(READ_TOKEN),
            &crate::audit_relay::test_support::lifecycle(
                "22222222-2222-4222-8222-222222222222",
                "sess-1",
            ),
        ),
    ];
    for request in cases {
        let uri = request.uri().to_string();
        let (status, body) = call(&router, request).await;
        assert_eq!(
            status,
            StatusCode::UNAUTHORIZED,
            "{uri} must refuse a read token"
        );
        assert_eq!(body["error"], "unauthorized");
    }
}

#[tokio::test]
async fn the_read_endpoint_refuses_the_write_token() {
    let (_dir, _state, router) = relay();
    let (status, body) = call(
        &router,
        get_request(
            "/internal/v1/audit/records?scope=all&record_kind=api_request\
             &from=2026-07-30T12:00:00.000Z&to=2026-07-31T12:00:00.000Z&limit=10",
            Some(WRITE_TOKEN),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert_eq!(body["error"], "unauthorized");
}

#[tokio::test]
async fn an_oversized_body_is_refused_rather_than_buffered() {
    let (_dir, state, router) = relay();
    let oversized = vec![b'x'; state.config.max_body_bytes + 1];
    let request = Request::builder()
        .method("POST")
        .uri("/internal/v1/audit/request-starts")
        .header("content-type", "application/json")
        .header("authorization", format!("Bearer {WRITE_TOKEN}"))
        .body(Body::from(oversized))
        .expect("the request builds");
    let (status, _) = call(&router, request).await;
    assert!(
        status == StatusCode::PAYLOAD_TOO_LARGE || status == StatusCode::BAD_REQUEST,
        "an oversized body must be refused, got {status}"
    );
}

#[tokio::test]
async fn no_response_body_ever_echoes_a_credential() {
    let (_dir, _state, router) = relay();
    // Present a wrong token AND a body; neither may come back.
    let start = crate::audit_relay::test_support::start("11111111-1111-4111-8111-111111111111");
    let (_, body) = call(
        &router,
        json_request(
            "POST",
            "/internal/v1/audit/request-starts",
            Some("canary-wrong-token-abcdef"),
            &start,
        ),
    )
    .await;
    let rendered = body.to_string();
    for canary in [WRITE_TOKEN, READ_TOKEN, "canary-wrong-token-abcdef"] {
        assert!(
            !rendered.contains(canary),
            "`{canary}` must not appear in a relay response"
        );
    }
}
