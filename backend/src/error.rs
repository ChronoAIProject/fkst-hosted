//! Unified application error type rendered as the canonical JSON envelope.

use axum::http::{header, HeaderValue, StatusCode};
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
    /// A required dependency (e.g. the GitHub API) is unreachable. Renders as
    /// 503. The message must be safe for clients (no connection detail).
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// BSON serialization failure. Renders as 500.
    #[error("bson serialization error: {0}")]
    Bson(#[from] bson::ser::Error),
    /// Any unexpected internal failure. Renders as 500.
    #[error("internal error: {0}")]
    Internal(#[from] anyhow::Error),
    /// Authentication failure (missing/invalid token). Renders as 401 with
    /// `WWW-Authenticate: Bearer` header.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// Authorization failure (insufficient permissions). Renders as 403.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// A verified, admitted caller selected an operations scope or filter that
    /// only a deployment global administrator may use (epic `AUTH-01`). Renders
    /// as 403 with its own stable code so a client can tell "you may not use
    /// THIS scope" apart from "you may not use this deployment" — the two need
    /// different remedies. The message must never reveal who IS configured.
    #[error("forbidden: {0}")]
    ScopeForbidden(String),
    /// The session-visibility projection is cold or incomplete, so no honest
    /// session-scoped answer exists yet. Renders as 503 with its own stable code
    /// rather than a misleading empty result (epic `AUTH-04`).
    #[error("unavailable: {0}")]
    SessionVisibilityUnavailable(String),
    /// The request cannot be processed due to a semantic issue (e.g. a
    /// dependent resource is missing or in an invalid state). Renders as 422.
    #[error("unprocessable: {0}")]
    Unprocessable(String),
    /// An upstream provider (reached via a proxy) rate-limited the request.
    /// Renders as 429 with a `Retry-After` header. The message is
    /// client-safe (no token or body detail).
    #[error("rate limited: {message}")]
    RateLimited {
        message: String,
        retry_after_secs: u64,
    },
    /// An upstream provider (reached via a proxy) returned an unexpected
    /// error. Renders as 502. The string is client-safe (no token/body
    /// detail).
    #[error("upstream error: {0}")]
    Upstream(String),
}

/// Stable JSON error envelope: `{"error": "<code>", "message": "<text>"}`.
///
/// Public + `ToSchema` so the generated OpenAPI spec can reference it as the
/// body of every documented 4xx/5xx response. `error` is one of the fixed code
/// strings (`invalid_request`, `not_found`, `conflict`, `unauthorized`,
/// `forbidden`, `operations_scope_forbidden`, `unprocessable`, `rate_limited`,
/// `upstream_error`, `unavailable`, `session_visibility_unavailable`,
/// `internal`); `message` is a client-safe human description.
#[derive(Debug, Serialize, utoipa::ToSchema)]
pub struct ErrorEnvelope {
    /// Stable machine-readable error code.
    #[schema(value_type = String, example = "invalid_request")]
    pub error: &'static str,
    /// Human-readable, client-safe description of the failure.
    #[schema(example = "invalid request: title must not be empty")]
    pub message: String,
}

/// Map GitHub-App-domain errors onto the unified type:
/// - NotInstalled / InstallationGone / InstallationMismatch /
///   TokenRequestRejected -> 422 Unprocessable
/// - AppAuth / InvalidKey -> 500 Internal
/// - RateLimited -> 503 Unavailable
/// - InvalidRepoRef -> 400 Validation
/// - Http -> 500 Internal
impl From<crate::github_app::GithubAppError> for AppError {
    fn from(err: crate::github_app::GithubAppError) -> Self {
        use crate::github_app::GithubAppError;
        match err {
            GithubAppError::NotInstalled {
                owner_repo,
                install_url,
            } => {
                let hint = install_url
                    .map(|url| format!(" ({url})"))
                    .unwrap_or_else(|| {
                        " (ask an admin to install the fkst-hosted GitHub App)".to_string()
                    });
                AppError::Unprocessable(format!("github app not installed on {owner_repo}{hint}"))
            }
            GithubAppError::InstallationGone { owner_repo } => AppError::Unprocessable(format!(
                "github app installation vanished for {owner_repo}"
            )),
            GithubAppError::InstallationMismatch { repositories } => {
                tracing::error!(
                    repositories = ?repositories,
                    "cross-repository token scope spans multiple github app installations"
                );
                AppError::Unprocessable(
                    "cross-repository delivery repositories must share one github app installation"
                        .to_string(),
                )
            }
            GithubAppError::NotFound { owner_repo, path } => {
                AppError::NotFound(format!("{owner_repo}: contents path not found: {path}"))
            }
            GithubAppError::TokenRequestRejected(detail) => {
                tracing::error!(detail = %detail, "github token request rejected");
                AppError::Unprocessable("github token request rejected".to_string())
            }
            GithubAppError::AppAuth => AppError::Internal(anyhow::anyhow!(
                "github app auth failed (key or app id rejected)"
            )),
            GithubAppError::InvalidKey => {
                AppError::Internal(anyhow::anyhow!("invalid github app private key"))
            }
            GithubAppError::RateLimited(reset_secs) => {
                AppError::Unavailable(format!("github rate limited; retry after {reset_secs}s"))
            }
            GithubAppError::InvalidRepoRef => {
                AppError::Validation("invalid repository reference".to_string())
            }
            GithubAppError::RefExists => AppError::Conflict("git ref already exists".to_string()),
            // A too-large blob is a client-visible size limit. The raw blob-stream
            // endpoint intercepts this to answer 413 directly (its response is
            // bytes, not the JSON envelope); this arm is the JSON fallback.
            GithubAppError::BlobTooLarge => {
                AppError::Unprocessable("file is too large to preview".to_string())
            }
            GithubAppError::Http(context) => {
                AppError::Internal(anyhow::anyhow!("github http error: {context}"))
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        // The tuple also carries `retry_after`: `Some(secs)` only for
        // `RateLimited` (rendered as a `Retry-After` header), `None` otherwise.
        let (status, code, message, www_authenticate, retry_after) = match &self {
            AppError::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                msg.clone(),
                false,
                None,
            ),
            AppError::NotFound(msg) => {
                (StatusCode::NOT_FOUND, "not_found", msg.clone(), false, None)
            }
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone(), false, None),
            AppError::Unavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                msg.clone(),
                false,
                None,
            ),
            AppError::Unauthorized(msg) => (
                StatusCode::UNAUTHORIZED,
                "unauthorized",
                msg.clone(),
                true,
                None,
            ),
            AppError::Forbidden(msg) => {
                (StatusCode::FORBIDDEN, "forbidden", msg.clone(), false, None)
            }
            AppError::ScopeForbidden(msg) => (
                StatusCode::FORBIDDEN,
                "operations_scope_forbidden",
                msg.clone(),
                false,
                None,
            ),
            AppError::SessionVisibilityUnavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "session_visibility_unavailable",
                msg.clone(),
                false,
                None,
            ),
            AppError::Unprocessable(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable",
                msg.clone(),
                false,
                None,
            ),
            AppError::RateLimited {
                message,
                retry_after_secs,
            } => (
                StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                message.clone(),
                false,
                Some(*retry_after_secs),
            ),
            AppError::Upstream(msg) => (
                StatusCode::BAD_GATEWAY,
                "upstream_error",
                msg.clone(),
                false,
                None,
            ),
            AppError::Config(_) | AppError::Bson(_) | AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                INTERNAL_CLIENT_MESSAGE.to_string(),
                false,
                None,
            ),
        };

        // `Upstream` renders as 502 (a server status) but is a CLIENT-tier
        // failure of the *upstream* provider, not of this service, so it is
        // logged at debug like the 4xx arms — not as a server error.
        let upstream_tier = matches!(self, AppError::Upstream(_) | AppError::RateLimited { .. });
        if status.is_server_error() && !upstream_tier {
            tracing::error!(error = ?self, "request failed");
        } else {
            tracing::debug!(error = %self, "client error");
        }

        let json = Json(ErrorEnvelope {
            error: code,
            message,
        });
        let mut response = (status, json).into_response();

        if www_authenticate {
            response
                .headers_mut()
                .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer"));
        }

        if let Some(secs) = retry_after {
            // A numeric Retry-After (delta-seconds). `from_str` cannot fail for
            // a decimal integer, but handle the error rather than unwrap.
            if let Ok(value) = HeaderValue::from_str(&secs.to_string()) {
                response.headers_mut().insert(header::RETRY_AFTER, value);
            }
        }

        response
    }
}

#[cfg(test)]
#[path = "error_tests.rs"]
mod tests;
