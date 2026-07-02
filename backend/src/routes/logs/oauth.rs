//! Browser-mode GitHub user-OAuth helpers for the log-download endpoint.
//!
//! When the log-download endpoint is hit WITHOUT an `Authorization` header it must
//! establish the caller's identity interactively: it redirects the browser to
//! GitHub's user-OAuth `authorize` page and, on return, exchanges the `code` for a
//! short-lived user token it trades for `{login, id}` (exactly like the API path).
//!
//! The `state` parameter is the CSRF/tamper guard: it carries the `session_id` the
//! whole flow is for, HMAC-signed with the App's OAuth client secret so a returning
//! callback cannot be pointed at a DIFFERENT session by editing the query. The
//! signature is verified in constant time; a bad/absent signature aborts the flow.
//!
//! Secret hygiene: the client secret rides only the token-exchange request body (and
//! the HMAC key), never a URL or a log; the exchanged user token is returned for the
//! caller to resolve `/user` and is never logged or stored.

use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use sha2::Sha256;

use crate::error::AppError;

type HmacSha256 = Hmac<Sha256>;

/// The OAuth token-exchange success body; only `access_token` is consumed.
#[derive(Deserialize)]
struct AccessTokenResponse {
    #[serde(default)]
    access_token: Option<String>,
}

/// Sign `session_id` into an opaque `state` value: `"<session_id>.<hmac_hex>"`, where
/// the MAC is `HMAC-SHA256(secret, session_id)`. The session id is round-tripped in
/// the clear (it is not secret) but cannot be TAMPERED with — a changed id fails the
/// signature check on return.
pub(super) fn sign_state(secret: &[u8], session_id: &str) -> String {
    format!("{session_id}.{}", mac_hex(secret, session_id))
}

/// Verify a `state` produced by [`sign_state`] and recover the `session_id`.
///
/// Returns `Some(session_id)` only when the value is `"<session_id>.<hex>"` and the
/// recomputed MAC over `session_id` matches `<hex>` in CONSTANT time; `None` for a
/// malformed or tampered value (so the caller aborts with a 400).
pub(super) fn verify_state(secret: &[u8], state: &str) -> Option<String> {
    // Split on the LAST '.' so a session id that itself contained '.' would still
    // round-trip (a UUIDv5 session id never does, but be robust).
    let (session_id, hex_sig) = state.rsplit_once('.')?;
    let expected = decode_hex(hex_sig)?;
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(session_id.as_bytes());
    mac.verify_slice(&expected)
        .ok()
        .map(|()| session_id.to_string())
}

/// Build the GitHub user-OAuth `authorize` URL to redirect the browser to. No scopes
/// are requested — `/user` returns the caller's `{login, id}` with an unscoped token,
/// which is all identity resolution needs.
pub(super) fn authorize_url(
    oauth_base: &str,
    client_id: &str,
    redirect_uri: &str,
    state: &str,
) -> Result<String, AppError> {
    let base = format!("{}/login/oauth/authorize", oauth_base.trim_end_matches('/'));
    let url = reqwest::Url::parse_with_params(
        &base,
        &[
            ("client_id", client_id),
            ("redirect_uri", redirect_uri),
            ("state", state),
        ],
    )
    .map_err(|e| {
        // The error carries only the (non-secret) base URL, never the token/secret.
        AppError::Internal(anyhow::anyhow!("build oauth authorize url: {e}"))
    })?;
    Ok(url.to_string())
}

/// Exchange an OAuth `code` for a user access token via
/// `POST {oauth_base}/login/oauth/access_token`. The client secret rides the request
/// body only. A missing `access_token` (GitHub rejected the code) → `Unauthorized`;
/// a transport/5xx failure → `Unavailable`. The returned token is never logged.
pub(super) async fn exchange_code(
    http: &reqwest::Client,
    oauth_base: &str,
    client_id: &str,
    client_secret: &SecretString,
    code: &str,
    redirect_uri: &str,
) -> Result<SecretString, AppError> {
    let url = format!(
        "{}/login/oauth/access_token",
        oauth_base.trim_end_matches('/')
    );
    let response = http
        .post(&url)
        .header(reqwest::header::ACCEPT, "application/json")
        .form(&[
            ("client_id", client_id),
            ("client_secret", client_secret.expose_secret()),
            ("code", code),
            ("redirect_uri", redirect_uri),
        ])
        .send()
        .await
        .map_err(|e| {
            // Never log the code/secret; only that the exchange transport failed.
            tracing::warn!(error = %e, "oauth code exchange transport error");
            AppError::Unavailable(
                "github oauth code exchange failed (upstream unreachable)".to_string(),
            )
        })?;

    if !response.status().is_success() {
        return Err(AppError::Unauthorized(
            "github oauth code exchange rejected".to_string(),
        ));
    }
    let body: AccessTokenResponse = response.json().await.map_err(|e| {
        tracing::warn!(error = %e, "oauth code exchange response did not parse");
        AppError::Unavailable(
            "github oauth code exchange failed (bad upstream response)".to_string(),
        )
    })?;
    match body.access_token.filter(|t| !t.is_empty()) {
        Some(token) => Ok(SecretString::from(token)),
        None => Err(AppError::Unauthorized(
            "github oauth code exchange returned no access token".to_string(),
        )),
    }
}

/// Hex `HMAC-SHA256(secret, message)`.
fn mac_hex(secret: &[u8], message: &str) -> String {
    let mut mac = HmacSha256::new_from_slice(secret).expect("HMAC accepts any key length");
    mac.update(message.as_bytes());
    mac.finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Decode a hex string into bytes, or `None` on an odd length / non-hex digit.
fn decode_hex(s: &str) -> Option<Vec<u8>> {
    if s.is_empty() || !s.len().is_multiple_of(2) {
        return None;
    }
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(s.len() / 2);
    let mut i = 0;
    while i < bytes.len() {
        let hi = (bytes[i] as char).to_digit(16)?;
        let lo = (bytes[i + 1] as char).to_digit(16)?;
        out.push((hi * 16 + lo) as u8);
        i += 2;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SECRET: &[u8] = b"oauth-client-secret";

    #[test]
    fn sign_then_verify_round_trips_the_session_id() {
        let state = sign_state(SECRET, "sess-abc-123");
        assert_eq!(
            verify_state(SECRET, &state).as_deref(),
            Some("sess-abc-123")
        );
    }

    #[test]
    fn a_tampered_session_id_fails_verification() {
        let state = sign_state(SECRET, "sess-abc-123");
        // Swap the session id but keep the original signature: must not verify.
        let (_, sig) = state.rsplit_once('.').unwrap();
        let forged = format!("sess-EVIL.{sig}");
        assert!(verify_state(SECRET, &forged).is_none());
    }

    #[test]
    fn a_wrong_secret_fails_verification() {
        let state = sign_state(SECRET, "sess-abc-123");
        assert!(verify_state(b"different-secret", &state).is_none());
    }

    #[test]
    fn malformed_state_values_are_rejected() {
        assert!(verify_state(SECRET, "no-dot-here").is_none());
        assert!(verify_state(SECRET, "sess.").is_none());
        assert!(verify_state(SECRET, "sess.zz").is_none());
    }

    #[test]
    fn authorize_url_encodes_the_params() {
        let url = authorize_url(
            "https://github.com",
            "Iv1.abc",
            "https://fkst.example/api/v1/logs/oauth/callback",
            "sess-1.deadbeef",
        )
        .expect("builds");
        assert!(url.starts_with("https://github.com/login/oauth/authorize?"));
        assert!(url.contains("client_id=Iv1.abc"));
        // The redirect_uri is percent-encoded.
        assert!(url.contains("redirect_uri=https%3A%2F%2Ffkst.example"));
        assert!(url.contains("state=sess-1.deadbeef"));
    }
}
