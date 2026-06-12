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
    /// A required dependency (e.g. MongoDB) is unreachable. Renders as 503.
    /// The message must be safe for clients (no connection detail).
    #[error("unavailable: {0}")]
    Unavailable(String),
    /// MongoDB driver failure. Renders as 500; the driver text may carry
    /// host/connection detail, so it is logged but never echoed.
    #[error("mongodb error: {0}")]
    Mongo(#[from] mongodb::error::Error),
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
    /// The request cannot be processed due to a semantic issue (e.g. a
    /// dependent resource is missing or in an invalid state). Renders as 422.
    #[error("unprocessable: {0}")]
    Unprocessable(String),
}

/// Map packages-domain errors onto the unified type: `Validation` -> 400,
/// `Duplicate` -> 409, `NotFound` -> 404, `Db` -> 500 (driver text logged,
/// never echoed).
impl From<crate::packages::PackageError> for AppError {
    fn from(err: crate::packages::PackageError) -> Self {
        use crate::packages::PackageError;
        match err {
            PackageError::Validation(message) => AppError::Validation(message),
            PackageError::Duplicate(name) => {
                AppError::Conflict(format!("package already exists: {name}"))
            }
            PackageError::NotFound(name) => {
                AppError::NotFound(format!("package not found: {name}"))
            }
            PackageError::Db(source) => AppError::Mongo(source),
        }
    }
}

/// Map repo-creation-domain errors onto the unified type:
/// - NameTaken -> 409 Conflict
/// - AuthFailed -> 403 Forbidden
/// - RateLimited -> 503 Unavailable
/// - NyxIdUnavailable -> 503 Unavailable
/// - ExchangeRejected -> 401 Unauthorized
/// - Upstream -> 502 Bad Gateway (mapped as 503 Unavailable)
/// - Malformed -> 500 Internal
impl From<crate::goals::CreateRepoError> for AppError {
    fn from(err: crate::goals::CreateRepoError) -> Self {
        use crate::goals::CreateRepoError;
        match err {
            CreateRepoError::NameTaken(name) => {
                AppError::Conflict(format!("repository name already exists: {name}"))
            }
            CreateRepoError::AuthFailed(detail) => {
                tracing::warn!(detail = %detail, "github auth failure during repo creation");
                AppError::Forbidden(
                    "github authorization failed: cannot create repository".to_string(),
                )
            }
            CreateRepoError::RateLimited => {
                AppError::Unavailable("github rate limited; retry later".to_string())
            }
            CreateRepoError::NyxIdUnavailable(detail) => {
                tracing::error!(detail = %detail, "nyxid unavailable during repo creation");
                AppError::Unavailable(
                    "credential proxy unavailable; cannot create repository".to_string(),
                )
            }
            CreateRepoError::ExchangeRejected(detail) => {
                tracing::warn!(detail = %detail, "token exchange rejected during repo creation");
                AppError::Unauthorized(
                    "token exchange rejected: cannot create repository".to_string(),
                )
            }
            CreateRepoError::Upstream { status, message } => {
                tracing::error!(status, message = %message, "upstream error during repo creation");
                AppError::Unavailable(format!(
                    "github returned error {status}; cannot create repository"
                ))
            }
            CreateRepoError::Malformed(detail) => {
                tracing::error!(detail = %detail, "malformed github response");
                AppError::Internal(anyhow::anyhow!(
                    "malformed github response during repo creation: {detail}"
                ))
            }
        }
    }
}

/// Stable JSON error envelope: `{"error": "<code>", "message": "<text>"}`.
#[derive(Debug, Serialize)]
struct ErrorEnvelope {
    error: &'static str,
    message: String,
}

/// Map auth-domain errors onto the unified type:
/// - client errors (missing/malformed/invalid token) -> 401 Unauthorized
/// - JWKS outage -> 503 Unavailable (inner detail logged only)
impl From<crate::auth::AuthError> for AppError {
    fn from(err: crate::auth::AuthError) -> Self {
        use crate::auth::AuthError;
        match err {
            AuthError::MissingToken => AppError::Unauthorized("missing bearer token".to_string()),
            AuthError::MalformedHeader => {
                AppError::Unauthorized("malformed authorization header".to_string())
            }
            AuthError::InvalidToken(reason) => {
                AppError::Unauthorized(format!("invalid token: {reason}"))
            }
            AuthError::JwksUnavailable(detail) => {
                tracing::error!(error = %detail, "JWKS fetch failed");
                AppError::Unavailable("authentication service unavailable".to_string())
            }
        }
    }
}

/// Map GitHub-App-domain errors onto the unified type:
/// - NotInstalled / InstallationGone / TokenRequestRejected -> 422 Unprocessable
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
            GithubAppError::Http(context) => {
                AppError::Internal(anyhow::anyhow!("github http error: {context}"))
            }
        }
    }
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, code, message, www_authenticate) = match &self {
            AppError::Validation(msg) => (
                StatusCode::BAD_REQUEST,
                "invalid_request",
                msg.clone(),
                false,
            ),
            AppError::NotFound(msg) => (StatusCode::NOT_FOUND, "not_found", msg.clone(), false),
            AppError::Conflict(msg) => (StatusCode::CONFLICT, "conflict", msg.clone(), false),
            AppError::Unavailable(msg) => (
                StatusCode::SERVICE_UNAVAILABLE,
                "unavailable",
                msg.clone(),
                false,
            ),
            AppError::Unauthorized(msg) => {
                (StatusCode::UNAUTHORIZED, "unauthorized", msg.clone(), true)
            }
            AppError::Forbidden(msg) => (StatusCode::FORBIDDEN, "forbidden", msg.clone(), false),
            AppError::Unprocessable(msg) => (
                StatusCode::UNPROCESSABLE_ENTITY,
                "unprocessable",
                msg.clone(),
                false,
            ),
            AppError::Config(_)
            | AppError::Mongo(_)
            | AppError::Bson(_)
            | AppError::Internal(_) => (
                StatusCode::INTERNAL_SERVER_ERROR,
                "internal",
                INTERNAL_CLIENT_MESSAGE.to_string(),
                false,
            ),
        };

        if status.is_server_error() {
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

        response
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use http_body_util::BodyExt;

    async fn render(err: AppError) -> (StatusCode, serde_json::Value, Vec<(String, String)>) {
        let response = err.into_response();
        let status = response.status();
        let headers: Vec<(String, String)> = response
            .headers()
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_str().unwrap_or_default().to_string()))
            .collect();
        let bytes = response
            .into_body()
            .collect()
            .await
            .expect("collect body")
            .to_bytes();
        let json = serde_json::from_slice(&bytes).expect("json body");
        (status, json, headers)
    }

    #[tokio::test]
    async fn validation_renders_400_invalid_request() {
        let (status, body, _headers) = render(AppError::Validation("bad field".into())).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        assert_eq!(body["message"], "bad field");
    }

    #[tokio::test]
    async fn not_found_renders_404_not_found() {
        let (status, body, _headers) =
            render(AppError::NotFound("package \"foo\" does not exist".into())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["error"], "not_found");
        assert_eq!(body["message"], "package \"foo\" does not exist");
    }

    #[tokio::test]
    async fn conflict_renders_409_conflict() {
        let (status, body, _headers) = render(AppError::Conflict("duplicate name".into())).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");
        assert_eq!(body["message"], "duplicate name");
    }

    #[tokio::test]
    async fn unavailable_renders_503_unavailable() {
        let (status, body, _headers) =
            render(AppError::Unavailable("mongo unreachable".into())).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "unavailable");
        assert_eq!(body["message"], "mongo unreachable");
    }

    #[tokio::test]
    async fn unauthorized_renders_401_with_www_authenticate_bearer() {
        let (status, body, headers) =
            render(AppError::Unauthorized("missing bearer token".into())).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");
        assert_eq!(body["message"], "missing bearer token");
        let www = headers.iter().find(|(k, _)| k == "www-authenticate");
        assert!(www.is_some(), "WWW-Authenticate header must be present");
        assert_eq!(www.unwrap().1, "Bearer");
    }

    #[tokio::test]
    async fn forbidden_renders_403_forbidden() {
        let (status, body, headers) =
            render(AppError::Forbidden("insufficient scope".into())).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body["error"], "forbidden");
        assert_eq!(body["message"], "insufficient scope");
        let www = headers.iter().find(|(k, _)| k == "www-authenticate");
        assert!(www.is_none(), "Forbidden must NOT set WWW-Authenticate");
    }

    #[tokio::test]
    async fn auth_error_missing_token_maps_to_401() {
        let err: AppError = crate::auth::AuthError::MissingToken.into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(body["error"], "unauthorized");
    }

    #[tokio::test]
    async fn auth_error_jwks_unavailable_maps_to_503() {
        let err: AppError =
            crate::auth::AuthError::JwksUnavailable("connection refused".to_string()).into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "unavailable");
        assert_eq!(body["message"], "authentication service unavailable");
        // Inner detail must not leak to client.
        assert!(!body.to_string().contains("connection refused"));
    }

    #[tokio::test]
    async fn mongo_renders_500_without_leaking_inner_text() {
        // A real driver error wrapping a deterministic inner text.
        let io = std::io::Error::other("dial mongodb://user:secret@db:27017 refused");
        let err = AppError::Mongo(mongodb::error::Error::from(io));
        // The inner text stays reachable for logging via Display/Debug.
        assert!(format!("{err}").contains("secret"));

        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("secret"));
        assert!(!body.to_string().contains("27017"));
    }

    #[tokio::test]
    async fn bson_renders_500_without_leaking_inner_text() {
        // BSON document keys must be strings; integer keys produce a real
        // bson::ser::Error.
        let bad_keys: std::collections::HashMap<u32, &str> =
            std::collections::HashMap::from([(1, "leaky-detail")]);
        let bson_err = bson::to_document(&bad_keys).expect_err("non-string keys must fail");
        let err = AppError::Bson(bson_err);
        let inner_text = format!("{err}");

        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        // The serializer's own message never reaches the client.
        let rendered = body.to_string();
        assert!(!rendered.contains(inner_text.trim_start_matches("bson serialization error: ")));
    }

    #[tokio::test]
    async fn internal_renders_500_without_leaking_inner_text() {
        let err = AppError::Internal(anyhow::anyhow!("db creds: secret"));
        // The inner text stays reachable for logging via Display/Debug.
        assert!(format!("{err}").contains("db creds: secret"));
        assert!(format!("{err:?}").contains("db creds: secret"));

        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("db creds"));
        assert!(!body.to_string().contains("secret"));
    }

    #[tokio::test]
    async fn package_validation_renders_400_invalid_request() {
        let err: AppError =
            crate::packages::PackageError::Validation("invalid package name".into()).into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
        assert_eq!(body["message"], "invalid package name");
    }

    #[tokio::test]
    async fn package_duplicate_renders_409_conflict() {
        let err: AppError = crate::packages::PackageError::Duplicate("demo".into()).into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body["error"], "conflict");
        assert_eq!(body["message"], "package already exists: demo");
    }

    #[tokio::test]
    async fn package_db_renders_500_without_leaking_driver_text() {
        let io = std::io::Error::other("dial mongodb://user:secret@db:27017 refused");
        let err: AppError =
            crate::packages::PackageError::Db(mongodb::error::Error::from(io)).into();

        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("secret"));
        assert!(!body.to_string().contains("27017"));
    }

    #[tokio::test]
    async fn config_renders_500_without_leaking_inner_text() {
        let err = AppError::Config("envy: missing FOO".into());
        // The inner text stays reachable for logging via Display/Debug.
        assert!(format!("{err}").contains("envy: missing FOO"));
        assert!(format!("{err:?}").contains("envy: missing FOO"));

        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(body["error"], "internal");
        assert_eq!(body["message"], "internal server error");
        assert!(!body.to_string().contains("envy"));
        assert!(!body.to_string().contains("FOO"));
    }

    #[tokio::test]
    async fn unprocessable_renders_422() {
        let (status, body, _headers) =
            render(AppError::Unprocessable("semantic issue".into())).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "unprocessable");
        assert_eq!(body["message"], "semantic issue");
    }

    #[tokio::test]
    async fn github_app_not_installed_renders_422_with_hint() {
        let err: AppError = crate::github_app::GithubAppError::NotInstalled {
            owner_repo: "acme/site".to_string(),
            install_url: Some("https://github.com/apps/fkst-hosted/installations/new".to_string()),
        }
        .into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["error"], "unprocessable");
        let msg = body["message"].as_str().expect("message");
        assert!(msg.contains("acme/site"), "message: {msg}");
        assert!(msg.contains("fkst-hosted"), "message: {msg}");
    }

    #[tokio::test]
    async fn github_app_not_installed_without_slug_gives_admin_hint() {
        let err: AppError = crate::github_app::GithubAppError::NotInstalled {
            owner_repo: "acme/site".to_string(),
            install_url: None,
        }
        .into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        let msg = body["message"].as_str().expect("message");
        assert!(msg.contains("ask an admin"), "message: {msg}");
    }

    #[tokio::test]
    async fn github_app_rate_limited_renders_503() {
        let err: AppError = crate::github_app::GithubAppError::RateLimited(120).into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(body["error"], "unavailable");
        assert!(body["message"].as_str().unwrap().contains("rate limited"));
    }

    #[tokio::test]
    async fn github_app_token_rejected_detail_never_reaches_client() {
        let err: AppError =
            crate::github_app::GithubAppError::TokenRequestRejected("secret detail".to_string())
                .into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert!(
            !body.to_string().contains("secret detail"),
            "rejected detail must not leak: {}",
            body
        );
    }

    #[tokio::test]
    async fn github_app_auth_and_key_errors_render_500() {
        for err in [
            AppError::from(crate::github_app::GithubAppError::AppAuth),
            AppError::from(crate::github_app::GithubAppError::InvalidKey),
            AppError::from(crate::github_app::GithubAppError::Http(
                "network failure".to_string(),
            )),
        ] {
            let (status, body, _headers) = render(err).await;
            assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
            assert_eq!(body["error"], "internal");
            assert_eq!(body["message"], "internal server error");
        }
    }

    #[tokio::test]
    async fn github_app_invalid_repo_ref_renders_400() {
        let err: AppError = crate::github_app::GithubAppError::InvalidRepoRef.into();
        let (status, body, _headers) = render(err).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body["error"], "invalid_request");
    }

    #[tokio::test]
    async fn no_error_or_debug_output_contains_minted_token_or_key() {
        use crate::github_app::GithubAppError;
        let secret_token = "ghs_SECRET_INSTALLATION_TOKEN_12345";
        let secret_pem = "-----BEGIN RSA PRIVATE KEY-----\nSECRET\n-----END RSA PRIVATE KEY-----";
        let errors: Vec<GithubAppError> = vec![
            GithubAppError::NotInstalled {
                owner_repo: "a/b".to_string(),
                install_url: None,
            },
            GithubAppError::InstallationGone {
                owner_repo: "a/b".to_string(),
            },
            GithubAppError::AppAuth,
            GithubAppError::RateLimited(60),
            GithubAppError::TokenRequestRejected(format!("permission denied for {secret_token}")),
            GithubAppError::InvalidKey,
            GithubAppError::InvalidRepoRef,
            GithubAppError::Http(format!("request failed with {secret_pem}")),
        ];
        for err in &errors {
            let display = format!("{err}");
            let debug = format!("{err:?}");
            assert!(
                !display.contains(secret_token),
                "Display leaked token: {display}"
            );
            assert!(!debug.contains(secret_token), "Debug leaked token: {debug}");
            assert!(
                !display.contains(secret_pem),
                "Display leaked key: {display}"
            );
            assert!(!debug.contains(secret_pem), "Debug leaked key: {debug}");
        }
        // The AppError mapping also must not leak.
        for err in &errors {
            let app_err: AppError = err.clone().into();
            let display = format!("{app_err}");
            let debug = format!("{app_err:?}");
            assert!(
                !display.contains(secret_token),
                "AppError Display leaked token: {display}"
            );
            assert!(
                !debug.contains(secret_token),
                "AppError Debug leaked token: {debug}"
            );
        }
    }
}
