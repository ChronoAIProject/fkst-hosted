//! Token → verified-identity resolution for the log-download endpoint, with a brief
//! in-memory cache.
//!
//! Both the API path (an `Authorization: Bearer` header) and the browser path (a
//! token freshly minted by the OAuth callback) establish WHO is calling by trading
//! the token for the caller's `{login, id}` via `GET {api_base}/user` — reusing the
//! same verifier the rest of the control plane uses ([`crate::github_identity`]).
//!
//! To stay rate-limit friendly under repeated downloads with the same token, the
//! result is cached briefly. The cache key is a SHA-256 HASH of the API base AND the
//! token, never the token itself, so a raw credential never lives in the cache (and
//! can never be dumped by a `{:?}` of it). A cache entry expires after [`CACHE_TTL`].
//!
//! The API base is part of the key because an identity is only meaningful relative to
//! the GitHub instance that vouched for it: the same token STRING presented against a
//! different `api_base` is a different assertion and must be re-verified, not answered
//! from a neighbouring instance's cached `{login, id}`. A single deployment reads one
//! fixed base, so this costs nothing in production — but keying on the token alone
//! made the cache a place where one instance's identity could answer for another's,
//! which is exactly the confusion the rest of this milestone exists to prevent.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use sha2::{Digest, Sha256};

use crate::error::AppError;
use crate::github_identity::{verify_token, GithubUser};

/// How long a resolved `token -> identity` mapping is trusted before re-verifying.
/// Short: an identity is stable, but a revoked token should stop working quickly.
const CACHE_TTL: Duration = Duration::from_secs(60);

/// Cap on cached entries; a full cache drops the oldest-expired first, then refuses
/// to grow unboundedly under a flood of distinct tokens (bounded memory).
const CACHE_MAX_ENTRIES: usize = 4096;

/// The credential-hash → (identity, inserted-at) cache. Keyed by
/// `sha256(api_base, token)` hex so the raw token is never stored.
fn cache() -> &'static Mutex<HashMap<String, (GithubUser, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (GithubUser, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The SHA-256 hex of `(api_base, token)` — the cache key. NEVER reversible to the
/// token. The base is length-prefixed rather than merely concatenated so no pair of
/// distinct `(base, token)` inputs can hash to one key by straddling the boundary.
fn token_key(api_base: &str, token: &str) -> String {
    let mut digest = Sha256::new();
    digest.update((api_base.len() as u64).to_be_bytes());
    digest.update(api_base.as_bytes());
    digest.update(token.as_bytes());
    digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Resolve `token` to the verified GitHub identity, serving a fresh cache hit when
/// available and otherwise calling `GET {api_base}/user`. The token is used ONLY for
/// that call and is never logged or stored in the clear.
///
/// - A rejected token (`/user` 401/403) → [`AppError::Unauthorized`] (deny).
/// - An unreachable/misbehaving GitHub → [`AppError::Unavailable`].
pub(crate) async fn resolve(api_base: &str, token: &str) -> Result<GithubUser, AppError> {
    let key = token_key(api_base, token);

    if let Some(user) = cache_get(&key) {
        return Ok(user);
    }

    // Cache miss (or expired): verify against GitHub. `verify_token` never logs the
    // token; it maps 401/403 → Unauthorized and 5xx/transport → Unavailable.
    let user = verify_token(api_base, token).await?;
    cache_put(key, user.clone());
    Ok(user)
}

/// Read a still-fresh cache entry, cloning the identity out.
fn cache_get(key: &str) -> Option<GithubUser> {
    let map = cache().lock().unwrap_or_else(|e| e.into_inner());
    map.get(key).and_then(|(user, at)| {
        if at.elapsed() < CACHE_TTL {
            Some(user.clone())
        } else {
            None
        }
    })
}

/// Insert a fresh entry, pruning expired ones (and, if still at the cap, clearing to
/// stay bounded). Best-effort: the cache is a rate-limit optimisation, not a
/// correctness requirement, so a poisoned lock is recovered rather than propagated.
fn cache_put(key: String, user: GithubUser) {
    let mut map = cache().lock().unwrap_or_else(|e| e.into_inner());
    map.retain(|_, (_, at)| at.elapsed() < CACHE_TTL);
    if map.len() >= CACHE_MAX_ENTRIES {
        // A pathological flood of distinct tokens: reset rather than grow unbounded.
        map.clear();
    }
    map.insert(key, (user, Instant::now()));
}

/// Clear the identity cache. TEST-ONLY: the cache is a process-global shared across
/// the whole test binary, so a test that reuses a token STRING for a different mocked
/// identity must reset it first to stay isolated from sibling tests.
#[cfg(test)]
pub(crate) fn clear_cache() {
    cache().lock().unwrap_or_else(|e| e.into_inner()).clear();
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: &str = "https://api.github.test";

    #[test]
    fn token_key_is_a_hash_never_the_token() {
        let token = "ghp_super_secret_token_value";
        let key = token_key(BASE, token);
        assert_ne!(key, token, "the key must not be the raw token");
        assert!(
            !key.contains("secret"),
            "the key must not embed the token: {key}"
        );
        assert!(
            !key.contains("github"),
            "the key must not embed the api base: {key}"
        );
        // 32-byte SHA-256 → 64 hex chars, stable for the same input.
        assert_eq!(key.len(), 64);
        assert_eq!(key, token_key(BASE, token));
    }

    /// One GitHub instance's cached identity must never answer for another's. The
    /// same token string against a different base is a different assertion, so it
    /// gets a different key and is re-verified rather than served from the cache.
    #[test]
    fn the_same_token_against_a_different_api_base_is_a_different_key() {
        let token = "user-token";
        assert_ne!(
            token_key(BASE, token),
            token_key("https://ghe.internal.test", token),
            "the api base must participate in the key"
        );
    }

    /// The base is length-prefixed, so no two distinct `(base, token)` pairs can
    /// collide by shifting bytes across the boundary between them.
    #[test]
    fn the_base_token_boundary_cannot_be_straddled() {
        assert_ne!(
            token_key("https://a", "bc"),
            token_key("https://ab", "c"),
            "a concatenation-only key would make these identical"
        );
    }

    /// A resolved identity is served from the cache for the SAME (base, token), which
    /// is the rate-limit optimisation the cache exists for.
    #[test]
    fn a_cached_identity_round_trips_for_the_same_base_and_token() {
        let key = token_key(BASE, "round-trip-token");
        cache_put(
            key.clone(),
            GithubUser {
                login: "alice".to_string(),
                id: 7,
            },
        );
        let hit = cache_get(&key).expect("a fresh entry is served");
        assert_eq!(hit.id, 7);
        assert!(
            cache_get(&token_key("https://other.test", "round-trip-token")).is_none(),
            "a different base must miss"
        );
    }
}
