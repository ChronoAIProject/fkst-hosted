//! The pure, I/O-free pieces of the frontend GitHub-OAuth flows: signed-state
//! payloads and their freshness window, the callback `redirect_uri`, the
//! post-install bounce, and the two token-shaping projections.
//!
//! Split out of [`crate::routes::auth`] so that module stays within the source
//! line budget while both it and [`crate::routes::auth_broader`] share one
//! definition of the state grammar. Nothing here performs I/O or touches the
//! request, which is also why it is the part that is exhaustively unit-testable.
//!
//! The `kind` prefix on a signed state (`login:<ts>`, `broader:<ts>`) is the
//! reason these live together: it namespaces the flows so a state minted for one
//! can never be replayed into the other, and that guarantee only holds while
//! both flows read it from the same place.

use std::time::{SystemTime, UNIX_EPOCH};

use secrecy::ExposeSecret;

use crate::routes::auth::TokenResponse;
use crate::routes::logs::oauth;

/// Freshness window for a signed `state`: a callback presenting a state older
/// than this (or from the future beyond a small skew) is rejected. Bounds replay
/// without a server-side session store.
pub(crate) const STATE_MAX_AGE_SECS: i64 = 600;

/// The OAuth `redirect_uri`: `<public_base>/api/v1/auth/github/callback`.
pub(crate) fn callback_redirect_uri(public_base: &str) -> String {
    format!(
        "{}/api/v1/auth/github/callback",
        public_base.trim_end_matches('/')
    )
}

/// Current Unix time in whole seconds (monotonic-agnostic; only used for the
/// state freshness window, so a small clock wobble is harmless).
pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A signed-state payload `"<kind>:<unix-seconds>"` (freshness-checked on return).
/// `kind` namespaces the flow (`login`, `broader`) so a state minted for one flow can
/// never be replayed into another — defense in depth on top of the HMAC signature.
pub(crate) fn signed_state_message(kind: &str) -> String {
    format!("{kind}:{}", now_unix())
}

/// Whether a recovered `"<kind>:<ts>"` state is within the freshness window (allowing
/// a small backward clock skew). A message whose prefix is not exactly `"<kind>:"` is
/// not fresh, so a `login` state does not satisfy a `broader` check and vice versa.
pub(crate) fn state_is_fresh_for(kind: &str, message: &str) -> bool {
    let Some(ts_str) = message.strip_prefix(&format!("{kind}:")) else {
        return false;
    };
    let Ok(ts) = ts_str.parse::<i64>() else {
        return false;
    };
    let age = now_unix() - ts;
    (-30..=STATE_MAX_AGE_SECS).contains(&age)
}

/// The login flow's signed-state payload: `login:<unix-seconds>`.
pub(crate) fn login_state_message() -> String {
    signed_state_message("login")
}

/// Whether a recovered `login:<ts>` state is within the freshness window.
pub(crate) fn state_is_fresh(message: &str) -> bool {
    state_is_fresh_for("login", message)
}

/// Build the frontend redirect URL carrying the token set in the fragment. GitHub
/// tokens are URL-safe (`[A-Za-z0-9_]`), so no percent-encoding is needed.
pub(crate) fn frontend_success_url(frontend: &str, tokens: &oauth::TokenSet) -> String {
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
pub(crate) fn token_response(tokens: &oauth::TokenSet) -> TokenResponse {
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

/// The dashboard URL a STATELESS GitHub post-install redirect bounces to, or
/// `None` for a normal login callback (a `state` is present, or no install
/// markers are). Stateless + `setup_action`/`installation_id` = GitHub sent the
/// browser here after an App install, not after our login redirect.
pub(crate) fn post_install_redirect(
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

#[cfg(test)]
#[path = "auth_oauth_state_tests.rs"]
mod tests;
