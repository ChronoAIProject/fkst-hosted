//! GitHub user-OAuth **login** for the fkst-hosted frontend.
//!
//! Unlike the log-download browser flow (which authorizes a single session and
//! streams a download), this trio establishes a durable *frontend session*: it
//! hands the SPA a short-lived GitHub user access token (plus a refresh token when
//! the App issues expiring tokens) so the SPA can call the rest of the API as
//! `Authorization: Bearer <token>` (verified by the [`GithubUser`] extractor).
//!
//! - `GET /api/v1/auth/github/login` → 302 into GitHub's OAuth authorize page, with
//!   a signed + time-bounded `state` (CSRF/replay guard).
//! - `GET /api/v1/auth/github/callback` → verify `state`, exchange the `code` for a
//!   token set SERVER-SIDE (the client secret never leaves the backend), then 302 to
//!   the configured frontend URL with the tokens in the URL **fragment** (never a
//!   query string or a log).
//! - `POST /api/v1/auth/github/refresh` → redeem a refresh token for a fresh access
//!   token so the user stays signed in across the 8h access-token lifetime without
//!   interruption. GitHub's token endpoint has no CORS and needs the client secret,
//!   so this MUST be server-side.
//!
//! [`GithubUser`]: crate::github_identity::GithubUser

use std::sync::OnceLock;
use std::time::Duration;

use axum::extract::{Query, State};
use axum::http::{header, Extensions, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::arguments::auth::{
    OauthResult, SafeGithubLogin, SafeGithubLoginCallback, SafeGithubRefreshToken,
};
use crate::audit::arguments::{record_safe, AuditedJson};
use crate::audit::AuditIdentity;
use crate::error::{AppError, ErrorEnvelope};
use crate::github_identity::GithubUser;
use crate::log_config::LogConfig;
use crate::routes::auth_oauth_state::{
    callback_redirect_uri, frontend_success_url, login_state_message, post_install_redirect,
    state_is_fresh, token_response,
};
use crate::routes::logs::{identity, oauth};
use crate::state::AppState;

/// The OAuth login-callback query (`?code=&state=` on success, `?error=` on denial).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct LoginCallbackQuery {
    /// The one-time OAuth code GitHub returns (absent on an error redirect).
    #[serde(default)]
    code: Option<String>,
    /// The signed `state` value the login endpoint issued.
    #[serde(default)]
    state: Option<String>,
    /// GitHub's error slug when the user denied authorization (e.g. `access_denied`).
    #[serde(default)]
    error: Option<String>,
    /// Present on GitHub's POST-INSTALL redirect (`install` / `update` /
    /// `request`) — the App has "Request user authorization during
    /// installation" on, so installs land on this callback too.
    #[serde(default)]
    setup_action: Option<String>,
    /// The installation id GitHub appends on the post-install redirect.
    #[serde(default)]
    installation_id: Option<String>,
}

/// The refresh request body: the (rotating) refresh token the SPA holds.
#[derive(Debug, Deserialize, ToSchema)]
pub struct RefreshRequest {
    /// The refresh token from a prior login/refresh (GitHub rotates it each use).
    pub refresh_token: String,
}

/// A token set returned to the SPA (refresh + JSON responses). The access/refresh
/// tokens are secrets the SPA legitimately needs, so they ride the body as strings.
#[derive(Debug, Serialize, ToSchema)]
pub struct TokenResponse {
    /// The GitHub user access token to send as `Authorization: Bearer <token>`.
    pub access_token: String,
    /// The rotated refresh token to persist for the next refresh; present only when
    /// the GitHub App issues expiring user tokens.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Seconds until the access token expires (when expiring tokens are enabled).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expires_in: Option<i64>,
    /// Seconds until the refresh token itself expires (~6 months); lets the SPA
    /// prompt a fresh login before the refresh token dies.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub refresh_token_expires_in: Option<i64>,
    /// The token scheme — always `bearer`.
    #[schema(value_type = String, example = "bearer")]
    pub token_type: &'static str,
}

/// `GET /api/v1/auth/github/login` — begin GitHub user-OAuth login.
///
/// 302-redirects the browser to GitHub's authorize page with a signed, time-bounded
/// `state`. When login is not configured (`FKST_GITHUB_OAUTH_*` / `FKST_PUBLIC_BASE_URL`
/// / `FKST_FRONTEND_URL`), returns 503.
#[utoipa::path(
    get,
    path = "/auth/github/login",
    tag = "auth",
    operation_id = "github_login",
    responses(
        (status = 302, description = "Redirect to GitHub's OAuth authorize page"),
        (status = 503, description = "Frontend login is not configured", body = ErrorEnvelope),
    )
)]
async fn github_login(State(state): State<AppState>, extensions: Extensions) -> Response {
    // The flow is the whole safe argument: the response is a redirect whose
    // target carries the client id and the signed state, none of which a record
    // may hold.
    record_safe(&extensions, &SafeGithubLogin::new());
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(public_base), Some(_frontend)) = (
        oauth_creds(log),
        log.public_base_url.as_deref(),
        log.frontend_url.as_deref(),
    ) else {
        return unconfigured();
    };
    let redirect_uri = callback_redirect_uri(public_base);
    let state_param = oauth::sign_state(secret.expose_secret().as_bytes(), &login_state_message());
    // The primary login flow requests NO scopes: an unscoped user-to-server token
    // resolves `/user`, which is all identity establishment needs.
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

/// `GET /api/v1/auth/github/callback` — GitHub's OAuth return.
///
/// Verifies the signed `state` + its freshness, exchanges the `code` for a token set
/// (server-side), and 302-redirects to the frontend with the tokens in the URL
/// fragment. Failures render a browser-friendly HTML page (never a token in a body).
#[utoipa::path(
    get,
    path = "/auth/github/callback",
    tag = "auth",
    operation_id = "github_login_callback",
    params(LoginCallbackQuery),
    responses(
        (status = 302, description = "Redirect to the frontend with the token in the URL fragment"),
        (status = 400, description = "Missing/tampered/expired OAuth state or code (HTML)"),
        (status = 401, description = "GitHub rejected the sign-in (HTML)"),
        (status = 503, description = "Frontend login is not configured (HTML)"),
    )
)]
async fn github_login_callback(
    State(state): State<AppState>,
    extensions: Extensions,
    Query(query): Query<LoginCallbackQuery>,
) -> Response {
    // One record per outcome, written by the inner function as it decides. The
    // `code`, the `state`, the exchanged tokens, and GitHub's own error slug are
    // never arguments — the closed result is what makes the flow queryable.
    let (response, result) = login_callback(&state, &extensions, query).await;
    record_safe(&extensions, &SafeGithubLoginCallback::new(result));
    response
}

/// The callback's real body, returning its outcome alongside the response so the
/// single recording site above cannot be missed on a new early return.
async fn login_callback(
    state: &AppState,
    extensions: &Extensions,
    query: LoginCallbackQuery,
) -> (Response, OauthResult) {
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(public_base), Some(frontend)) = (
        oauth_creds(log),
        log.public_base_url.as_deref(),
        log.frontend_url.as_deref(),
    ) else {
        return (
            html_error(StatusCode::SERVICE_UNAVAILABLE, "Login is not configured."),
            OauthResult::UpstreamError,
        );
    };

    // The user denied consent on GitHub — bounce back to the frontend with a flag.
    if query.error.is_some() {
        return (
            redirect_302(&format!(
                "{}#gh_error=access_denied",
                frontend.trim_end_matches('#')
            )),
            OauthResult::Denied,
        );
    }
    // GitHub's POST-INSTALL redirect: "Request user authorization during
    // installation" routes App installs to this callback with
    // `code`+`installation_id`+`setup_action` but NO `state`. That is not a
    // login — bounce to the dashboard, where the fresh installation is already
    // visible. The unused one-time code is deliberately dropped: with no state
    // there is no CSRF binding under which it would be safe to exchange.
    if let Some(target) = post_install_redirect(
        query.state.as_deref(),
        query.setup_action.as_deref(),
        query.installation_id.as_deref(),
        frontend,
    ) {
        // A stateless install bounce: no OAuth round-trip happened at all, which
        // is `invalid` login material rather than a denial or an upstream fault.
        return (redirect_302(&target), OauthResult::Invalid);
    }

    let (Some(code), Some(state_param)) = (
        query.code.filter(|c| !c.is_empty()),
        query.state.filter(|s| !s.is_empty()),
    ) else {
        return (
            html_error(StatusCode::BAD_REQUEST, "Missing OAuth code or state."),
            OauthResult::Invalid,
        );
    };
    // Verify the signed state (CSRF/tamper) and its freshness (replay bound).
    let Some(message) = oauth::verify_state(secret.expose_secret().as_bytes(), &state_param) else {
        return (
            html_error(StatusCode::BAD_REQUEST, "Invalid or tampered OAuth state."),
            OauthResult::Invalid,
        );
    };
    if !state_is_fresh(&message) {
        return (
            html_error(
                StatusCode::BAD_REQUEST,
                "This sign-in link has expired. Please try again.",
            ),
            OauthResult::Invalid,
        );
    }

    let redirect_uri = callback_redirect_uri(public_base);
    let tokens = match oauth::exchange_code_tokens(
        http_client(),
        &log.oauth_base_url,
        client_id,
        secret,
        &code,
        &redirect_uri,
    )
    .await
    {
        Ok(tokens) => tokens,
        Err(_) => {
            return (
                html_error(
                    StatusCode::UNAUTHORIZED,
                    "GitHub rejected the sign-in. Please try again.",
                ),
                OauthResult::UpstreamError,
            )
        }
    };
    // An exchanged token is not yet an identity. Resolve `GET /user` BEFORE the
    // sign-in is treated as successful, so the terminal record is attributed to a
    // verified numeric id rather than to an invented actor. A `/user` failure is a
    // stable authentication failure: fail closed instead of handing the SPA a
    // session whose owner we could not name.
    if resolve_oauth_identity(state, &tokens.access_token, extensions)
        .await
        .is_err()
    {
        return (
            html_error(
                StatusCode::UNAUTHORIZED,
                "Could not verify your GitHub identity.",
            ),
            OauthResult::UpstreamError,
        );
    }
    // Hand the token set to the SPA in the fragment (never a query string / log).
    (
        redirect_302(&frontend_success_url(frontend, &tokens)),
        OauthResult::Success,
    )
}

/// `POST /api/v1/auth/github/refresh` — redeem a refresh token for a fresh token set.
///
/// Keeps the user signed in past the 8h access-token lifetime without a re-login. The
/// refresh token is the credential (no Bearer needed); an expired/used one → 401 (the
/// SPA must re-login). GitHub rotates the refresh token, so the SPA must persist the
/// NEW one from the response.
#[utoipa::path(
    post,
    path = "/auth/github/refresh",
    tag = "auth",
    operation_id = "github_refresh_token",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "A fresh access token (+ rotated refresh token)", body = TokenResponse),
        (status = 400, description = "Empty refresh token", body = ErrorEnvelope),
        (status = 401, description = "The refresh token was rejected; re-login required", body = ErrorEnvelope),
        (status = 503, description = "Frontend login is not configured", body = ErrorEnvelope),
    )
)]
async fn github_refresh_token(
    State(state): State<AppState>,
    extensions: Extensions,
    AuditedJson(req): AuditedJson<RefreshRequest>,
) -> Response {
    let (response, result) = refresh_token(&state, &extensions, req).await;
    record_safe(&extensions, &SafeGithubRefreshToken::new(result));
    response
}

/// The refresh body, returning its outcome alongside the response. The refresh
/// token IS the credential, so nothing about the body is ever an argument.
async fn refresh_token(
    state: &AppState,
    extensions: &Extensions,
    req: RefreshRequest,
) -> (Response, OauthResult) {
    let log = &state.config.log;
    let Some((client_id, secret)) = oauth_creds(log) else {
        return (unconfigured(), OauthResult::UpstreamError);
    };
    let refresh = req.refresh_token.trim();
    if refresh.is_empty() {
        return (
            AppError::Validation("refresh_token must not be empty".to_string()).into_response(),
            OauthResult::Invalid,
        );
    }
    match oauth::refresh_tokens(
        http_client(),
        &log.oauth_base_url,
        client_id,
        secret,
        refresh,
    )
    .await
    {
        Ok(tokens) => {
            // Same rule as the callback: a refreshed session is attributed only
            // after `GET /user` names its owner.
            match resolve_oauth_identity(state, &tokens.access_token, extensions).await {
                Ok(_) => (
                    Json(token_response(&tokens)).into_response(),
                    OauthResult::Success,
                ),
                Err(err) => (err.into_response(), OauthResult::UpstreamError),
            }
        }
        Err(err) => (err.into_response(), OauthResult::UpstreamError),
    }
}

/// Resolve the verified identity behind a freshly exchanged/refreshed OAuth token
/// and publish it as this request's audit actor.
///
/// Shared with the broader-visibility connect flow. The token is used for the one
/// `GET /user` call and never stored, logged, or placed in the extensions — the
/// recorded identity is the numeric id plus a login snapshot and nothing else.
pub(super) async fn resolve_oauth_identity(
    state: &AppState,
    token: &SecretString,
    extensions: &Extensions,
) -> Result<GithubUser, AppError> {
    let user = identity::resolve(&state.config.github_api_base_url, token.expose_secret()).await?;
    crate::audit::identity::record_identity(
        extensions,
        AuditIdentity::github_oauth(user.id, user.login.clone()),
    );
    tracing::info!(user_id = user.id, "oauth identity verified after exchange");
    Ok(user)
}

// ---- Helpers ----------------------------------------------------------------

/// The `(client_id, client_secret)` pair, present only when BOTH are configured.
fn oauth_creds(log: &LogConfig) -> Option<(&str, &SecretString)> {
    match (
        log.oauth_client_id.as_deref(),
        log.oauth_client_secret.as_ref(),
    ) {
        (Some(id), Some(secret)) => Some((id, secret)),
        _ => None,
    }
}

/// The 503 rendered when frontend login is not configured.
fn unconfigured() -> Response {
    AppError::Unavailable(
        "frontend login is not configured (set FKST_GITHUB_OAUTH_CLIENT_ID/SECRET, \
         FKST_PUBLIC_BASE_URL, and FKST_FRONTEND_URL)"
            .to_string(),
    )
    .into_response()
}

/// A 302 redirect to `location`. An un-encodable location renders a 500 rather than
/// panicking (never happens for our own URLs).
pub(super) fn redirect_302(location: &str) -> Response {
    match HeaderValue::from_str(location) {
        Ok(value) => (StatusCode::FOUND, [(header::LOCATION, value)]).into_response(),
        Err(_) => AppError::Internal(anyhow::anyhow!("invalid redirect location")).into_response(),
    }
}

/// A browser-friendly HTML error page (fixed, escaping-free message text).
pub(super) fn html_error(status: StatusCode, message: &str) -> Response {
    let body = format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><title>{code}</title></head>\
         <body><h1>{code}</h1><p>{message}</p></body></html>",
        code = status.as_u16()
    );
    // The browser paths render HTML rather than the JSON envelope, so they carry
    // no `error` field — the stable audit code is attached as a typed extension
    // instead, keeping OAuth failures correlatable without the message (which may
    // describe caller-supplied state) ever reaching a record. A 401/403 page is
    // additionally marked a policy rejection, so a sign-in denial classifies the
    // same here as it does on the JSON surface.
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

/// A pooled HTTP client for the OAuth token exchange/refresh (bounded timeout + UA).
pub(super) fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("fkst-hosted")
            .build()
            .expect("build auth-oauth http client")
    })
}

/// The frontend-login router (nested under `/api/v1`). Open at the app layer: the
/// login/callback establish identity and the refresh is guarded by the refresh token
/// itself, so there is no documented security scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(github_login))
        .routes(routes!(github_login_callback))
        .routes(routes!(github_refresh_token))
        // The optional broader-visibility classic-OAuth connect flow (issue #572);
        // its routes are inert (return 503) unless the broader pair is configured.
        .merge(crate::routes::auth_broader::router())
}

#[cfg(test)]
#[path = "auth_handler_tests.rs"]
mod handler_tests;
