//! Deterministic derivation of a per-session execd access token (issue #417).
//!
//! execd (the in-sandbox exec daemon, reached through the lifecycle proxy) gates
//! every request on an `X-EXECD-ACCESS-TOKEN` header. Rather than persist a random
//! token per sandbox, we derive it deterministically from a single long-lived seed
//! secret + the session id, so any control-plane replica can recompute the exact
//! token for a session without shared state. HMAC (keyed) — NOT a bare hash — so the
//! token is unforgeable without the seed.
//!
//! Reuses the repo's hand-rolled `hmac` + `sha2` primitives (the same
//! `type HmacSha256 = Hmac<Sha256>` alias as `github_app_webhook::verify`) and the
//! lowercase-hex fold from `routes::logs::oauth::mac_hex` — no `hex` crate. The
//! result is a [`SecretString`] so it redacts in `Debug` and is never logged.

use hmac::{Hmac, Mac};
use secrecy::{ExposeSecret, SecretString};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Derive a session's execd access token: lowercase-hex `HMAC-SHA256(seed,
/// session_id)`.
///
/// Deterministic (same seed + id -> same token) and unforgeable without `seed`.
/// The returned [`SecretString`] must never be logged; the sole place it is exposed
/// is the execd client's header-stamping choke-point.
pub fn derive_execd_token(seed: &SecretString, session_id: &str) -> SecretString {
    let mut mac = HmacSha256::new_from_slice(seed.expose_secret().as_bytes())
        .expect("HMAC accepts any key length");
    mac.update(session_id.as_bytes());
    let hex = mac
        .finalize()
        .into_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    SecretString::from(hex)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derive_execd_token_matches_known_answer_vector() {
        // Known-answer vector precomputed offline (tool noted for reproducibility):
        //   printf '%s' "sess-1" | openssl dgst -sha256 -hmac "execd-seed"
        // -> 24723d83a9822cfa44a46bb3297cb4bcb644c1c68334c655b2c136f772372a17
        let token = derive_execd_token(&SecretString::from("execd-seed"), "sess-1");
        assert_eq!(
            token.expose_secret(),
            "24723d83a9822cfa44a46bb3297cb4bcb644c1c68334c655b2c136f772372a17"
        );
    }

    #[test]
    fn derive_execd_token_is_deterministic_and_session_scoped() {
        let seed = SecretString::from("execd-seed");
        // Same inputs -> same token.
        assert_eq!(
            derive_execd_token(&seed, "sess-1").expose_secret(),
            derive_execd_token(&seed, "sess-1").expose_secret()
        );
        // A different session id yields a different token.
        assert_ne!(
            derive_execd_token(&seed, "sess-1").expose_secret(),
            derive_execd_token(&seed, "sess-2").expose_secret()
        );
    }

    #[test]
    fn derived_token_debug_never_leaks_the_token() {
        let token = derive_execd_token(&SecretString::from("execd-seed"), "sess-1");
        let debug = format!("{token:?}");
        assert!(
            !debug.contains(token.expose_secret()),
            "token leaked in Debug output: {debug}"
        );
    }
}
