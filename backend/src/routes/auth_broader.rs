//! Broader-visibility GitHub OAuth connect flow (issue #572, epic #572 · R1b).
//!
//! The dashboard's primary login token is a GitHub **App** user-to-server token,
//! which GitHub scopes to the repos/orgs where the App is installed. To let the
//! canvas surface repos/orgs where the App is NOT installed, this OPTIONAL, ADDITIVE
//! flow authorizes a SECOND credential — a **classic** OAuth App authorization
//! carrying `repo` + `read:org` — used ONLY to enumerate the caller's repos/orgs
//! (`crate::routes::canvas::overview`). The App token still drives
//! `/user/installations` (the installed flags) and the reconciler.
//!
//! - `GET /api/v1/auth/github/broader` → 302 into GitHub's classic-OAuth authorize
//!   page (broader client id, `scope=repo read:org`, a signed + time-bounded
//!   `state`), or 503 when the broader pair is not configured.
//! - `GET /api/v1/auth/github/broader/callback` → verify `state`, exchange the `code`
//!   for a **classic access token** (no refresh/expiry), then 302 to the frontend
//!   with the token in the URL **fragment** (`#broader_token=<token>`) — the same
//!   fragment-only, never-logged discipline as the primary login. Every failure
//!   renders a browser-friendly HTML page; a token never lands in a body/log/query.
//!
//! The broader token is NEVER logged and NEVER placed in a query string or a response
//! body except the SPA-bound fragment. All shared plumbing (state freshness, the HTML
//! error page, the 302 helper, the pooled HTTP client) is reused from
//! [`crate::routes::auth`]; the signed-state + authorize-URL + token-exchange
//! primitives come from [`crate::routes::logs::oauth`].

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use secrecy::ExposeSecret;
use serde::Deserialize;
use utoipa::IntoParams;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::error::{AppError, ErrorEnvelope};
use crate::routes::auth::{
    html_error, http_client, redirect_302, signed_state_message, state_is_fresh_for,
};
use crate::routes::logs::oauth;
use crate::state::AppState;

/// The classic-OAuth scopes the broader-visibility token requests: `repo` (private
/// repo enumeration) + `read:org` (org membership). Space-separated per the OAuth
/// spec; the authorize-URL builder percent-encodes it.
const BROADER_SCOPE: &str = "repo read:org";

/// The `state` namespace for the broader flow (distinct from the login flow's
/// `login`), so a state minted for one flow can never be replayed into the other.
const BROADER_STATE_KIND: &str = "broader";

/// The broader-OAuth callback query (`?code=&state=` on success, `?error=` on denial).
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub struct BroaderCallbackQuery {
    /// The one-time OAuth code GitHub returns (absent on an error redirect).
    #[serde(default)]
    code: Option<String>,
    /// The signed `state` value the connect endpoint issued.
    #[serde(default)]
    state: Option<String>,
    /// GitHub's error slug when the user denied authorization (e.g. `access_denied`).
    #[serde(default)]
    error: Option<String>,
}

/// `GET /api/v1/auth/github/broader` — begin the broader-visibility OAuth connect.
///
/// 302-redirects the browser to GitHub's classic-OAuth authorize page (broader client
/// id, `scope=repo read:org`, a signed + time-bounded `state`). When the broader pair
/// (or `FKST_PUBLIC_BASE_URL` / `FKST_FRONTEND_URL`) is not configured, returns 503 —
/// leaving the feature entirely inert.
#[utoipa::path(
    get,
    path = "/auth/github/broader",
    tag = "auth",
    operation_id = "github_broader_connect",
    responses(
        (status = 302, description = "Redirect to GitHub's classic-OAuth authorize page (repo + read:org)"),
        (status = 503, description = "The broader-visibility OAuth flow is not configured", body = ErrorEnvelope),
    )
)]
async fn github_broader(State(state): State<AppState>) -> Response {
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(public_base), Some(_frontend)) = (
        log.broader_oauth(),
        log.public_base_url.as_deref(),
        log.frontend_url.as_deref(),
    ) else {
        return unconfigured();
    };
    let redirect_uri = broader_callback_redirect_uri(public_base);
    let state_param = oauth::sign_state(
        secret.expose_secret().as_bytes(),
        &signed_state_message(BROADER_STATE_KIND),
    );
    match oauth::authorize_url(
        &log.oauth_base_url,
        client_id,
        &redirect_uri,
        &state_param,
        Some(BROADER_SCOPE),
    ) {
        Ok(url) => redirect_302(&url),
        Err(err) => err.into_response(),
    }
}

/// `GET /api/v1/auth/github/broader/callback` — GitHub's classic-OAuth return.
///
/// Verifies the signed `state` + its freshness, exchanges the `code` for a CLASSIC
/// access token (no refresh/expiry), and 302-redirects to the frontend with the token
/// in the URL fragment (`#broader_token=<token>`). Failures render a browser-friendly
/// HTML page — the token never appears in a body, log, or query string.
#[utoipa::path(
    get,
    path = "/auth/github/broader/callback",
    tag = "auth",
    operation_id = "github_broader_callback",
    params(BroaderCallbackQuery),
    responses(
        (status = 302, description = "Redirect to the frontend with the broader token in the URL fragment"),
        (status = 400, description = "Missing/tampered/expired OAuth state or code (HTML)"),
        (status = 401, description = "GitHub rejected the authorization (HTML)"),
        (status = 503, description = "The broader-visibility OAuth flow is not configured (HTML)"),
    )
)]
async fn github_broader_callback(
    State(state): State<AppState>,
    Query(query): Query<BroaderCallbackQuery>,
) -> Response {
    let log = &state.config.log;
    let (Some((client_id, secret)), Some(public_base), Some(frontend)) = (
        log.broader_oauth(),
        log.public_base_url.as_deref(),
        log.frontend_url.as_deref(),
    ) else {
        return html_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Broader GitHub access is not configured.",
        );
    };

    // The user declined the authorization on GitHub — bounce back to the frontend
    // with a flag (never a token).
    if query.error.is_some() {
        return redirect_302(&format!(
            "{}#broader_error=access_denied",
            frontend.trim_end_matches('#')
        ));
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
    if !state_is_fresh_for(BROADER_STATE_KIND, &message) {
        return html_error(
            StatusCode::BAD_REQUEST,
            "This link has expired. Please try again.",
        );
    }

    let redirect_uri = broader_callback_redirect_uri(public_base);
    // A classic OAuth token has no refresh/expiry — we consume only the access token
    // from the exchanged set and hand it to the SPA.
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
                "GitHub rejected the authorization. Please try again.",
            )
        }
    };
    // Hand the broader token to the SPA in the fragment (never a query string / log).
    redirect_302(&broader_success_url(frontend, &tokens.access_token))
}

// ---- Helpers ----------------------------------------------------------------

/// The broader callback `redirect_uri`: `<public_base>/api/v1/auth/github/broader/callback`.
fn broader_callback_redirect_uri(public_base: &str) -> String {
    format!(
        "{}/api/v1/auth/github/broader/callback",
        public_base.trim_end_matches('/')
    )
}

/// Build the frontend redirect URL carrying the broader token in the fragment. GitHub
/// tokens are URL-safe (`[A-Za-z0-9_]`), so no percent-encoding is needed.
fn broader_success_url(frontend: &str, token: &secrecy::SecretString) -> String {
    format!(
        "{}#broader_token={}",
        frontend.trim_end_matches('#'),
        token.expose_secret()
    )
}

/// The 503 rendered when the broader-visibility OAuth flow is not configured.
fn unconfigured() -> Response {
    AppError::Unavailable(
        "the broader-visibility GitHub OAuth flow is not configured (set \
         FKST_GITHUB_BROADER_OAUTH_CLIENT_ID/SECRET, FKST_PUBLIC_BASE_URL, and \
         FKST_FRONTEND_URL)"
            .to_string(),
    )
    .into_response()
}

/// The broader-visibility connect router (nested under `/api/v1`, merged into the auth
/// router). Open at the app layer: the connect/callback establish a second credential
/// and are guarded by the signed `state`, so there is no documented security scheme.
pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new()
        .routes(routes!(github_broader))
        .routes(routes!(github_broader_callback))
}

#[cfg(test)]
#[path = "auth_broader_tests.rs"]
mod tests;
