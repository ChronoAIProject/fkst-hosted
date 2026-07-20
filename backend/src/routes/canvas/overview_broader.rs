//! Broader-visibility enumeration-token resolution for the canvas overview
//! (issue #572 · R1b).
//!
//! The overview's repo/org enumeration normally rides the App user-to-server token,
//! which GitHub scopes to installed repos/orgs. When the caller connects the broader
//! classic-OAuth credential (`crate::routes::auth_broader`) the SPA forwards that token
//! on `GET /api/v1/overview` via the [`BROADER_TOKEN_HEADER`]. This module resolves
//! WHICH token drives enumeration, enforcing the mandatory same-user check first: a
//! broader token is trusted ONLY when it verifies to the same GitHub id as the Bearer
//! identity. Otherwise it is ignored (redacted warning) and the App token is used —
//! so a bad or foreign broader header can only ever narrow visibility back to today's
//! behavior, never escalate to another user's repos, and never fail the request.

use axum::http::HeaderMap;
use secrecy::{ExposeSecret, SecretString};

use crate::github_identity::{verify_token, GithubUser};
use crate::state::AppState;

/// The request header carrying the OPTIONAL broader-visibility OAuth token. The value
/// is a bare token (an optional `Bearer ` prefix is tolerated); it is NEVER logged.
pub(super) const BROADER_TOKEN_HEADER: &str = "x-github-broader-token";

/// Pull the OPTIONAL broader-visibility token out of the [`BROADER_TOKEN_HEADER`],
/// tolerating an optional `Bearer ` prefix and rejecting an empty value. The token is
/// held in a [`SecretString`] so it is never accidentally logged.
fn broader_token_header(headers: &HeaderMap) -> Option<SecretString> {
    let value = headers.get(BROADER_TOKEN_HEADER)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .unwrap_or(value)
        .trim();
    (!token.is_empty()).then(|| SecretString::from(token.to_string()))
}

/// Resolve the token that drives the caller's repo/org ENUMERATION.
///
/// When the caller supplies a broader-visibility token (via [`BROADER_TOKEN_HEADER`])
/// AND it verifies to the SAME GitHub id as the Bearer identity, that broader token is
/// used — so repos/orgs where the App is not installed still appear. A broader token
/// that is absent, fails to verify, or resolves to a DIFFERENT id is IGNORED (a
/// redacted warning is logged) and the `app_token` is used instead. This never fails
/// the request, and it never returns the foreign token — the same-user check is
/// mandatory before the broader token is ever trusted.
pub(super) async fn resolve_enumeration_token(
    state: &AppState,
    user: &GithubUser,
    app_token: &SecretString,
    headers: &HeaderMap,
) -> SecretString {
    let Some(broader) = broader_token_header(headers) else {
        return app_token.clone();
    };
    match verify_token(&state.config.github_api_base_url, broader.expose_secret()).await {
        Ok(verified) if verified.id == user.id => {
            tracing::debug!(
                user_id = user.id,
                "canvas overview: using the broader-visibility token for enumeration"
            );
            broader
        }
        Ok(verified) => {
            // Same-user check failed: a broader token belonging to a different id is
            // never trusted — fall back to the App token (never the foreign token).
            tracing::warn!(
                user_id = user.id,
                broader_id = verified.id,
                "canvas overview: broader-visibility token identity mismatch; ignoring \
                 it and falling back to the app token"
            );
            app_token.clone()
        }
        Err(_) => {
            // Verification failed (rejected/unreachable): ignore the broader token and
            // degrade to the App token rather than failing the whole overview.
            tracing::warn!(
                user_id = user.id,
                "canvas overview: broader-visibility token failed verification; ignoring \
                 it and falling back to the app token"
            );
            app_token.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    fn headers_with(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(BROADER_TOKEN_HEADER, HeaderValue::from_str(value).unwrap());
        headers
    }

    #[test]
    fn absent_header_yields_none() {
        assert!(broader_token_header(&HeaderMap::new()).is_none());
    }

    #[test]
    fn bare_and_bearer_prefixed_tokens_are_both_accepted() {
        assert_eq!(
            broader_token_header(&headers_with("gho_bare"))
                .unwrap()
                .expose_secret(),
            "gho_bare"
        );
        assert_eq!(
            broader_token_header(&headers_with("Bearer gho_prefixed"))
                .unwrap()
                .expose_secret(),
            "gho_prefixed"
        );
    }

    #[test]
    fn blank_or_whitespace_header_is_treated_as_absent() {
        assert!(broader_token_header(&headers_with("   ")).is_none());
        assert!(broader_token_header(&headers_with("Bearer   ")).is_none());
    }
}
