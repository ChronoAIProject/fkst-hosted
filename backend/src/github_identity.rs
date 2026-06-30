//! GitHub-token identity verification + the [`GithubUser`] axum extractor.
//!
//! The per-user environment/secret store (PR4a) is keyed by the caller's
//! immutable numeric GitHub **id**, and identity is the GitHub token itself:
//! every protected endpoint carries `Authorization: Bearer <github token>`. The
//! control plane trades that token for the verified `{ login, id }` by calling
//! `GET {github_api_base}/user` once — the token needs no scopes, is never
//! stored, and is used solely to learn *who* is calling.
//!
//! Because the `id` comes from the verified `/user` response (NEVER from the
//! request path or body), a caller can only ever touch their OWN
//! `fkst-user-<id>` objects: there is no client-supplied identifier to forge.
//!
//! A missing/invalid token is `401`; an unreachable/misbehaving GitHub is `503`
//! (a dependency failure, distinct from "the caller is unauthenticated").

use std::sync::OnceLock;

use axum::extract::FromRequestParts;
use axum::http::request::Parts;
use serde::Deserialize;

use crate::error::AppError;
use crate::state::AppState;

/// Request timeout for the `/user` identity check (mirrors the GitHub App
/// transport's 20s budget).
const REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(20);

/// The verified GitHub identity behind a request.
///
/// `id` is the user's immutable numeric GitHub id — logins are renamable, so the
/// store keys off the id. This type is the *verified* identity (never
/// client-supplied) and is deliberately NOT a request/response DTO: it never
/// appears in the OpenAPI components and is never serialized back to a client.
#[derive(Debug, Clone, Deserialize)]
pub struct GithubUser {
    /// The GitHub login (handle). Stamped as a label for readability only.
    pub login: String,
    /// The immutable numeric GitHub id. Determines the `fkst-user-<id>` objects.
    pub id: i64,
}

/// A pooled HTTP client for the identity check. Built once; `reqwest::Client`
/// holds a connection pool, so a per-request rebuild would lose keep-alive.
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            // GitHub rejects requests without a User-Agent.
            .user_agent("fkst-hosted")
            .build()
            .expect("build github identity http client")
    })
}

/// Verify a GitHub token by trading it for the caller's identity.
///
/// Calls `GET {base_url}/user` with `Authorization: Bearer <token>`:
/// - `200` → the parsed `{ login, id }`.
/// - `401` (or any other 4xx) → [`AppError::Unauthorized`] (token rejected); the
///   distinction is irrelevant to the caller — either way they are not
///   authenticated.
/// - `5xx` / transport / unparseable body → [`AppError::Unavailable`] (GitHub is
///   the failing dependency, not the caller).
///
/// The token is used here and dropped; it is never logged or stored.
pub async fn verify_token(base_url: &str, token: &str) -> Result<GithubUser, AppError> {
    let url = format!("{}/user", base_url.trim_end_matches('/'));
    let response = http_client()
        .get(&url)
        .header(reqwest::header::ACCEPT, "application/vnd.github+json")
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| {
            // Never log the token; only the transport error.
            tracing::warn!(error = %e, "github identity check transport error");
            AppError::Unavailable("github identity check failed (upstream unreachable)".to_string())
        })?;

    let status = response.status();
    if status == reqwest::StatusCode::UNAUTHORIZED {
        return Err(AppError::Unauthorized("github token rejected".to_string()));
    }
    if status.is_server_error() {
        return Err(AppError::Unavailable(
            "github identity check failed (upstream error)".to_string(),
        ));
    }
    if !status.is_success() {
        // Any other non-2xx (e.g. 403): the token did not yield a usable identity.
        return Err(AppError::Unauthorized("github token rejected".to_string()));
    }

    response.json::<GithubUser>().await.map_err(|e| {
        tracing::warn!(error = %e, "github /user response did not parse");
        AppError::Unavailable("github identity check failed (bad upstream response)".to_string())
    })
}

/// Pull the bearer token out of the `Authorization` header, or 401.
fn bearer_token(parts: &Parts) -> Result<&str, AppError> {
    let header = parts
        .headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            AppError::Unauthorized(
                "missing Authorization: Bearer <github token> header".to_string(),
            )
        })?;
    // Accept either casing of the scheme; reject an empty token.
    header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            AppError::Unauthorized(
                "Authorization header must be 'Bearer <github token>'".to_string(),
            )
        })
}

#[async_trait::async_trait]
impl FromRequestParts<AppState> for GithubUser {
    type Rejection = AppError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let token = bearer_token(parts)?;
        verify_token(&state.config.github_api_base_url, token).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use axum::http::Request;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn state_with_base(base_url: &str) -> AppState {
        let config = Config {
            github_api_base_url: base_url.to_string(),
            ..Config::default()
        };
        AppState {
            config,
            github_app: None,
            github_app_webhook_secret: None,
        }
    }

    fn parts_with_auth(value: Option<&str>) -> Parts {
        let mut builder = Request::builder().uri("/api/v1/users/me/env");
        if let Some(v) = value {
            builder = builder.header(axum::http::header::AUTHORIZATION, v);
        }
        builder
            .body(axum::body::Body::empty())
            .expect("request builds")
            .into_parts()
            .0
    }

    #[tokio::test]
    async fn verify_token_returns_identity_on_200() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer gho_validtoken"))
            .and(header("accept", "application/vnd.github+json"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "login": "octocat",
                "id": 583231,
                "name": "The Octocat"
            })))
            .expect(1)
            .mount(&server)
            .await;

        let user = verify_token(&server.uri(), "gho_validtoken")
            .await
            .expect("valid token verifies");
        assert_eq!(user.login, "octocat");
        assert_eq!(user.id, 583231);
    }

    #[tokio::test]
    async fn verify_token_maps_401_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(401))
            .mount(&server)
            .await;

        let err = verify_token(&server.uri(), "bad")
            .await
            .expect_err("401 must reject");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn verify_token_maps_other_4xx_to_unauthorized() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(403))
            .mount(&server)
            .await;

        let err = verify_token(&server.uri(), "bad")
            .await
            .expect_err("403 must reject");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn verify_token_maps_5xx_to_unavailable() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let err = verify_token(&server.uri(), "tok")
            .await
            .expect_err("5xx is a dependency failure");
        assert!(matches!(err, AppError::Unavailable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn extractor_accepts_a_valid_bearer_token() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/user"))
            .and(header("authorization", "Bearer gho_xyz"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_json(serde_json::json!({ "login": "alice", "id": 42 })),
            )
            .mount(&server)
            .await;

        let state = state_with_base(&server.uri());
        let mut parts = parts_with_auth(Some("Bearer gho_xyz"));
        let user = GithubUser::from_request_parts(&mut parts, &state)
            .await
            .expect("extractor verifies the token");
        assert_eq!(user.id, 42);
        assert_eq!(user.login, "alice");
    }

    #[tokio::test]
    async fn extractor_rejects_a_missing_authorization_header() {
        // No server call should happen — the header is absent.
        let state = state_with_base("http://127.0.0.1:1");
        let mut parts = parts_with_auth(None);
        let err = GithubUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("missing header must 401");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn extractor_rejects_a_non_bearer_authorization_header() {
        let state = state_with_base("http://127.0.0.1:1");
        let mut parts = parts_with_auth(Some("Basic dXNlcjpwYXNz"));
        let err = GithubUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("non-bearer header must 401");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn extractor_rejects_an_empty_bearer_token() {
        let state = state_with_base("http://127.0.0.1:1");
        let mut parts = parts_with_auth(Some("Bearer    "));
        let err = GithubUser::from_request_parts(&mut parts, &state)
            .await
            .expect_err("empty token must 401");
        assert!(matches!(err, AppError::Unauthorized(_)), "got {err:?}");
    }
}
