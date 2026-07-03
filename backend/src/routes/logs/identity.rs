//! Token → verified-identity resolution for the log-download endpoint, with a brief
//! in-memory cache.
//!
//! Both the API path (an `Authorization: Bearer` header) and the browser path (a
//! token freshly minted by the OAuth callback) establish WHO is calling by trading
//! the token for the caller's `{login, id}` via `GET {api_base}/user` — reusing the
//! same verifier the rest of the control plane uses ([`crate::github_identity`]).
//!
//! To stay rate-limit friendly under repeated downloads with the same token, the
//! result is cached briefly. The cache key is a SHA-256 HASH of the token, never the
//! token itself, so a raw credential never lives in the cache (and can never be
//! dumped by a `{:?}` of it). A cache entry expires after [`CACHE_TTL`].

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

/// The token-hash → (identity, inserted-at) cache. Keyed by `sha256(token)` hex so
/// the raw token is never stored.
fn cache() -> &'static Mutex<HashMap<String, (GithubUser, Instant)>> {
    static CACHE: OnceLock<Mutex<HashMap<String, (GithubUser, Instant)>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The SHA-256 hex of a token — the cache key. NEVER reversible to the token.
fn token_key(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    digest.iter().map(|b| format!("{b:02x}")).collect()
}

/// Resolve `token` to the verified GitHub identity, serving a fresh cache hit when
/// available and otherwise calling `GET {api_base}/user`. The token is used ONLY for
/// that call and is never logged or stored in the clear.
///
/// - A rejected token (`/user` 401/403) → [`AppError::Unauthorized`] (deny).
/// - An unreachable/misbehaving GitHub → [`AppError::Unavailable`].
pub(super) async fn resolve(api_base: &str, token: &str) -> Result<GithubUser, AppError> {
    let key = token_key(token);

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

    #[test]
    fn token_key_is_a_hash_never_the_token() {
        let token = "ghp_super_secret_token_value";
        let key = token_key(token);
        assert_ne!(key, token, "the key must not be the raw token");
        assert!(
            !key.contains("secret"),
            "the key must not embed the token: {key}"
        );
        // 32-byte SHA-256 → 64 hex chars, stable for the same input.
        assert_eq!(key.len(), 64);
        assert_eq!(key, token_key(token));
    }
}
