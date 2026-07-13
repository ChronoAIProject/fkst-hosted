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
//!   token → 401. On success the endpoint STREAMS the redacted bundle back as a gzip
//!   attachment.
//! - **Browser mode** — no header. The endpoint 302-redirects to GitHub user-OAuth
//!   with a SIGNED `state` carrying the `session_id` (CSRF/tamper guard); the
//!   `/api/v1/logs/oauth/callback` route verifies `state`, exchanges the code for a
//!   user token, resolves `/user`, and — on authorization — STREAMS the bundle back as
//!   an attachment (the download starts).
//!
//! Authorization (both modes) is the pure three-tier
//! [`crate::reconcile::log_authz::is_authorized`] over the session's trigger context,
//! looked up in the reconciler-maintained [`crate::log_access`] registry (session_id
//! is a one-way hash, so this reverse map is how the endpoint recovers the author id
//! + `### Log Access Allowlist` allow-list). Deny → 403. Unknown session / missing object → 404.
//!
//! Secret hygiene: the caller's token and the OAuth client secret are NEVER logged, and
//! NO presigned S3 URL is ever exposed to the caller — the control plane fetches the
//! bundle server-side (a presigned URL is used only internally) and returns the bytes.

mod identity;
// Shared with `crate::routes::auth` (the frontend login flow reuses the signed-state
// + authorize-URL + token-exchange primitives).
pub(crate) mod oauth;

use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::log_config::LogConfig;
use crate::reconcile::log_authz;
use crate::state::AppState;
use crate::storage::StorageError;

/// The chrono-storage object key the producer uploads a session's redacted bundle to.
fn log_object_key(session_id: &str) -> String {
    format!("logs/{session_id}/latest.tar.gz")
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
/// (see the module docs). With a Bearer token it resolves identity and streams the
/// redacted bundle as a gzip attachment; without one it 302-redirects into the browser
/// OAuth flow. No presigned S3 URL is ever exposed to the caller — the control plane
/// fetches the bytes server-side and returns them.
#[utoipa::path(
    get,
    path = "/logs/{session_id}",
    tag = "logs",
    operation_id = "download_session_logs",
    params(("session_id" = String, Path, description = "The deterministic session id (from the announce link)")),
    responses(
        (status = 200, description = "The redacted log bundle, streamed as a gzip attachment (API mode — a Bearer token was supplied)", content_type = "application/gzip"),
        (status = 302, description = "Redirect: browser mode → GitHub OAuth"),
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
/// caller's identity, authorizes, and streams the redacted bundle as a gzip attachment.
/// Every failure renders a browser-friendly HTML page (never a token in the URL or body).
#[utoipa::path(
    get,
    path = "/logs/oauth/callback",
    tag = "logs",
    operation_id = "session_logs_oauth_callback",
    params(OAuthCallbackQuery),
    responses(
        (status = 200, description = "Authorized → the redacted log bundle as a gzip attachment", content_type = "application/gzip"),
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
    match stream_download(&state, &session_id).await {
        Ok(response) => response,
        Err(err) => browser_error(err),
    }
}

// ---- API mode + browser redirect --------------------------------------------

/// API mode: resolve identity from the Bearer `token`, authorize, and stream the
/// redacted bundle back as a gzip attachment — identical to the browser path, so NO
/// presigned S3 URL is ever handed to a caller (the presigned URL is used server-side
/// only, inside [`stream_download`]). Every failure renders the JSON [`AppError`] envelope.
async fn api_mode(state: &AppState, session_id: &str, token: &str) -> Response {
    let user = match identity::resolve(&state.config.github_api_base_url, token).await {
        Ok(user) => user,
        Err(err) => return err.into_response(),
    };
    if let Err(err) = authorize(state, session_id, &user) {
        return err.into_response();
    }
    match stream_download(state, session_id).await {
        Ok(response) => response,
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
pub(crate) fn authorize(
    state: &AppState,
    session_id: &str,
    user: &GithubUser,
) -> Result<(), AppError> {
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

/// Fetch the session's bundle from chrono-storage (server-side) and return it as an
/// `attachment` download. Serving it THROUGH the control plane — rather than 302-ing the
/// browser to the presigned S3 URL — means the caller only ever talks to THIS host (robust
/// for a browser on a different machine/network than the cluster), and the explicit
/// `Content-Disposition: attachment` makes the browser SAVE the bundle rather than fetch it
/// into the void (a cross-origin nav to an `application/gzip` URL lacking that header is
/// silently discarded by some browsers). API (Bearer) callers still receive a presigned URL.
async fn stream_download(state: &AppState, session_id: &str) -> Result<Response, AppError> {
    let Some(storage) = state.storage.as_ref() else {
        return Err(AppError::Unavailable(
            "log storage is not configured".to_string(),
        ));
    };
    let key = log_object_key(session_id);
    let bytes = match storage.download(&key).await {
        Ok(bytes) => bytes,
        Err(StorageError::Status { status: 404 }) => {
            return Err(AppError::NotFound("no logs available yet".to_string()));
        }
        Err(err) => {
            tracing::warn!(session_id = %session_id, error = %err, "log download failed");
            return Err(AppError::Upstream("log storage error".to_string()));
        }
    };
    let disposition = format!("attachment; filename=\"fkst-logs-{session_id}.tar.gz\"");
    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/gzip"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&disposition)
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
            ),
        ],
        bytes,
    )
        .into_response())
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
