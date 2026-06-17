//! Startup wiring decisions that are pure enough to unit-test in isolation.
//!
//! The binary entrypoint (`main.rs`) is an imperative side-effecting sequence
//! that cannot be exercised without a real TCP bind, so the one decision with
//! genuine branching — HOW the shared [`NyxIdClient`] is constructed under the
//! owner-only credential model (#219) — is lifted here as a pure function.
//!
//! Owner-only model (#219): the NyxID client is built whenever auth is enabled
//! (the `AuthMode::Enabled` settings carry the issuer base URL), because every
//! feature it drives by default — per-session key mint (#111), the Ornn proxy
//! (#114), the github_hub connections lookups, and repo-create — authenticates
//! with the FORWARDED USER TOKEN, not the service account. The service account
//! (`NYXID_CLIENT_ID/SECRET`) stays OPTIONAL: present, it ALSO enables the
//! SA-only org features; absent, the user-token paths are unaffected.

use std::time::Duration;

use fkst_shared::nyxid::{NyxIdClient, NyxIdError};

use crate::auth::AuthMode;

/// Build the shared [`NyxIdClient`] for the given auth mode and OPTIONAL
/// service-account credentials, per the owner-only model (#219).
///
/// - Auth disabled → `Ok(None)`: no NyxID host is configured, so per-session
///   token provisioning and Ornn stay off (pre-#111 behaviour).
/// - Auth enabled, SA creds present → a full SA-backed client (org features on).
/// - Auth enabled, SA creds absent → an owner-only client built from the base
///   URL alone (org features off; user-token paths fully functional).
///
/// A non-`None` result is precisely the condition under which `main` enables
/// per-session token provisioning AND Ornn — so callers gate both on this.
pub fn build_nyxid_client(
    auth_mode: &AuthMode,
    client_id: Option<&str>,
    client_secret: Option<&secrecy::SecretString>,
    github_proxy_slug: &str,
    org_cache_ttl: Duration,
) -> Result<Option<NyxIdClient>, NyxIdError> {
    let AuthMode::Enabled(settings) = auth_mode else {
        return Ok(None);
    };
    let client = match (client_id, client_secret) {
        (Some(id), Some(secret)) => NyxIdClient::new(
            &settings.base_url,
            github_proxy_slug,
            id.to_string(),
            secret.clone(),
            org_cache_ttl,
        )?,
        // No (or partial — config rejects partial at load) service account:
        // owner-only client driving only the user-token paths.
        _ => NyxIdClient::new_owner_only(&settings.base_url, github_proxy_slug, org_cache_ttl)?,
    };
    Ok(Some(client))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::NyxIdAuthSettings;
    use crate::nyxid::DEFAULT_GITHUB_PROXY_SLUG;
    use secrecy::SecretString;

    fn enabled() -> AuthMode {
        AuthMode::Enabled(NyxIdAuthSettings {
            base_url: "https://nyxid.example.com".to_string(),
        })
    }

    /// The owner-only baseline (#219): auth enabled + base URL + NO
    /// NYXID_CLIENT_ID/SECRET must build a client (so per-session token + Ornn
    /// turn on) that reports NO service account (so org features stay off).
    #[test]
    fn owner_only_auth_enabled_without_sa_builds_user_token_client() {
        let client = build_nyxid_client(
            &enabled(),
            None,
            None,
            DEFAULT_GITHUB_PROXY_SLUG,
            Duration::from_secs(30),
        )
        .expect("client build")
        .expect("a client is built whenever auth is enabled");
        // A built client is the gate for per-session token + Ornn enablement.
        assert!(
            !client.has_service_account(),
            "owner-only client must carry no service account"
        );
    }

    /// SA-present path is unchanged: a client is built AND reports a service
    /// account, so the SA-only org features stay available.
    #[test]
    fn auth_enabled_with_sa_builds_service_account_client() {
        let secret = SecretString::from("sas_secret".to_string());
        let client = build_nyxid_client(
            &enabled(),
            Some("sa_id"),
            Some(&secret),
            DEFAULT_GITHUB_PROXY_SLUG,
            Duration::from_secs(30),
        )
        .expect("client build")
        .expect("a client is built whenever auth is enabled");
        assert!(
            client.has_service_account(),
            "an SA-configured client must report a service account"
        );
    }

    /// Auth disabled → no client, so per-session token + Ornn stay disabled
    /// (pre-#111 behaviour), regardless of SA creds being present.
    #[test]
    fn auth_disabled_builds_no_client() {
        let built = build_nyxid_client(
            &AuthMode::Disabled,
            None,
            None,
            DEFAULT_GITHUB_PROXY_SLUG,
            Duration::from_secs(30),
        )
        .expect("client build");
        assert!(built.is_none(), "auth disabled must build no NyxID client");
    }
}
