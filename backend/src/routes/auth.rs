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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use axum::extract::{Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use secrecy::{ExposeSecret, SecretString};
use serde::{Deserialize, Serialize};
use utoipa::{IntoParams, ToSchema};
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::log_config::LogConfig;
use crate::routes::logs::oauth;
use crate::state::AppState;

/// Freshness window for a login `state`: a callback presenting a signed state older
/// than this (or from the future beyond a small skew) is rejected. Bounds replay
/// without a server-side session store.
const STATE_MAX_AGE_SECS: i64 = 600;

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
async fn github_login(State(state): State<AppState>) -> Response {
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
    match oauth::authorize_url(&log.oauth_base_url, client_id, &redirect_uri, &state_param) {
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
    Query(query): Query<LoginCallbackQuery>,
) -> Response {
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(public_base), Some(frontend)) = (
        oauth_creds(log),
        log.public_base_url.as_deref(),
        log.frontend_url.as_deref(),
    ) else {
        return html_error(StatusCode::SERVICE_UNAVAILABLE, "Login is not configured.");
    };

    // The user denied consent on GitHub — bounce back to the frontend with a flag.
    if query.error.is_some() {
        return redirect_302(&format!(
            "{}#gh_error=access_denied",
            frontend.trim_end_matches('#')
        ));
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
        return redirect_302(&target);
    }

    let (Some(code), Some(state_param)) = (
        query.code.filter(|c| !c.is_empty()),
        query.state.filter(|s| !s.is_empty()),
    ) else {
        return html_error(StatusCode::BAD_REQUEST, "Missing OAuth code or state.");
    };
    // Verify the signed state (CSRF/tamper) and its freshness (replay bound).
    let Some(message) = oauth::verify_state(secret.expose_secret().as_bytes(), &state_param) else {
        return html_error(StatusCode::BAD_REQUEST, "Invalid or tampered OAuth state.");
    };
    if !state_is_fresh(&message) {
        return html_error(
            StatusCode::BAD_REQUEST,
            "This sign-in link has expired. Please try again.",
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
            return html_error(
                StatusCode::UNAUTHORIZED,
                "GitHub rejected the sign-in. Please try again.",
            )
        }
    };
    // Hand the token set to the SPA in the fragment (never a query string / log).
    redirect_302(&frontend_success_url(frontend, &tokens))
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
    Json(req): Json<RefreshRequest>,
) -> Response {
    let log = &state.config.log;
    let Some((client_id, secret)) = oauth_creds(log) else {
        return unconfigured();
    };
    let refresh = req.refresh_token.trim();
    if refresh.is_empty() {
        return AppError::Validation("refresh_token must not be empty".to_string()).into_response();
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
        Ok(tokens) => Json(token_response(&tokens)).into_response(),
        Err(err) => err.into_response(),
    }
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

/// The OAuth `redirect_uri`: `<public_base>/api/v1/auth/github/callback`.
fn callback_redirect_uri(public_base: &str) -> String {
    format!(
        "{}/api/v1/auth/github/callback",
        public_base.trim_end_matches('/')
    )
}

/// Current Unix time in whole seconds (monotonic-agnostic; only used for the
/// login-state freshness window, so a small clock wobble is harmless).
fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// The signed-state payload: `login:<unix-seconds>` (freshness-checked on return).
fn login_state_message() -> String {
    format!("login:{}", now_unix())
}

/// Whether a recovered `login:<ts>` state is within the freshness window (allowing a
/// small backward clock skew).
fn state_is_fresh(message: &str) -> bool {
    let Some(ts_str) = message.strip_prefix("login:") else {
        return false;
    };
    let Ok(ts) = ts_str.parse::<i64>() else {
        return false;
    };
    let age = now_unix() - ts;
    (-30..=STATE_MAX_AGE_SECS).contains(&age)
}

/// Build the frontend redirect URL carrying the token set in the fragment. GitHub
/// tokens are URL-safe (`[A-Za-z0-9_]`), so no percent-encoding is needed.
fn frontend_success_url(frontend: &str, tokens: &oauth::TokenSet) -> String {
    let mut fragment = format!("gh_token={}", tokens.access_token.expose_secret());
    if let Some(refresh) = &tokens.refresh_token {
        fragment.push_str(&format!("&gh_refresh={}", refresh.expose_secret()));
    }
    if let Some(expires_in) = tokens.expires_in {
        fragment.push_str(&format!("&gh_expires_in={expires_in}"));
    }
    format!("{}#{fragment}", frontend.trim_end_matches('#'))
}

/// Convert a [`oauth::TokenSet`] into the JSON [`TokenResponse`] for the SPA.
fn token_response(tokens: &oauth::TokenSet) -> TokenResponse {
    TokenResponse {
        access_token: tokens.access_token.expose_secret().to_string(),
        refresh_token: tokens
            .refresh_token
            .as_ref()
            .map(|t| t.expose_secret().to_string()),
        expires_in: tokens.expires_in,
        refresh_token_expires_in: tokens.refresh_token_expires_in,
        token_type: "bearer",
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

/// A pooled HTTP client for the OAuth token exchange/refresh (bounded timeout + UA).
fn http_client() -> &'static reqwest::Client {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        reqwest::Client::builder()
            .timeout(Duration::from_secs(20))
            .user_agent("fkst-hosted")
            .build()
            .expect("build auth-oauth http client")
    })
}

/// The dashboard URL a STATELESS GitHub post-install redirect bounces to, or
/// `None` for a normal login callback (a `state` is present, or no install
/// markers are). Stateless + `setup_action`/`installation_id` = GitHub sent the
/// browser here after an App install, not after our login redirect.
fn post_install_redirect(
    state: Option<&str>,
    setup_action: Option<&str>,
    installation_id: Option<&str>,
    frontend: &str,
) -> Option<String> {
    let stateless = state.map(str::is_empty).unwrap_or(true);
    if stateless && (setup_action.is_some() || installation_id.is_some()) {
        Some(format!("{}/dashboard", frontend.trim_end_matches('/')))
    } else {
        None
    }
}

/// The frontend-login router (nested under `/api/v1`). Open at the app layer: the
/// login/callback establish identity and the refresh is guarded by the refresh token
/// itself, so there is no documented security scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(github_login))
        .routes(routes!(github_login_callback))
        .routes(routes!(github_refresh_token))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn callback_redirect_uri_appends_the_path_and_trims_slash() {
        assert_eq!(
            callback_redirect_uri("https://fkst.example/"),
            "https://fkst.example/api/v1/auth/github/callback"
        );
        assert_eq!(
            callback_redirect_uri("https://fkst.example"),
            "https://fkst.example/api/v1/auth/github/callback"
        );
    }

    #[test]
    fn a_freshly_signed_state_round_trips_and_is_fresh() {
        let secret = b"client-secret";
        let state = oauth::sign_state(secret, &login_state_message());
        let message = oauth::verify_state(secret, &state).expect("verifies");
        assert!(state_is_fresh(&message), "just-issued state must be fresh");
    }

    #[test]
    fn an_old_state_is_rejected_as_stale() {
        // A timestamp well outside the window.
        let old = now_unix() - (STATE_MAX_AGE_SECS + 60);
        assert!(!state_is_fresh(&format!("login:{old}")));
    }

    #[test]
    fn malformed_state_messages_are_not_fresh() {
        assert!(!state_is_fresh("not-a-login-message"));
        assert!(!state_is_fresh("login:not-a-number"));
        assert!(!state_is_fresh("login:"));
    }

    #[test]
    fn success_url_carries_all_tokens_in_the_fragment() {
        let tokens = oauth::TokenSet {
            access_token: SecretString::from("ghu_access".to_string()),
            refresh_token: Some(SecretString::from("ghr_refresh".to_string())),
            expires_in: Some(28800),
            refresh_token_expires_in: Some(15811200),
        };
        let url = frontend_success_url("https://app.example/fkst/", &tokens);
        assert_eq!(
            url,
            "https://app.example/fkst/#gh_token=ghu_access&gh_refresh=ghr_refresh&gh_expires_in=28800"
        );
    }

    #[test]
    fn success_url_omits_refresh_when_absent() {
        let tokens = oauth::TokenSet {
            access_token: SecretString::from("ghu_access".to_string()),
            refresh_token: None,
            expires_in: None,
            refresh_token_expires_in: None,
        };
        let url = frontend_success_url("https://app.example/", &tokens);
        assert_eq!(url, "https://app.example/#gh_token=ghu_access");
    }

    #[test]
    fn token_response_exposes_the_tokens_and_bearer_type() {
        let tokens = oauth::TokenSet {
            access_token: SecretString::from("ghu_x".to_string()),
            refresh_token: Some(SecretString::from("ghr_y".to_string())),
            expires_in: Some(28800),
            refresh_token_expires_in: None,
        };
        let resp = token_response(&tokens);
        assert_eq!(resp.access_token, "ghu_x");
        assert_eq!(resp.refresh_token.as_deref(), Some("ghr_y"));
        assert_eq!(resp.expires_in, Some(28800));
        assert_eq!(resp.token_type, "bearer");
    }
}

#[cfg(test)]
mod post_install_tests {
    use super::post_install_redirect;

    #[test]
    fn stateless_install_redirects_to_the_dashboard() {
        assert_eq!(
            post_install_redirect(
                None,
                Some("install"),
                Some("146704012"),
                "https://fkst.example/"
            ),
            Some("https://fkst.example/dashboard".to_string())
        );
        // installation_id alone is enough (setup_action can be absent).
        assert_eq!(
            post_install_redirect(Some(""), None, Some("1"), "https://fkst.example"),
            Some("https://fkst.example/dashboard".to_string())
        );
    }

    #[test]
    fn a_real_login_callback_is_untouched() {
        // State present -> normal login path even if GitHub echoes extras.
        assert_eq!(
            post_install_redirect(
                Some("signed-state"),
                Some("install"),
                Some("1"),
                "https://f"
            ),
            None
        );
        // No install markers + no state -> not an install; the 400 path owns it.
        assert_eq!(post_install_redirect(None, None, None, "https://f"), None);
    }
}
