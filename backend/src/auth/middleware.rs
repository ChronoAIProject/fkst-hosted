//! Axum middleware that builds the proxy-trusted `AuthContext` from the
//! NyxID-injected identity headers, plus a `FromRequestParts` extractor.
//!
//! No token is verified here: the NyxID proxy already authenticated the caller
//! and injected the identity. This middleware decodes/reads that identity and
//! makes it available to handlers (see [`super::identity`] for the trust
//! rationale).

use axum::body::Body;
use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use axum::http::{HeaderMap, Request};
use axum::middleware::Next;
use axum::response::Response;
use secrecy::SecretString;
use utoipa_axum::router::OpenApiRouter;

use crate::error::AppError;
use crate::state::AppState;

use super::identity::decode_identity_token;
use super::AuthContext;
use super::AuthError;

/// Header carrying the signed identity JWT injected by the NyxID proxy. Decoded
/// (not verified) into the full identity, including `permissions[]`.
const HEADER_IDENTITY_TOKEN: &str = "X-NyxID-Identity-Token";
/// Fallback header carrying just the user id ("headers mode": no RBAC).
const HEADER_USER_ID: &str = "X-NyxID-User-Id";
/// Fallback header carrying the user email.
const HEADER_USER_EMAIL: &str = "X-NyxID-User-Email";
/// Fallback header carrying the user display name.
const HEADER_USER_NAME: &str = "X-NyxID-User-Name";

/// Wrap a router with the proxy-trusted identity middleware. Every request
/// passing through must carry a NyxID-injected identity (an identity token or
/// at least the `X-NyxID-User-Id` header).
///
/// Operates on the [`OpenApiRouter`] (not a bare `axum::Router`) so the protected
/// `/api/v1` surface keeps contributing its `#[utoipa::path]` operations to the
/// generated spec while the identity layer is applied — the layer is transparent
/// to the collected paths.
pub fn protect(router: OpenApiRouter<AppState>) -> OpenApiRouter<AppState> {
    router.layer(axum::middleware::from_fn(auth_fn))
}

/// Middleware: build the `AuthContext` from the injected identity and insert it
/// into request extensions for the `FromRequestParts` extractor to pick up.
async fn auth_fn(request: Request<Body>, next: Next) -> Result<Response, AppError> {
    let auth_ctx = build_context(request.headers())?;
    let mut request = request;
    request.extensions_mut().insert(auth_ctx);
    Ok(next.run(request).await)
}

/// Read a header as a `&str`, ignoring values that are not valid UTF-8.
fn header_str<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// Capture the caller's raw bearer token (for NyxID org lookups + downstream
/// forwarding only — never for authentication). The proxy strips it by default,
/// so this is commonly absent ("headers mode").
fn extract_bearer(headers: &HeaderMap) -> Option<SecretString> {
    header_str(headers, axum::http::header::AUTHORIZATION.as_str())
        .and_then(|h| h.strip_prefix("Bearer "))
        .filter(|t| !t.is_empty())
        .map(|t| SecretString::new(t.to_string().into()))
}

/// Build the `AuthContext` from the injected identity headers.
///
/// Resolution order (mirrors Ornn's `nyxidAuth`):
/// 1. `X-NyxID-Identity-Token` present and decodable → full identity (roles +
///    permissions + groups).
/// 2. else `X-NyxID-User-Id` present → "headers mode": identity with EMPTY
///    roles/permissions (so RBAC denies every gated action; only owner-path /
///    legacy access remains).
/// 3. else → `MissingIdentity` (401).
fn build_context(headers: &HeaderMap) -> Result<AuthContext, AuthError> {
    let user_access_token = extract_bearer(headers);

    if let Some(raw) = header_str(headers, HEADER_IDENTITY_TOKEN) {
        if let Some(claims) = decode_identity_token(raw) {
            let email = claims.email.unwrap_or_default();
            let display_name = claims
                .name
                .filter(|n| !n.is_empty())
                .or_else(|| (!email.is_empty()).then(|| email.clone()))
                .unwrap_or_else(|| claims.sub.clone());
            return Ok(AuthContext {
                user_id: claims.sub,
                email,
                display_name,
                roles: claims.roles,
                permissions: claims.permissions,
                groups: claims.groups,
                user_access_token,
            });
        }
        // A present-but-undecodable identity token means the proxy contract is
        // broken; do NOT silently fall through to headers mode (which would
        // grant headers-mode access with no permissions). Log and reject.
        tracing::warn!("identity token present but undecodable; rejecting request");
        return Err(AuthError::MissingIdentity);
    }

    if let Some(user_id) = header_str(headers, HEADER_USER_ID) {
        let email = header_str(headers, HEADER_USER_EMAIL)
            .unwrap_or_default()
            .to_string();
        let display_name = header_str(headers, HEADER_USER_NAME)
            .filter(|n| !n.is_empty())
            .map(str::to_string)
            .or_else(|| (!email.is_empty()).then(|| email.clone()))
            .unwrap_or_else(|| user_id.to_string());
        return Ok(AuthContext {
            user_id: user_id.to_string(),
            email,
            display_name,
            // Headers mode carries no permissions: every gated action is denied.
            roles: vec![],
            permissions: vec![],
            groups: vec![],
            user_access_token,
        });
    }

    Err(AuthError::MissingIdentity)
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
    use axum::http::HeaderValue;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    use base64::Engine as _;
    use secrecy::ExposeSecret;

    /// Build a JWT-shaped identity token (header + base64url payload + an
    /// arbitrary unverified signature). The signature is never checked.
    fn identity_token(payload: &serde_json::Value) -> String {
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256","typ":"JWT"}"#);
        let body = URL_SAFE_NO_PAD.encode(payload.to_string().as_bytes());
        format!("{header}.{body}.unverified-signature")
    }

    #[test]
    fn identity_token_yields_roles_and_permissions() {
        let mut headers = HeaderMap::new();
        let token = identity_token(&serde_json::json!({
            "sub": "u-1",
            "email": "u@example.com",
            "name": "User",
            "roles": ["org-admin"],
            "permissions": ["fkst:goal:read"],
            "groups": ["g"],
        }));
        headers.insert(
            HEADER_IDENTITY_TOKEN,
            HeaderValue::from_str(&token).unwrap(),
        );

        let ctx = build_context(&headers).expect("identity decodes");
        assert_eq!(ctx.user_id, "u-1");
        assert_eq!(ctx.email, "u@example.com");
        assert_eq!(ctx.display_name, "User");
        assert_eq!(ctx.roles, vec!["org-admin"]);
        assert_eq!(ctx.permissions, vec!["fkst:goal:read"]);
        assert_eq!(ctx.groups, vec!["g"]);
        assert!(ctx.user_access_token.is_none());
    }

    #[test]
    fn bad_signature_identity_token_is_still_accepted() {
        // The whole point of trusting the proxy: a syntactically valid payload
        // is accepted regardless of the (unchecked) signature segment.
        let mut headers = HeaderMap::new();
        let header = URL_SAFE_NO_PAD.encode(br#"{"alg":"RS256"}"#);
        let body = URL_SAFE_NO_PAD.encode(
            serde_json::json!({ "sub": "trusted" })
                .to_string()
                .as_bytes(),
        );
        let token = format!("{header}.{body}.GARBAGE-SIGNATURE");
        headers.insert(
            HEADER_IDENTITY_TOKEN,
            HeaderValue::from_str(&token).unwrap(),
        );

        let ctx = build_context(&headers).expect("trusted payload accepted");
        assert_eq!(ctx.user_id, "trusted");
    }

    #[test]
    fn header_fallback_yields_empty_roles_and_permissions() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_USER_ID, HeaderValue::from_static("u-2"));
        headers.insert(
            HEADER_USER_EMAIL,
            HeaderValue::from_static("u2@example.com"),
        );

        let ctx = build_context(&headers).expect("headers-mode context");
        assert_eq!(ctx.user_id, "u-2");
        assert_eq!(ctx.email, "u2@example.com");
        assert_eq!(ctx.display_name, "u2@example.com");
        assert!(ctx.roles.is_empty(), "headers mode carries no roles");
        assert!(
            ctx.permissions.is_empty(),
            "headers mode carries no permissions (RBAC denies)"
        );
    }

    #[test]
    fn neither_identity_nor_user_id_is_missing_identity() {
        let headers = HeaderMap::new();
        let err = build_context(&headers).expect_err("must reject");
        assert!(matches!(err, AuthError::MissingIdentity));
    }

    #[test]
    fn undecodable_identity_token_is_rejected_not_downgraded() {
        // A present but junk identity token must NOT silently downgrade to a
        // headers-mode context even when X-NyxID-User-Id is also present.
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_IDENTITY_TOKEN, HeaderValue::from_static("not-a-jwt"));
        headers.insert(HEADER_USER_ID, HeaderValue::from_static("u-3"));
        let err = build_context(&headers).expect_err("must reject");
        assert!(matches!(err, AuthError::MissingIdentity));
    }

    #[test]
    fn bearer_token_is_captured_for_forwarding() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_USER_ID, HeaderValue::from_static("u-4"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer the-user-token"),
        );
        let ctx = build_context(&headers).expect("context");
        assert_eq!(
            ctx.user_access_token
                .as_ref()
                .map(|t| t.expose_secret().to_string()),
            Some("the-user-token".to_string())
        );
    }

    #[test]
    fn malformed_bearer_is_ignored() {
        let mut headers = HeaderMap::new();
        headers.insert(HEADER_USER_ID, HeaderValue::from_static("u-5"));
        headers.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Basic abc"),
        );
        let ctx = build_context(&headers).expect("context");
        assert!(ctx.user_access_token.is_none());
    }
}
