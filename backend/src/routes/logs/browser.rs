//! Browser mode: the GitHub user-OAuth round-trip that lets a person download a
//! session's logs from a link, with no API token in hand.
//!
//! A bare `GET /api/v1/logs/{session_id}` (no `Authorization` header) cannot
//! prove who is asking, so instead of failing it 302-redirects into GitHub's
//! consent screen carrying a SIGNED `state` that holds the session id (the CSRF
//! and tamper guard — the session id is the only thing that must survive the
//! round-trip). GitHub returns the browser to
//! `GET /api/v1/logs/oauth/callback`, which verifies that state, exchanges the
//! one-time code for a user token, resolves `/user` to a verified `{login, id}`,
//! and runs the SAME [`super::authorize`] gate the API mode runs before
//! streaming the bundle back as an attachment.
//!
//! ## Why this is its own module
//!
//! Everything here is browser-shaped: HTML error pages instead of the JSON
//! envelope, 302s instead of status codes, and an OAuth state machine the API
//! mode has no part in. Keeping it beside — rather than inside — the endpoint
//! module leaves `super` holding just the two entry points and the shared
//! server-side fetch.
//!
//! ## Secret hygiene
//!
//! The exchanged token is used for exactly one `GET /user` call and is never
//! logged, stored, or echoed; the OAuth client secret doubles as the state HMAC
//! key and never leaves this process; and no presigned storage URL is ever
//! handed to the caller — the bytes are fetched server-side.

use axum::extract::State;
use axum::http::{header, Extensions, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use secrecy::ExposeSecret;
use serde::Deserialize;
use std::sync::OnceLock;
use std::time::Duration;
use utoipa::IntoParams;

use super::{authorize, identity, oauth, stream_download};
use crate::audit::arguments::auth::{OauthResult, SessionLogsCallbackInput};
use crate::audit::arguments::{record, AuditedQuery};
use crate::error::AppError;
use crate::log_config::LogConfig;
use crate::state::AppState;

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
pub(super) async fn oauth_callback(
    State(state): State<AppState>,
    extensions: Extensions,
    AuditedQuery(query): AuditedQuery<OAuthCallbackQuery>,
) -> Response {
    let (response, session_id, result) = log_oauth_callback(&state, &extensions, query).await;
    // `session_id` is `Some` only once the state's HMAC verified: a tampered
    // state names no session, and extracting one from it would let anyone attach
    // their failed callback to a session they never had access to.
    record(
        &extensions,
        &SessionLogsCallbackInput {
            verified_session_id: session_id.as_deref(),
            result,
        },
    );
    if let Some(session_id) = &session_id {
        super::record_session_correlation(&extensions, session_id);
    }
    response
}

/// The callback's real body, returning the VERIFIED session id and the outcome
/// alongside the response so the single recording site above cannot be missed on
/// a new early return.
async fn log_oauth_callback(
    state: &AppState,
    extensions: &Extensions,
    query: OAuthCallbackQuery,
) -> (Response, Option<String>, OauthResult) {
    let log = &state.config.log;
    // Browser login must be configured to have issued the redirect in the first place.
    let Some((client_id, secret)) = oauth_creds(log) else {
        return (
            html_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Browser login is not configured.",
            ),
            None,
            OauthResult::UpstreamError,
        );
    };
    let Some(base) = log.public_base_url.as_deref() else {
        return (
            html_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Browser login is not configured.",
            ),
            None,
            OauthResult::UpstreamError,
        );
    };

    // The user denied the authorization on GitHub's consent screen.
    if query.error.is_some() {
        return (
            html_error(StatusCode::FORBIDDEN, "GitHub authorization was denied."),
            None,
            OauthResult::Denied,
        );
    }
    let (Some(code), Some(state_param)) = (
        query.code.filter(|c| !c.is_empty()),
        query.state.filter(|s| !s.is_empty()),
    ) else {
        return (
            html_error(StatusCode::BAD_REQUEST, "Missing OAuth code or state."),
            None,
            OauthResult::Invalid,
        );
    };
    // Verify the signed state (CSRF/tamper guard) and recover the session id.
    let Some(session_id) = oauth::verify_state(secret.expose_secret().as_bytes(), &state_param)
    else {
        return (
            html_error(StatusCode::BAD_REQUEST, "Invalid or tampered OAuth state."),
            None,
            OauthResult::Invalid,
        );
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
        Err(err) => {
            return (
                browser_error(err),
                Some(session_id),
                OauthResult::UpstreamError,
            )
        }
    };

    // The exchanged token is not an identity until `GET /user` names its owner: a
    // failure here is a stable authentication failure, never an invented actor.
    let user =
        match identity::resolve(&state.config.github_api_base_url, token.expose_secret()).await {
            Ok(user) => user,
            Err(_) => {
                return (
                    html_error(
                        StatusCode::UNAUTHORIZED,
                        "Could not verify your GitHub identity.",
                    ),
                    Some(session_id),
                    OauthResult::UpstreamError,
                )
            }
        };
    crate::audit::identity::record_identity(
        extensions,
        crate::audit::AuditIdentity::github_oauth(user.id, user.login.clone()),
    );

    // Authorize, then stream the latest bundle; render every failure as HTML. The
    // browser path serves the latest bundle only (the run selector is not carried
    // through the OAuth round-trip).
    // The recorded `result` describes the FLOW, not just its OAuth leg: a
    // refused or failed download is not a successful session-logs flow, and
    // `result` is a dashboard facet where "success" on a denied log download
    // would be actively misleading. An authorization refusal is `denied` (the
    // same word GitHub's own consent refusal earns — both mean "this caller
    // does not get this"); a bundle read that could not be served is
    // `upstream_error`. The HTTP status each renders is unchanged.
    if let Err(err) = authorize(state, &session_id, &user) {
        return (browser_error(err), Some(session_id), OauthResult::Denied);
    }
    match stream_download(state, &session_id, None).await {
        Ok(response) => (response, Some(session_id), OauthResult::Success),
        Err(err) => (
            browser_error(err),
            Some(session_id),
            OauthResult::UpstreamError,
        ),
    }
}

/// Browser mode entry: 302-redirect into GitHub user-OAuth, carrying a signed
/// `state`. When browser login is unconfigured, tell the caller to use a Bearer
/// token instead.
pub(super) fn browser_redirect(state: &AppState, session_id: &str) -> Response {
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
    match oauth::authorize_url(
        &log.oauth_base_url,
        client_id,
        &redirect_uri,
        &state_param,
        None,
    ) {
        Ok(url) => redirect_302(&url),
        Err(err) => err.into_response(),
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
    // No JSON envelope on the browser paths, so the stable audit code — and, for
    // a 401/403, the policy-rejection marker — travel as typed response
    // extensions instead. Without the marker the SAME authorization denial would
    // record as `rejected` for a Bearer caller and as a plain `client_error` for
    // a browser one (see `crate::audit::request::response`).
    crate::audit::request::with_browser_error(
        (
            status,
            [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
            body,
        )
            .into_response(),
        status,
    )
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
