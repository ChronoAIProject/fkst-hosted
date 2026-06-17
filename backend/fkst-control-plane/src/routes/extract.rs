//! Request extractors that reject with the crate's canonical error envelope.
//!
//! Axum's stock `Json` rejection answers with plaintext bodies and statuses
//! like 413/415/422; the API contract is a single JSON envelope and `400` for
//! every malformed request, so `AppJson` re-maps the rejection onto
//! [`AppError`] before it ever leaves the handler layer.

use axum::extract::rejection::JsonRejection;
use axum::extract::{FromRequest, Request};
use axum::http::StatusCode;

use crate::error::AppError;

/// `axum::Json` with the rejection mapped onto [`AppError`] so a bad body
/// renders the canonical `{"error", "message"}` envelope as a `400`.
pub struct AppJson<T>(pub T);

/// Cap (in characters) on the rejection text recorded in the warn log.
///
/// Serde's rejection text quotes fragments of the client's own payload, so
/// it is attacker-influenced: without a cap a hostile body could flood the
/// log, and without escaping it could forge log lines via embedded newlines
/// or terminal control sequences.
const MAX_REJECTION_LOG_CHARS: usize = 256;

/// Truncate client-influenced rejection text for logging: at most
/// [`MAX_REJECTION_LOG_CHARS`] characters (`char`-wise, so never split mid
/// code point). The caller MUST log the result with the `?` (Debug) sigil so
/// newlines and control characters render escaped, never verbatim.
fn log_reason(text: &str) -> String {
    text.chars().take(MAX_REJECTION_LOG_CHARS).collect()
}

/// Map a body rejection onto the unified error type.
///
/// - Over-limit bodies (the body-limit layer surfaces 413) become a terse
///   `400 "request body too large"` — deliberately `400`, not `413`, to keep
///   one status/envelope discipline for every invalid request.
/// - Everything else (malformed JSON, unknown field, type mismatch, missing
///   `Content-Type`) echoes the rejection text: it only ever describes the
///   client's own payload, never server internals.
fn map_rejection(rejection: JsonRejection) -> AppError {
    if rejection.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return AppError::Validation("request body too large".to_string());
    }
    AppError::Validation(format!("invalid request body: {rejection}"))
}

#[axum::async_trait]
impl<S, T> FromRequest<S> for AppJson<T>
where
    axum::Json<T>: FromRequest<S, Rejection = JsonRejection>,
    S: Send + Sync,
{
    type Rejection = AppError;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        match axum::Json::<T>::from_request(req, state).await {
            Ok(axum::Json(value)) => Ok(AppJson(value)),
            Err(rejection) => {
                // Debug-escaped (`?`) and length-capped: the rejection text
                // echoes client payload fragments and must not be able to
                // forge or flood log output.
                let reason = log_reason(&rejection.to_string());
                tracing::warn!(reason = ?reason, "request body rejected");
                Err(map_rejection(rejection))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::extract::DefaultBodyLimit;
    use axum::http::{header, Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use http_body_util::BodyExt;
    use serde::Deserialize;
    use tower::ServiceExt;

    use super::*;

    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Probe {
        #[allow(dead_code)]
        value: String,
    }

    async fn probe(AppJson(_probe): AppJson<Probe>) -> StatusCode {
        StatusCode::OK
    }

    /// Tiny router with an aggressive body limit so both rejection arms are
    /// reachable through the real extractor path.
    fn test_router() -> Router {
        Router::new()
            .route("/probe", post(probe))
            .layer(DefaultBodyLimit::max(64))
    }

    async fn send(body: &str) -> (StatusCode, serde_json::Value) {
        let response = test_router()
            .oneshot(
                Request::post("/probe")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(body.to_string()))
                    .expect("request builds"),
            )
            .await
            .expect("router must respond");
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let json = serde_json::from_slice(&bytes).expect("envelope is JSON");
        (status, json)
    }

    #[tokio::test]
    async fn valid_body_passes_through() {
        let response = test_router()
            .oneshot(
                Request::post("/probe")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"value":"ok"}"#))
                    .expect("request builds"),
            )
            .await
            .expect("router must respond");
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn over_limit_body_maps_to_400_request_body_too_large() {
        // 65 bytes of valid-prefix JSON against a 64-byte limit: the body
        // limit fires (a 413 internally) and must surface as our 400.
        let oversized = format!(r#"{{"value":"{}"}}"#, "x".repeat(64));
        assert!(oversized.len() > 64);
        let (status, body) = send(&oversized).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_ne!(status, StatusCode::PAYLOAD_TOO_LARGE);
        assert_eq!(body["error"], "invalid_request");
        assert_eq!(body["message"], "request body too large");
    }

    #[tokio::test]
    async fn malformed_json_maps_to_400_invalid_request_body() {
        let (status, body) = send("{not json").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        let message = body["message"].as_str().expect("message is a string");
        assert!(
            message.starts_with("invalid request body: "),
            "got: {message}"
        );
    }

    #[test]
    fn log_reason_caps_the_length_in_characters() {
        let long = "é".repeat(MAX_REJECTION_LOG_CHARS + 100);
        let reason = log_reason(&long);
        assert_eq!(reason.chars().count(), MAX_REJECTION_LOG_CHARS);

        let short = "tiny reason";
        assert_eq!(log_reason(short), short);
    }

    #[test]
    fn log_reason_renders_control_characters_escaped_under_debug() {
        // The call site logs with the `?` sigil (Debug). A payload-borne
        // newline or ESC must come out escaped, not as a raw byte that could
        // forge a second log line or smuggle a terminal control sequence.
        let hostile = "line1\nFORGED level=warn\u{1b}[0m";
        let rendered = format!("{:?}", log_reason(hostile));
        assert!(!rendered.contains('\n'), "raw newline leaked: {rendered}");
        assert!(!rendered.contains('\u{1b}'), "raw ESC leaked: {rendered}");
        assert!(rendered.contains("\\n"), "newline must render escaped");
    }

    #[tokio::test]
    async fn unknown_field_maps_to_400_invalid_request_body() {
        let (status, body) = send(r#"{"value":"ok","bogus":1}"#).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        let message = body["message"].as_str().expect("message is a string");
        assert!(
            message.starts_with("invalid request body: "),
            "got: {message}"
        );
    }
}
