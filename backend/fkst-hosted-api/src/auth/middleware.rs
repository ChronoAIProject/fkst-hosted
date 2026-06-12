//! Axum middleware that enforces JWT authentication on protected routes and
//! a `FromRequestParts` extractor for `AuthContext`.

use std::sync::Arc;

use axum::body::Body;
use axum::extract::{FromRequestParts, State};
use axum::http::request::Parts;
use axum::http::Request;
use axum::middleware::Next;
use axum::response::Response;

use crate::error::AppError;
use crate::state::AppState;

use axum::Router;

use super::verify::Verifier;
use super::AuthContext;
use super::AuthError;

/// Wrap a router with JWT authentication middleware. Every request passing
/// through must carry a valid `Authorization: Bearer <token>` header.
pub fn protect(router: Router<AppState>, verifier: Arc<Verifier>) -> Router<AppState> {
    router.layer(axum::middleware::from_fn_with_state(
        AuthMiddlewareState { verifier },
        auth_fn,
    ))
}

/// Internal state passed to the middleware via `from_fn_with_state`.
#[derive(Clone)]
struct AuthMiddlewareState {
    verifier: Arc<Verifier>,
}

/// Middleware function that extracts the Bearer token, verifies it, and inserts
/// `AuthContext` into request extensions.
async fn auth_fn(
    State(mid_state): State<AuthMiddlewareState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AppError> {
    let bearer = extract_bearer(request.headers())?;
    let auth_ctx = mid_state.verifier.verify(bearer).await?;
    let mut request = request;
    request.extensions_mut().insert(auth_ctx);
    Ok(next.run(request).await)
}

/// Extract the bearer token from the `Authorization` header.
/// Returns `AuthError::MissingToken` if absent, `AuthError::MalformedHeader`
/// if the header is present but not a valid Bearer token.
fn extract_bearer(headers: &axum::http::HeaderMap) -> Result<&str, AuthError> {
    let header_value = headers
        .get(axum::http::header::AUTHORIZATION)
        .ok_or(AuthError::MissingToken)?;

    let text = header_value
        .to_str()
        .map_err(|_| AuthError::MalformedHeader)?;

    // Must start with "Bearer " (case-sensitive per RFC 6750).
    text.strip_prefix("Bearer ")
        .ok_or(AuthError::MalformedHeader)
}

/// `FromRequestParts` implementation for `AuthContext`.
///
/// Extraction strategy:
/// 1. If the extension is present (middleware ran), clone it.
/// 2. If absent and auth is disabled, yield the dev context.
/// 3. If absent and auth is enabled, this is a programming error (route not
///    behind the auth layer) -> 500.
#[axum::async_trait]
impl FromRequestParts<AppState> for AuthContext {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        match parts.extensions.get::<AuthContext>() {
            Some(ctx) => Ok(ctx.clone()),
            None => match &state.auth_mode {
                super::AuthMode::Disabled => Ok(AuthContext::dev()),
                super::AuthMode::Enabled(_) => Err(AppError::Internal(anyhow::anyhow!(
                    "route not behind the auth layer"
                ))),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn extract_bearer_returns_token_after_prefix() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer abc123"),
        );
        assert_eq!(extract_bearer(&headers).unwrap(), "abc123");
    }

    #[test]
    fn extract_bearer_missing_header_is_missing_token() {
        let headers = HeaderMap::new();
        let err = extract_bearer(&headers).unwrap_err();
        assert!(matches!(err, AuthError::MissingToken));
    }

    #[test]
    fn extract_bearer_non_bearer_is_malformed_header() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic abc123"),
        );
        let err = extract_bearer(&headers).unwrap_err();
        assert!(matches!(err, AuthError::MalformedHeader));
    }

    #[test]
    fn extract_bearer_bearer_without_space_is_malformed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer"),
        );
        let err = extract_bearer(&headers).unwrap_err();
        assert!(matches!(err, AuthError::MalformedHeader));
    }

    #[test]
    fn extract_bearer_lowercase_bearer_is_malformed() {
        let mut headers = HeaderMap::new();
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("bearer abc123"),
        );
        let err = extract_bearer(&headers).unwrap_err();
        assert!(matches!(err, AuthError::MalformedHeader));
    }
}
