//! JWKS public-key cache with lazy fetch, TTL-based refresh, and stale-if-error.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use jsonwebtoken::DecodingKey;
use tokio::sync::{Mutex, RwLock};

use super::AuthError;

/// Path appended to the NyxID base URL to fetch the JWKS.
const JWKS_PATH: &str = "/.well-known/jwks.json";

/// Timeout for the HTTP GET to the JWKS endpoint.
const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Minimum interval between refresh attempts triggered by an unknown `kid`.
/// Prevents a flood of refreshes when many requests arrive with an unseen kid
/// (e.g. during key rotation).
const UNKNOWN_KID_REFRESH_MIN: Duration = Duration::from_secs(10);

/// JWKS response shape from the `.well-known/jwks.json` endpoint.
#[derive(Debug, serde::Deserialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

/// A single JSON Web Key from the JWKS response.
#[derive(Debug, serde::Deserialize)]
struct Jwk {
    kid: String,
    kty: String,
    #[allow(dead_code)]
    alg: Option<String>,
    #[allow(dead_code)]
    r#use: Option<String>,
    n: String,
    e: String,
}

/// Cached state: the map of kid -> DecodingKey, and when it was fetched.
struct CacheState {
    keys: HashMap<String, Arc<DecodingKey>>,
    fetched_at: Instant,
    /// True when the most recent refresh attempt failed. Used to distinguish
    /// "kid genuinely absent from fresh keys" (401) from "kid absent but we
    /// cannot confirm because JWKS is down" (503).
    last_refresh_failed: bool,
}

impl CacheState {
    fn empty() -> Self {
        Self {
            keys: HashMap::new(),
            fetched_at: Instant::now() - Duration::from_secs(365 * 24 * 3600),
            last_refresh_failed: false,
        }
    }
}

/// A JWKS cache that lazily fetches public keys from a NyxID issuer and
/// refreshes them on TTL expiry or when an unknown `kid` is requested.
///
/// Thread-safe: a `tokio::sync::Mutex` serializes refresh attempts (single-
/// flight), while reads go through `RwLock` for concurrent access.
pub struct JwksCache {
    base_url: String,
    ttl: Duration,
    state: RwLock<CacheState>,
    /// Serializes refresh attempts so only one request is in-flight at a time.
    refresh_lock: Mutex<()>,
    /// Earliest instant at which a refresh triggered by an unknown kid is
    /// allowed. Rate-limited to prevent thundering-herd on key rotation.
    unknown_kid_floor: RwLock<Instant>,
    http: reqwest::Client,
}

impl std::fmt::Debug for JwksCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("JwksCache")
            .field("base_url", &self.base_url)
            .field("ttl", &self.ttl)
            .finish()
    }
}

impl JwksCache {
    /// Create a new cache. No network call is made until `key_for` is called.
    pub fn new(base_url: &str, ttl: Duration) -> Self {
        let http = reqwest::Client::builder()
            .timeout(FETCH_TIMEOUT)
            .build()
            .expect("reqwest client must build");
        Self {
            base_url: base_url.to_string(),
            ttl,
            state: RwLock::new(CacheState::empty()),
            refresh_lock: Mutex::new(()),
            unknown_kid_floor: RwLock::new(Instant::now()),
            http,
        }
    }

    /// Look up the decoding key for the given `kid`. Triggers a lazy fetch on
    /// first call, refreshes on TTL expiry, and re-fetches on unknown kid
    /// (rate-limited). Stale keys are served if a refresh fails.
    pub async fn key_for(&self, kid: &str) -> Result<Arc<DecodingKey>, AuthError> {
        // Fast path: read-lock, check TTL and kid presence.
        {
            let state = self.state.read().await;
            if state.fetched_at.elapsed() < self.ttl {
                if let Some(key) = state.keys.get(kid) {
                    return Ok(Arc::clone(key));
                }
                // TTL not expired, kid not found. We'll try a refresh below,
                // but only if we're past the rate-limit floor.
            }
        }

        // Slow path: attempt a refresh (single-flight).
        self.refresh_if_needed(kid).await?;

        // Re-read after potential refresh.
        let state = self.state.read().await;
        match state.keys.get(kid) {
            Some(key) => Ok(Arc::clone(key)),
            None => {
                if state.last_refresh_failed {
                    // JWKS is down and the kid is not in the stale cache. We
                    // cannot confirm the kid is truly absent, so surface this
                    // as a 503 (service unavailable) rather than 401.
                    Err(AuthError::JwksUnavailable(
                        "key not found and JWKS refresh failed".to_string(),
                    ))
                } else {
                    Err(AuthError::InvalidToken("unknown key id"))
                }
            }
        }
    }

    /// Perform a refresh if the TTL has expired or the kid is unknown (rate-
    /// limited). On failure, the existing cached keys are kept (stale-if-error).
    async fn refresh_if_needed(&self, kid: &str) -> Result<(), AuthError> {
        // Check whether we need to refresh at all (TTL or unknown kid).
        {
            let state = self.state.read().await;
            let ttl_ok = state.fetched_at.elapsed() < self.ttl;
            if ttl_ok && state.keys.contains_key(kid) {
                return Ok(());
            }
        }

        // Rate-limit unknown-kid refreshes.
        {
            let floor = self.unknown_kid_floor.read().await;
            if Instant::now() < *floor {
                // Rate-limited: skip refresh, let the caller try with stale keys.
                return Ok(());
            }
        }

        // Single-flight: only one refresh at a time.
        let _guard = self.refresh_lock.lock().await;

        // Double-check after acquiring the lock (another request may have
        // refreshed while we waited).
        {
            let state = self.state.read().await;
            if state.fetched_at.elapsed() < self.ttl && state.keys.contains_key(kid) {
                return Ok(());
            }
        }

        match self.fetch_jwks().await {
            Ok(new_keys) => {
                let mut state = self.state.write().await;
                state.keys = new_keys;
                state.fetched_at = Instant::now();
                state.last_refresh_failed = false;
                tracing::debug!(keys = state.keys.len(), "JWKS cache refreshed");
                Ok(())
            }
            Err(e) => {
                // Stale-if-error: log the failure but keep serving old keys.
                // Known kids are served from the stale cache. Unknown kids
                // get 503 (not 401) because we cannot confirm the kid is
                // truly absent when the JWKS is down.
                tracing::warn!(error = %e, "JWKS fetch failed; serving stale keys");
                {
                    let mut state = self.state.write().await;
                    state.last_refresh_failed = true;
                }
                // Update the rate-limit floor so we don't hammer a down issuer.
                let mut floor = self.unknown_kid_floor.write().await;
                *floor = Instant::now() + UNKNOWN_KID_REFRESH_MIN;
                Ok(())
            }
        }
    }

    /// Fetch and parse the JWKS endpoint.
    async fn fetch_jwks(&self) -> Result<HashMap<String, Arc<DecodingKey>>, AuthError> {
        let url = format!("{}{JWKS_PATH}", self.base_url);
        let response = self
            .http
            .get(&url)
            .send()
            .await
            .map_err(|e| AuthError::JwksUnavailable(format!("HTTP GET failed: {e}")))?;

        if !response.status().is_success() {
            let status = response.status();
            return Err(AuthError::JwksUnavailable(format!(
                "JWKS endpoint returned {status}"
            )));
        }

        let jwks: JwksResponse = response
            .json()
            .await
            .map_err(|e| AuthError::JwksUnavailable(format!("JWKS parse failed: {e}")))?;

        let mut keys = HashMap::new();
        for jwk in jwks.keys {
            if jwk.kty != "RSA" {
                continue;
            }
            match DecodingKey::from_rsa_components(&jwk.n, &jwk.e) {
                Ok(key) => {
                    keys.insert(jwk.kid, Arc::new(key));
                }
                Err(e) => {
                    tracing::warn!(error = %e, "skipping unparseable JWKS key");
                }
            }
        }

        if keys.is_empty() {
            return Err(AuthError::JwksUnavailable(
                "JWKS endpoint returned no RSA keys".to_string(),
            ));
        }

        Ok(keys)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_cache_builds_without_network() {
        let cache = JwksCache::new("https://example.com", Duration::from_secs(300));
        assert_eq!(cache.base_url, "https://example.com");
    }

    #[test]
    fn debug_output_does_not_leak_keys() {
        let cache = JwksCache::new("https://example.com", Duration::from_secs(300));
        let rendered = format!("{cache:?}");
        assert!(rendered.contains("https://example.com"));
        assert!(!rendered.contains("keys"));
    }
}
