//! Unit tests for the URI-free HTTP tracing span.

use super::*;
use axum::body::Body;
use axum::http::Request;

/// Render the span's own fields by formatting its metadata plus a recorded
/// snapshot. `tracing` does not expose recorded values without a subscriber, so
/// the assertion here is on the field NAMES the span declares — the integration
/// canary in `tests/audit_tracing.rs` proves the emitted output too.
#[test]
fn the_span_declares_only_method_and_request_id() {
    let request = Request::builder()
        .uri("/api/v1/auth/github/callback?code=secret-code&state=secret-state")
        .header("x-request-id", "req-1")
        .header("authorization", "Bearer super-secret-token")
        .body(Body::empty())
        .expect("request builds");
    let span = SafeHttpSpan.make_span(&request);
    let fields: Vec<&str> = span
        .metadata()
        .expect("span has metadata")
        .fields()
        .iter()
        .map(|field| field.name())
        .collect();
    assert_eq!(fields, vec!["method", "request_id"]);
    assert_eq!(span.metadata().expect("metadata").name(), "http_request");
}

#[test]
fn an_unacceptable_request_id_header_is_not_carried_into_the_span() {
    // Defence in depth: the middleware normalizes the header before this layer
    // sees it, but this type must be safe wherever it is mounted.
    let request = Request::builder()
        .uri("/health")
        .header("x-request-id", "spoofed value with spaces")
        .body(Body::empty())
        .expect("request builds");
    // Building the span must not panic and must not carry the raw value; the
    // integration canary asserts the emitted output.
    let _span = SafeHttpSpan.make_span(&request);
    assert!(!is_acceptable("spoofed value with spaces"));
}
