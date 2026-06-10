//! Unified application error type rendered as the canonical JSON envelope.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Serialize;

/// Fixed client-facing message for every 5xx response. The underlying error
/// text (which may mention env var names, paths, or connection strings) is
/// logged only and never sent to the client.
const INTERNAL_CLIENT_MESSAGE: &str = "internal server error";

/// Unified error type used across the fkst-hosted API.
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// Configuration could not be loaded or parsed. Renders as 500.
    #[error("configuration error: {0}")]
    Config(String),
    /// The request payload or parameters are invalid. Renders as 400.
    #[error("invalid request: {0}")]
    Validation(String),
    /// The requested resource does not exist. Renders as 404.
    #[error("not found: {0}")]
    NotFound(String),
    /// The request conflicts with the current state. Renders as 409.
    #[error("conflict: {0}")]
    Conflict(String),
    /// Any unexpected internal failure. Renders as 500.
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
}

/// Stable JSON error envelope: `{"error": "<code>", "message": "<text>"}`.
#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: &'static str,
    message: String,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message) = match &self {
            AppError::Validation(msg) => (StatusCode::BAD_REQUEST, "invalid_request", msg.clone()),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone()),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone()),
            AppError::Config(_) | AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                INTERNAL_CLIENT_MESSAGE.to_string(),
            ),
        };

        if status.is_server_error() {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = %self, "client error");
        }

        (
            status,
            Json(ErrorEnvelope {
                error: code,
                message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn render(err: AppError) -> (StatusCode, serde_json::Value) {
        let response = err.into_response();
        let status = response.status();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let json = serde_json::from_slice(&bytes).expect("json body");
        (status, json)
    }

    #[tokio::test]
    async fn validation_renders_400_invalid_request() {
        let (status, body) = render(AppError::Validation("bad field".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        assert_eq!(body["message"], "bad field");
    }

    #[tokio::test]
    async fn not_found_renders_404_not_found() {
        let (status, body) =
            render(AppError::NotFound("package \"foo\" does not exist".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not_found");
        assert_eq!(body["message"], "package \"foo\" does not exist");
    }

    #[tokio::test]
    async fn conflict_renders_409_conflict() {
        let (status, body) = render(AppError::Conflict("duplicate name".into())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");
        assert_eq!(body["message"], "duplicate name");
    }

    #[tokio::test]
    async fn internal_renders_500_without_leaking_inner_text() {
        let err = AppError::Internal(anyhow::anyhow!("db creds: secret"));
        // The inner text stays reachable for logging via Display/Debug.
        assert!(format!("{err}").contains("db creds: secret"));
        assert!(format!("{err:?}").contains("db creds: secret"));

        let (status, body) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("db creds"));
        assert!(!body.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn config_renders_500_without_leaking_inner_text() {
        let err = AppError::Config("envy: missing FOO".into());
        // The inner text stays reachable for logging via Display/Debug.
        assert!(format!("{err}").contains("envy: missing FOO"));
        assert!(format!("{err:?}").contains("envy: missing FOO"));

        let (status, body) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("envy"));
        assert!(!body.to_string().contains("FOO"));
    }
}
