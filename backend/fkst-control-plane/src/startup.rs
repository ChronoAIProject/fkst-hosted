//! Startup wiring decisions that are pure enough to unit-test in isolation.
//!
//! The binary entrypoint (`main.rs`) is an imperative side-effecting sequence
//! that cannot be exercised without a real TCP bind, so the one decision with
//! genuine branching — WHETHER the shared [`NyxIdClient`] is constructed under
//! the owner-only credential model (#257) — is lifted here as a pure function.
//!
//! Owner-only model (#257): the NyxID client is built whenever auth is enabled
//! (the `AuthMode::Enabled` settings carry the issuer base URL), because every
//! feature it drives — per-session key mint (#111), the Ornn proxy (#114), the
//! github_hub connections lookups, and repo-create — authenticates with the
//! FORWARDED USER TOKEN. There is no service account: the control plane needs
//! only the NyxID base URL, no client credential.

use std::time::Duration;

use fkst_shared::nyxid::{NyxIdClient, NyxIdError};

use crate::auth::AuthMode;

/// Build the shared owner-only [`NyxIdClient`] for the given auth mode (#257).
///
/// - Auth disabled → `Ok(None)`: no NyxID host is configured, so per-session
///   token provisioning and Ornn stay off (pre-#111 behaviour).
/// - Auth enabled → an owner-only client built from the issuer base URL; every
///   path it drives authenticates with the forwarded user token.
///
/// A non-`None` result is precisely the condition under which `main` enables
/// per-session token provisioning AND Ornn — so callers gate both on this.
pub fn build_nyxid_client(
    auth_mode: &AuthMode,
    github_proxy_slug: &str,
    org_cache_ttl: Duration,
) -> Result<Option<NyxIdClient>, NyxIdError> {
    let AuthMode::Enabled(settings) = auth_mode else {
        return Ok(None);
    };
    let client = NyxIdClient::new(&settings.base_url, github_proxy_slug, org_cache_ttl)?;
    Ok(Some(client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::NyxIdAuthSettings;
    use crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG;

    fn enabled() -> AuthMode {
        AuthMode::Enabled(NyxIdAuthSettings {
            base_url: "https://nyxid.example.com".to_string(),
        })
    }

    /// Auth enabled + base URL must build an owner-only client (#257) — the gate
    /// that turns on per-session token provisioning and Ornn.
    #[test]
    fn auth_enabled_builds_owner_only_client() {
        let built = build_nyxid_client(
            &enabled(),
            DEFAULT_GITHUB_PROXY_SLUG,
            Duration::from_secs(30),
        )
        .expect("client build");
        assert!(
            built.is_some(),
            "a client is built whenever auth is enabled"
        );
    }

    /// Auth disabled → no client, so per-session token + Ornn stay disabled
    /// (pre-#111 behaviour).
    #[test]
    fn auth_disabled_builds_no_client() {
        let built = build_nyxid_client(
            &AuthMode::Disabled,
            DEFAULT_GITHUB_PROXY_SLUG,
            Duration::from_secs(30),
        )
        .expect("client build");
        assert!(built.is_none(), "auth disabled must build no NyxID client");
    }
}
