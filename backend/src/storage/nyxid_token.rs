//! NyxID service-account OAuth2 client-credentials token provider.
//!
//! chrono-storage sits behind the NyxID proxy and expects a NyxID
//! service-account access token on every call. This provider mints one via the
//! standard OAuth2 client-credentials grant
//! (`POST {token_url}` with `grant_type=client_credentials` + `client_id` +
//! `client_secret`, `application/x-www-form-urlencoded`), caches it, and re-mints
//! it once it comes within [`REFRESH_BUFFER`] of expiry so a call never rides an
//! expired credential.
//!
//! Secret hygiene: the client secret and the minted token live only in
//! [`SecretString`]s; the secret rides the request body (never a URL, never a
//! log), the token is returned for use as a bearer header, and neither ever
//! appears in an error or in `Debug` output (both are proven by tests).

use std::time::{Duration, Instant};

use secrecy::{ExposeSecret, SecretString};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::config::ChronoStorageConfig;
use super::{scrub_transport, StorageError};

/// Re-mint the token once it is within this window of expiry, so an in-flight
/// call always carries a token with comfortable remaining lifetime.
const REFRESH_BUFFER: Duration = Duration::from_secs(60);

/// Fallback token lifetime when the OAuth2 response omits `expires_in`. A
/// standards-compliant server always sends it; this conservative default merely
/// avoids caching an unknown-lifetime token indefinitely.
const DEFAULT_TOKEN_TTL: Duration = Duration::from_secs(300);

/// The OAuth2 token-endpoint success body. Only the two fields we act on are
/// deserialized; `token_type` and any extras are ignored.
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// A cached access token and the instant it should be considered expired.
struct CachedToken {
    token: SecretString,
    expires_at: Instant,
}

/// Caches and refreshes a NyxID service-account access token over a shared
/// [`reqwest::Client`].
pub struct NyxidSaTokenProvider {
    http: reqwest::Client,
    token_url: String,
    client_id: String,
    client_secret: SecretString,
    /// The cached token, guarded by an async mutex. The lock is held across the
    /// mint so concurrent callers coalesce onto a single refresh rather than
    /// stampeding the token endpoint.
    cache: Mutex<Option<CachedToken>>,
}

// Manual `Debug` that redacts both the client secret and the cache (which holds
// the minted token) — the codebase convention (mirroring `GithubAppConfig`).
impl std::fmt::Debug for NyxidSaTokenProvider {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NyxidSaTokenProvider")
            .field("token_url", &self.token_url)
            .field("client_id", &self.client_id)
            .field("client_secret", &"<redacted>")
            .field("cache", &"<redacted>")
            .finish()
    }
}

impl NyxidSaTokenProvider {
    /// Build a provider over a shared HTTP client using the storage config's
    /// NyxID credentials.
    pub fn new(http: reqwest::Client, config: &ChronoStorageConfig) -> Self {
        Self {
            http,
            token_url: config.nyxid_token_url.clone(),
            client_id: config.nyxid_client_id.clone(),
            client_secret: config.nyxid_client_secret.clone(),
            cache: Mutex::new(None),
        }
    }

    /// Return a currently-valid access token, minting a fresh one on the first
    /// call and whenever the cached token is within [`REFRESH_BUFFER`] of expiry.
    pub async fn access_token(&self) -> Result<SecretString, StorageError> {
        let mut guard = self.cache.lock().await;

        if let Some(cached) = guard.as_ref() {
            // `checked_duration_since` is `None` once we are past `expires_at`
            // (already expired), which correctly falls through to a refresh.
            let fresh_enough = cached
                .expires_at
                .checked_duration_since(Instant::now())
                .map(|remaining| remaining > REFRESH_BUFFER)
                .unwrap_or(false);
            if fresh_enough {
                return Ok(cached.token.clone());
            }
        }

        let fresh = self.mint().await?;
        let token = fresh.token.clone();
        *guard = Some(fresh);
        Ok(token)
    }

    /// Perform the OAuth2 client-credentials request and parse the token.
    async fn mint(&self) -> Result<CachedToken, StorageError> {
        // The secret rides the form body only; `.form(..)` sets
        // `Content-Type: application/x-www-form-urlencoded`.
        let params = [
            ("grant_type", "client_credentials"),
            ("client_id", self.client_id.as_str()),
            ("client_secret", self.client_secret.expose_secret()),
        ];

        let response = self
            .http
            .post(&self.token_url)
            .form(&params)
            .send()
            .await
            .map_err(|e| {
                // `scrub_transport` keeps the URL (and thus any query) out of the
                // log/error; the body (with the secret) is never in a reqwest error.
                let detail = scrub_transport(&e);
                tracing::warn!(error = %detail, "nyxid token request transport error");
                StorageError::TokenTransport(detail)
            })?;

        let status = response.status();
        if !status.is_success() {
            tracing::warn!(status = %status, "nyxid token endpoint returned non-success");
            return Err(StorageError::TokenStatus {
                status: status.as_u16(),
            });
        }

        let body: TokenResponse = response.json().await.map_err(|e| {
            tracing::warn!(error = %scrub_transport(&e), "nyxid token response did not parse");
            StorageError::TokenMalformed
        })?;

        let ttl = body
            .expires_in
            .map(Duration::from_secs)
            .unwrap_or(DEFAULT_TOKEN_TTL);
        Ok(CachedToken {
            token: SecretString::from(body.access_token),
            expires_at: Instant::now() + ttl,
        })
    }
}

#[cfg(test)]
#[path = "nyxid_token_tests.rs"]
mod tests;
