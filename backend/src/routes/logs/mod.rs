//! The dual-mode, identity-gated session-log download endpoint.
//!
//! `GET /api/v1/logs/{session_id}` serves a session's redacted log bundle (uploaded
//! by the producer to chrono-storage at `logs/<session_id>/latest.tar.gz`) to — and
//! only to — a caller the session's trigger issue authorizes. It is UNAUTHENTICATED
//! at the routing layer (like the webhook): identity + authorization are enforced
//! INSIDE the handler, so the handler must be robust to junk input.
//!
//! Two modes establish identity:
//!
//! - **API mode** — an `Authorization: Bearer <github-token>` header. The token is
//!   traded for `{login, id}` via `GET {api_base}/user` (never logged, used only for
//!   that call, never stored; the lookup is cached briefly by token HASH). A rejected
//!   token → 401. On success the endpoint returns JSON `{ url, expires_in }` with a
//!   fresh 900s presigned URL, safe because it goes only to the authenticated caller.
//! - **Browser mode** — no header. The endpoint 302-redirects to GitHub user-OAuth
//!   with a SIGNED `state` carrying the `session_id` (CSRF/tamper guard); the
//!   `/api/v1/logs/oauth/callback` route verifies `state`, exchanges the code for a
//!   user token, resolves `/user`, and — on authorization — 302-redirects the browser
//!   to the presigned URL (the download starts).
//!
//! Authorization (both modes) is the pure three-tier
//! [`crate::reconcile::log_authz::is_authorized`] over the session's trigger context,
//! looked up in the reconciler-maintained [`crate::log_access`] registry (session_id
//! is a one-way hash, so this reverse map is how the endpoint recovers the author id
//! + `### Log Access` allow-list). Deny → 403. Unknown session / missing object → 404.
//!
//! Secret hygiene: the caller's token and the OAuth client secret are NEVER logged;
//! the presigned URL is a short-lived capability handed only to the resolved caller.

mod identity;
mod oauth;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use secrecy::ExposeSecret;
use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use std::time::Duration;
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::log_config::LogConfig;
use crate::reconcile::log_authz;
use crate::state::AppState;
use crate::storage::StorageError;

/// The lifetime requested for a minted presigned GET URL, in seconds (15 minutes) —
/// long enough for a browser download to start, short enough that a leaked URL
/// expires quickly. Reported to API callers as `expires_in`.
const PRESIGN_TTL_SECS: u64 = 900;

/// The chrono-storage object key the producer uploads a session's redacted bundle to.
fn log_object_key(session_id: &str) -> String {
    format!("logs/{session_id}/latest.tar.gz")
}

/// The JSON body an API-mode (Bearer) caller receives: a short-lived presigned GET
/// URL for the session's redacted log bundle plus its lifetime in seconds.
#[derive(Debug, Serialize, ToSchema)]
pub struct LogDownloadResponse {
    /// A freshly-minted, short-lived presigned URL that downloads the redacted bundle.
    #[schema(example = "https://storage.example/logs/<id>/latest.tar.gz?sig=...")]
    pub url: String,
    /// The presigned URL's lifetime in seconds.
    #[schema(example = 900)]
    pub expires_in: u64,
}

/// The OAuth callback query (`?code=&state=` on success, `?error=` on user denial).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct OAuthCallbackQuery {
    /// The one-time OAuth code GitHub returns (absent on an error redirect).
    #[serde(default)]
    code: Option<String>,
    /// The signed `state` value the endpoint issued (carries the `session_id`).
    #[serde(default)]
    state: Option<String>,
    /// GitHub's error slug when the user denied the authorization (e.g. `access_denied`).
    #[serde(default)]
    error: Option<String>,
}

/// `GET /api/v1/logs/{session_id}` — download a session's redacted logs.
///
/// UNAUTHENTICATED at the routing layer; identity + authorization run in-handler
/// (see the module docs). With a Bearer token it resolves identity and returns a
/// presigned-URL JSON body; without one it 302-redirects into the browser OAuth flow.
#[utoipa::path(
    get,
    path = "/logs/{session_id}",
    tag = "logs",
    operation_id = "download_session_logs",
    params(("session_id" = String, Path, description = "The deterministic session id (from the announce link)")),
    responses(
        (status = 200, description = "Presigned download URL (API mode — a Bearer token was supplied)", body = LogDownloadResponse),
        (status = 302, description = "Redirect: browser mode → GitHub OAuth; authorized → the presigned URL"),
        (status = 401, description = "The supplied Bearer token was rejected by GitHub", body = ErrorEnvelope),
        (status = 403, description = "Authenticated but not authorized to access these logs", body = ErrorEnvelope),
        (status = 404, description = "Unknown session, or no logs retained yet", body = ErrorEnvelope),
        (status = 503, description = "Log storage / browser login not configured", body = ErrorEnvelope),
    )
)]
async fn download_session_logs(
    State(state): State<AppState>,
    Path(session_id): Path<String>,
    headers: HeaderMap,
) -> Response {
    match bearer_token(&headers) {
        // API mode: a Bearer token is present — resolve identity + serve JSON.
        Some(token) => api_mode(&state, &session_id, &token).await,
        // Browser mode: no token — redirect into the GitHub OAuth flow.
        None => browser_redirect(&state, &session_id),
    }
}

/// `GET /api/v1/logs/oauth/callback` — the browser-mode OAuth return.
///
/// Verifies the signed `state`, exchanges the `code` for a user token, resolves the
/// caller's identity, authorizes, and 302-redirects to the presigned URL. Every
/// failure renders a browser-friendly HTML page (never a token in the URL or body).
#[utoipa::path(
    get,
    path = "/logs/oauth/callback",
    tag = "logs",
    operation_id = "session_logs_oauth_callback",
    params(OAuthCallbackQuery),
    responses(
        (status = 302, description = "Authorized → redirect to the presigned download URL"),
        (status = 400, description = "Missing or tampered OAuth state/code (HTML)"),
        (status = 403, description = "Authenticated but not authorized (HTML)"),
        (status = 404, description = "No logs retained yet (HTML)"),
    )
)]
async fn oauth_callback(
    State(state): State<AppState>,
    Query(query): Query<OAuthCallbackQuery>,
) -> Response {
    let log = &state.config.log;
    // Browser login must be configured to have issued the redirect in the first place.
    let Some((client_id, secret)) = oauth_creds(log) else {
        return html_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Browser login is not configured.",
        );
    };
    let Some(base) = log.public_base_url.as_deref() else {
        return html_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Browser login is not configured.",
        );
    };

    // The user denied the authorization on GitHub's consent screen.
    if query.error.is_some() {
        return html_error(StatusCode::FORBIDDEN, "GitHub authorization was denied.");
    }
    let (Some(code), Some(state_param)) = (
        query.code.filter(|c| !c.is_empty()),
        query.state.filter(|s| !s.is_empty()),
    ) else {
        return html_error(StatusCode::BAD_REQUEST, "Missing OAuth code or state.");
    };
    // Verify the signed state (CSRF/tamper guard) and recover the session id.
    let Some(session_id) = oauth::verify_state(secret.expose_secret().as_bytes(), &state_param)
    else {
        return html_error(StatusCode::BAD_REQUEST, "Invalid or tampered OAuth state.");
    };

    let redirect_uri = callback_redirect_uri(base);
    let token = match oauth::exchange_code(
        http_client(),
        &log.oauth_base_url,
        client_id,
        secret,
        &code,
        &redirect_uri,
    )
    .await
    {
        Ok(token) => token,
        Err(err) => return browser_error(err),
    };

    let user =
        match identity::resolve(&state.config.github_api_base_url, token.expose_secret()).await {
            Ok(user) => user,
            Err(_) => {
                return html_error(
                    StatusCode::UNAUTHORIZED,
                    "Could not verify your GitHub identity.",
                )
            }
        };

    // Authorize, then 302 to the presigned URL; render every failure as HTML.
    if let Err(err) = authorize(&state, &session_id, &user) {
        return browser_error(err);
    }
    match presign(&state, &session_id).await {
        Ok(url) => redirect_302(&url),
        Err(err) => browser_error(err),
    }
}

// ---- API mode + browser redirect --------------------------------------------

/// API mode: resolve identity from the Bearer `token`, authorize, and return the
/// presigned-URL JSON. Every failure renders the JSON [`AppError`] envelope.
async fn api_mode(state: &AppState, session_id: &str, token: &str) -> Response {
    let user = match identity::resolve(&state.config.github_api_base_url, token).await {
        Ok(user) => user,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = authorize(state, session_id, &user) {
        return err.into_response();
    }
    match presign(state, session_id).await {
        Ok(url) => Json(LogDownloadResponse {
            url,
            expires_in: PRESIGN_TTL_SECS,
        })
        .into_response(),
        Err(err) => err.into_response(),
    }
}

/// Browser mode: 302-redirect into GitHub user-OAuth, carrying a signed `state`. When
/// browser login is unconfigured, tell the caller to use a Bearer token instead.
fn browser_redirect(state: &AppState, session_id: &str) -> Response {
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(base)) =
        (oauth_creds(log), log.public_base_url.as_deref())
    else {
        return AppError::Unavailable(
            "browser login is not configured for log downloads; pass an \
             'Authorization: Bearer <github-token>' header instead"
                .to_string(),
        )
        .into_response();
    };
    let redirect_uri = callback_redirect_uri(base);
    let state_param = oauth::sign_state(secret.expose_secret().as_bytes(), session_id);
    match oauth::authorize_url(&log.oauth_base_url, client_id, &redirect_uri, &state_param) {
        Ok(url) => redirect_302(&url),
        Err(err) => err.into_response(),
    }
}

// ---- Shared authorize + serve -----------------------------------------------

/// Authorize `user` against the session's trigger context. Looks the context up in
/// the reconciler-maintained registry (a one-way `session_id` cannot yield it
/// otherwise); an unknown session → 404 (never reveals more), an unauthorized caller
/// → 403. The token is NEVER referenced here; only the resolved (public) identity is.
fn authorize(state: &AppState, session_id: &str, user: &GithubUser) -> Result<(), AppError> {
    let Some(context) = state.log_registry.get(session_id) else {
        // Deny-by-default: with no context we cannot authorize, so we do not serve.
        return Err(AppError::NotFound(
            "no logs available for this session".to_string(),
        ));
    };
    if log_authz::is_authorized(
        user.id,
        &user.login,
        context.author_id,
        &context.log_access,
        &state.config.log.admins,
    ) {
        tracing::info!(
            session_id = %session_id,
            requester_id = user.id,
            requester_login = %user.login,
            "log download authorized"
        );
        Ok(())
    } else {
        tracing::info!(
            session_id = %session_id,
            requester_id = user.id,
            requester_login = %user.login,
            "log download denied (not authorized)"
        );
        Err(AppError::Forbidden(
            "not authorized to access these logs".to_string(),
        ))
    }
}

/// Mint a fresh 900s presigned GET URL for the session's log bundle. A missing object
/// → 404 (`no logs available yet`); no storage configured → 503; any other storage
/// error → 502. The signed URL is returned (to the resolved caller) and never logged.
async fn presign(state: &AppState, session_id: &str) -> Result<String, AppError> {
    let Some(storage) = state.storage.as_ref() else {
        return Err(AppError::Unavailable(
            "log storage is not configured".to_string(),
        ));
    };
    let key = log_object_key(session_id);
    match storage.presigned_get_url(&key, PRESIGN_TTL_SECS).await {
        Ok(url) => Ok(url),
        Err(StorageError::Status { status: 404 }) => {
            Err(AppError::NotFound("no logs available yet".to_string()))
        }
        Err(err) => {
            // The error carries only a numeric status / URL-free category — never the
            // key, the signed URL, or the SA token.
            tracing::warn!(session_id = %session_id, error = %err, "log presign failed");
            Err(AppError::Upstream("log storage error".to_string()))
        }
    }
}

// ---- Small helpers ----------------------------------------------------------

/// The `(client_id, client_secret)` pair, present only when BOTH are configured
/// (the config layer enforces the all-or-nothing invariant; this is defensive).
fn oauth_creds(log: &LogConfig) -> Option<(&str, &secrecy::SecretString)> {
    match (
        log.oauth_client_id.as_deref(),
        log.oauth_client_secret.as_ref(),
    ) {
        (Some(id), Some(secret)) => Some((id, secret)),
        _ => None,
    }
}

/// The OAuth `redirect_uri`: `<public_base>/api/v1/logs/oauth/callback`.
fn callback_redirect_uri(public_base: &str) -> String {
    format!(
        "{}/api/v1/logs/oauth/callback",
        public_base.trim_end_matches('/')
    )
}

/// Extract a non-empty bearer token from the `Authorization` header (either casing of
/// the scheme). `None` when the header is absent, non-bearer, or empty — that steers
/// the request into browser mode rather than erroring.
fn bearer_token(headers: &HeaderMap) -> Option<String> {
    let value = headers.get(header::AUTHORIZATION)?.to_str().ok()?;
    let token = value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))?
        .trim();
    (!token.is_empty()).then(|| token.to_string())
}

/// A 302 redirect to `location`. An un-encodable location (never from our own URLs)
/// renders a 500 rather than panicking.
fn redirect_302(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(value) => (StatusCode::FOUND, [(header::LOCATION, value)]).into_response(),
        Err(_) => AppError::Internal(anyhow::anyhow!("invalid redirect location")).into_response(),
    }
}

/// A browser-friendly HTML error page (fixed, escaping-free message text).
fn html_error(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{code}</title></head>\
         <body><h1>{code}</h1><p>{message}</p></body></html>",
        code = status.as_u16()
    );
    (
        status,
        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
        body,
    )
        .into_response()
}

/// Map an [`AppError`] to a browser-friendly HTML error page (the browser paths never
/// render the JSON envelope). The message is a fixed, client-safe string per tier.
fn browser_error(err: AppError) -> Response {
    let (status, message) = match err {
        AppError::NotFound(_) => (
            StatusCode::NOT_FOUND,
            "No logs are available for this session yet.",
        ),
        AppError::Forbidden(_) => (
            StatusCode::FORBIDDEN,
            "You are not authorized to access these logs.",
        ),
        AppError::Unauthorized(_) => (
            StatusCode::UNAUTHORIZED,
            "Could not verify your GitHub identity.",
        ),
        AppError::Unavailable(_) => (
            StatusCode::SERVICE_UNAVAILABLE,
            "Log download is temporarily unavailable.",
        ),
        _ => (StatusCode::BAD_GATEWAY, "Log download failed."),
    };
    html_error(status, message)
}

/// A pooled HTTP client for the OAuth token exchange (bounded timeout + User-Agent).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("fkst-hosted")
            .build()
            .expect("build log-oauth http client")
    })
}

/// The log-download router (nested under `/api/v1`). Open at the app layer — both
/// identity and authorization are enforced INSIDE each handler (GitHub token or
/// OAuth), so there is no documented security scheme (like the webhook).
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(download_session_logs))
        .routes(routes!(oauth_callback))
}

#[cfg(test)]
mod test_support;
#[cfg(test)]
#[path = "tests.rs"]
mod tests;
#[cfg(test)]
#[path = "tests_browser.rs"]
mod tests_browser;
