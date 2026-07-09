//! Typed configuration for the on-demand session-log download surface.
//!
//! A single envy pass over the bare `FKST_` prefix (mirroring
//! [`crate::reconcile_config`] and [`crate::storage::config`]) collects the knobs
//! the identity-gated `/api/v1/logs/{session_id}` endpoint + the announce-comment
//! link need:
//!
//! - `FKST_LOG_ADMINS` — comma-separated GitHub logins/ids that may pull ANY
//!   session's logs (tier 3 of [`crate::reconcile::log_authz`]).
//! - `FKST_PUBLIC_BASE_URL` — the externally-reachable base URL the announce comment
//!   builds the download link from (e.g. `https://fkst.example`). When unset, the
//!   announce comment omits the log line (there is no reachable URL to advertise).
//! - `FKST_GITHUB_OAUTH_CLIENT_ID` / `FKST_GITHUB_OAUTH_CLIENT_SECRET` — the GitHub
//!   App's *user* OAuth credentials, used by the endpoint's BROWSER mode (redirect →
//!   callback code-exchange). Both or neither; the secret is held in a
//!   [`SecretString`] and never logged.
//! - `FKST_GITHUB_OAUTH_BASE_URL` — the OAuth host for the authorize + token-exchange
//!   calls. Default `https://github.com` (overridable for GitHub Enterprise or a
//!   test mock; distinct from the REST `FKST_GITHUB_API_BASE_URL`).
//! - `FKST_FRONTEND_URL` — the frontend base URL the OAuth *login* callback
//!   (`crate::routes::auth`) redirects back to, handing the SPA its token in the URL
//!   fragment. When unset, the frontend login flow is unavailable.
//!
//! Every knob is OPTIONAL (the feature degrades gracefully), with ONE fail-closed:
//! configuring exactly one of the OAuth id/secret pair is an operator mistake (the
//! browser flow needs both), so it is rejected naming the missing half.

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;

/// Bare `FKST_` prefix so the keys read naturally (`FKST_LOG_ADMINS`,
/// `FKST_PUBLIC_BASE_URL`). envy drops every field it does not recognize, so this
/// pass sees only its own keys and never collides with the other bare-`FKST_`
/// passes ([`crate::reconcile_config`], [`crate::storage::config`]).
const LOG_ENV_PREFIX: &str = "FKST_";

/// The default OAuth host for the browser-login authorize + token-exchange calls.
const DEFAULT_OAUTH_BASE_URL: &str = "https://github.com";

/// The bare `FKST_`-prefixed variables for the log-download surface. All optional at
/// the envy layer; the presence/absence policy is applied in [`LogConfig::from_vars`].
#[derive(Debug, Deserialize)]
struct LogVars {
    /// Comma-separated logins/ids; parsed as a single String (not a Vec) to sidestep
    /// envy's Vec handling, split in [`LogConfig::from_vars`].
    #[serde(default)]
    log_admins: Option<String>,
    #[serde(default)]
    public_base_url: Option<String>,
    #[serde(default)]
    github_oauth_client_id: Option<String>,
    #[serde(default)]
    github_oauth_client_secret: Option<String>,
    #[serde(default)]
    github_oauth_base_url: Option<String>,
    #[serde(default)]
    frontend_url: Option<String>,
}

/// Resolved log-download configuration. Every field is optional except the OAuth
/// base URL (which has a sensible default), so the surface degrades gracefully when
/// unconfigured.
#[derive(Clone, Debug)]
pub struct LogConfig {
    /// Global admins (logins and/or numeric ids) allowed to download ANY session's
    /// logs. Env: `FKST_LOG_ADMINS` (comma-separated). Empty when unset.
    pub admins: Vec<String>,
    /// Externally-reachable base URL the download link is built from. Env:
    /// `FKST_PUBLIC_BASE_URL`. `None` (blank coerced) → the announce comment omits
    /// the log line.
    pub public_base_url: Option<String>,
    /// The GitHub App's user-OAuth client id (browser mode). Env:
    /// `FKST_GITHUB_OAUTH_CLIENT_ID`. `None` (blank coerced) → browser mode is
    /// unavailable and the endpoint tells the caller to pass a Bearer token instead.
    pub oauth_client_id: Option<String>,
    /// The GitHub App's user-OAuth client secret (browser mode). Env:
    /// `FKST_GITHUB_OAUTH_CLIENT_SECRET`. Held in a [`SecretString`]; never logged.
    /// Also doubles as the HMAC key that signs the OAuth `state` (CSRF protection).
    pub oauth_client_secret: Option<SecretString>,
    /// The OAuth host for authorize + token-exchange. Env:
    /// `FKST_GITHUB_OAUTH_BASE_URL`. Default `https://github.com`.
    pub oauth_base_url: String,
    /// The frontend URL the login callback redirects back to (with the issued
    /// token in the URL fragment). Env: `FKST_FRONTEND_URL`. `None` (blank
    /// coerced) → the frontend login flow is unavailable (503).
    pub frontend_url: Option<String>,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            admins: Vec::new(),
            public_base_url: None,
            oauth_client_id: None,
            oauth_client_secret: None,
            oauth_base_url: DEFAULT_OAUTH_BASE_URL.to_string(),
            frontend_url: None,
        }
    }
}

/// Trim a raw env value; a blank string is treated as absent so a stray empty
/// ConfigMap value never masquerades as a real setting.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl LogConfig {
    /// Deserialize the log-download configuration from environment-style pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the process
    /// environment; shares the caller's already-collected `vars` snapshot (see
    /// [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<LogConfig, AppError> {
        let raw: LogVars = envy::prefixed(LOG_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;

        // Split the comma-separated admin list, trimming, stripping a leading `@`,
        // and dropping empties. An unset/blank var yields an empty allow-list.
        let admins: Vec<String> = raw
            .log_admins
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().trim_start_matches('@').to_string())
            .filter(|s| !s.is_empty())
            .collect();

        let oauth_client_id = non_blank(raw.github_oauth_client_id);
        let oauth_client_secret = non_blank(raw.github_oauth_client_secret);
        // Fail closed on a half-configured OAuth pair: the browser login flow needs
        // BOTH the id and the secret, so exactly one set is an operator mistake.
        match (&oauth_client_id, &oauth_client_secret) {
            (Some(_), None) => {
                return Err(AppError::Config(
                    "FKST_GITHUB_OAUTH_CLIENT_SECRET must be set when \
                     FKST_GITHUB_OAUTH_CLIENT_ID is set (browser log-download login \
                     needs both)"
                        .to_string(),
                ))
            }
            (None, Some(_)) => {
                return Err(AppError::Config(
                    "FKST_GITHUB_OAUTH_CLIENT_ID must be set when \
                     FKST_GITHUB_OAUTH_CLIENT_SECRET is set (browser log-download login \
                     needs both)"
                        .to_string(),
                ))
            }
            _ => {}
        }

        let oauth_base_url = non_blank(raw.github_oauth_base_url)
            .unwrap_or_else(|| DEFAULT_OAUTH_BASE_URL.to_string());

        Ok(LogConfig {
            admins,
            public_base_url: non_blank(raw.public_base_url),
            oauth_client_id,
            oauth_client_secret: oauth_client_secret.map(SecretString::from),
            oauth_base_url,
            frontend_url: non_blank(raw.frontend_url),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_nothing_is_set() {
        let config = LogConfig::from_vars(&vars(&[])).expect("defaults deserialize");
        assert!(config.admins.is_empty());
        assert_eq!(config.public_base_url, None);
        assert_eq!(config.oauth_client_id, None);
        assert!(config.oauth_client_secret.is_none());
        assert_eq!(config.oauth_base_url, "https://github.com");
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = LogConfig::from_vars(&vars(&[])).expect("defaults");
        let from_default = LogConfig::default();
        assert_eq!(from_default.admins, from_env.admins);
        assert_eq!(from_default.public_base_url, from_env.public_base_url);
        assert_eq!(from_default.oauth_client_id, from_env.oauth_client_id);
        assert_eq!(from_default.oauth_base_url, from_env.oauth_base_url);
    }

    #[test]
    fn admins_are_split_trimmed_and_at_stripped() {
        let config = LogConfig::from_vars(&vars(&[("FKST_LOG_ADMINS", " @ops, 12345 ,, alice ")]))
            .expect("admins parse");
        assert_eq!(config.admins, vec!["ops", "12345", "alice"]);
    }

    #[test]
    fn public_base_url_and_oauth_pair_are_read() {
        let config = LogConfig::from_vars(&vars(&[
            ("FKST_PUBLIC_BASE_URL", "https://fkst.example/"),
            ("FKST_GITHUB_OAUTH_CLIENT_ID", "Iv1.abc"),
            ("FKST_GITHUB_OAUTH_CLIENT_SECRET", "shh"),
            ("FKST_GITHUB_OAUTH_BASE_URL", "https://ghe.example"),
            ("FKST_FRONTEND_URL", "https://app.example/fkst/"),
        ]))
        .expect("full config loads");
        assert_eq!(
            config.public_base_url.as_deref(),
            Some("https://fkst.example/")
        );
        assert_eq!(config.oauth_client_id.as_deref(), Some("Iv1.abc"));
        assert_eq!(
            config.oauth_client_secret.as_ref().unwrap().expose_secret(),
            "shh"
        );
        assert_eq!(config.oauth_base_url, "https://ghe.example");
        assert_eq!(
            config.frontend_url.as_deref(),
            Some("https://app.example/fkst/")
        );
    }

    #[test]
    fn blank_values_are_treated_as_unset() {
        let config = LogConfig::from_vars(&vars(&[
            ("FKST_PUBLIC_BASE_URL", "   "),
            ("FKST_GITHUB_OAUTH_BASE_URL", "  "),
        ]))
        .expect("blank is unset");
        assert_eq!(config.public_base_url, None);
        // A blank OAuth base falls back to the default, never an empty (unusable) URL.
        assert_eq!(config.oauth_base_url, "https://github.com");
    }

    #[test]
    fn half_configured_oauth_pair_fails_closed_naming_the_missing_half() {
        let only_id = LogConfig::from_vars(&vars(&[("FKST_GITHUB_OAUTH_CLIENT_ID", "Iv1.abc")]))
            .expect_err("id without secret must fail");
        assert!(only_id
            .to_string()
            .contains("FKST_GITHUB_OAUTH_CLIENT_SECRET"));

        let only_secret =
            LogConfig::from_vars(&vars(&[("FKST_GITHUB_OAUTH_CLIENT_SECRET", "shh")]))
                .expect_err("secret without id must fail");
        assert!(only_secret
            .to_string()
            .contains("FKST_GITHUB_OAUTH_CLIENT_ID"));
    }

    #[test]
    fn client_secret_is_redacted_in_debug_output() {
        let config = LogConfig::from_vars(&vars(&[
            ("FKST_GITHUB_OAUTH_CLIENT_ID", "Iv1.abc"),
            ("FKST_GITHUB_OAUTH_CLIENT_SECRET", "super-secret"),
        ]))
        .expect("valid");
        let debug = format!("{config:?}");
        assert!(
            !debug.contains("super-secret"),
            "Debug leaked the client secret: {debug}"
        );
    }
}
