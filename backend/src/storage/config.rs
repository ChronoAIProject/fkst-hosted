//! Typed configuration for the optional chrono-storage log-streaming feature.
//!
//! A single envy pass over the bare `FKST_` prefix (mirroring
//! [`crate::reconcile_config`]) collects the five knobs that point the control
//! plane at chrono-storage (a MinIO/S3 front behind the NyxID proxy) and the
//! NyxID service-account OAuth2 client-credentials grant that authenticates every
//! call:
//!
//! - `FKST_STORAGE_BASE_URL` — the proxied chrono-storage base URL.
//! - `FKST_STORAGE_BUCKET` — the bucket every object lands in.
//! - `FKST_NYXID_TOKEN_URL` — the OAuth2 `token` endpoint.
//! - `FKST_NYXID_CLIENT_ID` — the service-account client id.
//! - `FKST_NYXID_CLIENT_SECRET` — the service-account client secret (held in a
//!   [`SecretString`], never logged, redacted in `Debug`).
//!
//! The whole feature is OPTIONAL. When NONE of the five vars are set the config
//! resolves to `None` (log streaming stays disabled and the process boots
//! normally). A PARTIAL configuration — some set, some missing — is a genuine
//! operator mistake, so it fails closed naming the missing variables rather than
//! silently half-enabling a feature that would then error on first use.

use secrecy::SecretString;
use serde::Deserialize;

use crate::error::AppError;

/// Bare `FKST_` prefix so the keys read naturally
/// (`FKST_STORAGE_BASE_URL`, `FKST_NYXID_CLIENT_ID`). envy drops every field it
/// does not recognize, so this pass sees only the five storage keys and never
/// collides with the other bare-`FKST_` passes ([`crate::reconcile_config`]).
const STORAGE_ENV_PREFIX: &str = "FKST_";

/// The bare `FKST_`-prefixed variables for the chrono-storage feature. All
/// optional at the envy layer; the presence/absence policy is applied in
/// [`ChronoStorageConfig::from_vars`].
#[derive(Debug, Deserialize)]
struct StorageVars {
    #[serde(default)]
    storage_base_url: Option<String>,
    #[serde(default)]
    storage_bucket: Option<String>,
    #[serde(default)]
    nyxid_token_url: Option<String>,
    #[serde(default)]
    nyxid_client_id: Option<String>,
    #[serde(default)]
    nyxid_client_secret: Option<String>,
}

/// Resolved chrono-storage configuration. Present only when all five vars are
/// configured; the feature is otherwise disabled (`None`).
#[derive(Clone)]
pub struct ChronoStorageConfig {
    /// Base URL of the proxied chrono-storage service. Env:
    /// `FKST_STORAGE_BASE_URL`. Trailing slashes are trimmed by the client.
    pub base_url: String,
    /// Bucket every object is written to / read from. Env: `FKST_STORAGE_BUCKET`.
    pub bucket: String,
    /// NyxID OAuth2 `token` endpoint (client-credentials grant). Env:
    /// `FKST_NYXID_TOKEN_URL`.
    pub nyxid_token_url: String,
    /// NyxID service-account client id. Env: `FKST_NYXID_CLIENT_ID`. Not a
    /// secret (an OAuth client identifier), but paired with the secret below.
    pub nyxid_client_id: String,
    /// NyxID service-account client secret. Env: `FKST_NYXID_CLIENT_SECRET`.
    /// Held in a [`SecretString`]; never logged and redacted in `Debug`.
    pub nyxid_client_secret: SecretString,
}

// Manual `Debug` that renders the client secret as `<redacted>` (the codebase
// convention, mirroring `GithubAppConfig`) so an accidental `{:?}` on the config
// — or on anything embedding it — can never spill the credential into a log.
impl std::fmt::Debug for ChronoStorageConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ChronoStorageConfig")
            .field("base_url", &self.base_url)
            .field("bucket", &self.bucket)
            .field("nyxid_token_url", &self.nyxid_token_url)
            .field("nyxid_client_id", &self.nyxid_client_id)
            .field("nyxid_client_secret", &"<redacted>")
            .finish()
    }
}

/// Trim a raw env value; a blank string is treated as absent so a stray empty
/// ConfigMap value never masquerades as a real setting.
fn non_blank(value: Option<String>) -> Option<String> {
    value
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

impl ChronoStorageConfig {
    /// Deserialize the chrono-storage configuration from environment-style pairs.
    ///
    /// Returns `Ok(None)` when the feature is entirely unset (log streaming
    /// disabled), `Ok(Some(_))` when fully configured, and `Err` when only some
    /// of the five vars are set (fail closed, naming the missing ones).
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment; shares the caller's already-collected `vars` snapshot
    /// (see [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<Option<Self>, AppError> {
        let raw: StorageVars = envy::prefixed(STORAGE_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;

        // (env var name, normalized value) in a fixed order so the "missing"
        // report is deterministic.
        let fields = [
            ("FKST_STORAGE_BASE_URL", non_blank(raw.storage_base_url)),
            ("FKST_STORAGE_BUCKET", non_blank(raw.storage_bucket)),
            ("FKST_NYXID_TOKEN_URL", non_blank(raw.nyxid_token_url)),
            ("FKST_NYXID_CLIENT_ID", non_blank(raw.nyxid_client_id)),
            (
                "FKST_NYXID_CLIENT_SECRET",
                non_blank(raw.nyxid_client_secret),
            ),
        ];

        // Feature entirely unset => disabled. This is the common (default) path.
        if fields.iter().all(|(_, v)| v.is_none()) {
            return Ok(None);
        }

        // Partial config => fail closed, naming every missing var so the operator
        // fixes it in one pass rather than one redeploy at a time.
        let missing: Vec<&str> = fields
            .iter()
            .filter(|(_, v)| v.is_none())
            .map(|(k, _)| *k)
            .collect();
        if !missing.is_empty() {
            return Err(AppError::Config(format!(
                "chrono-storage is partially configured; set or unset all of its vars \
                 (missing: {})",
                missing.join(", ")
            )));
        }

        // Every field is Some here (the all-None and any-None cases returned
        // above), so the destructured unwraps cannot panic.
        let [base_url, bucket, nyxid_token_url, nyxid_client_id, nyxid_client_secret] =
            fields.map(|(_, v)| v.expect("all fields present past the partial-config guard"));
        Ok(Some(Self {
            base_url,
            bucket,
            nyxid_token_url,
            nyxid_client_id,
            nyxid_client_secret: SecretString::from(nyxid_client_secret),
        }))
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

    fn full() -> Vec<(String, String)> {
        vars(&[
            ("FKST_STORAGE_BASE_URL", "https://storage.example/proxy"),
            ("FKST_STORAGE_BUCKET", "fkst-logs"),
            ("FKST_NYXID_TOKEN_URL", "https://nyx.example/oauth/token"),
            ("FKST_NYXID_CLIENT_ID", "sa-client"),
            ("FKST_NYXID_CLIENT_SECRET", "sa-secret"),
        ])
    }

    #[test]
    fn unset_resolves_to_none() {
        // The default path: an environment with none of the vars keeps the
        // feature disabled and never fails the process.
        assert!(ChronoStorageConfig::from_vars(&vars(&[]))
            .expect("empty env is valid")
            .is_none());
    }

    #[test]
    fn fully_configured_resolves_to_some() {
        let config = ChronoStorageConfig::from_vars(&full())
            .expect("full config is valid")
            .expect("feature enabled");
        assert_eq!(config.base_url, "https://storage.example/proxy");
        assert_eq!(config.bucket, "fkst-logs");
        assert_eq!(config.nyxid_token_url, "https://nyx.example/oauth/token");
        assert_eq!(config.nyxid_client_id, "sa-client");
        assert_eq!(config.nyxid_client_secret.expose_secret(), "sa-secret");
    }

    #[test]
    fn partial_config_fails_closed_naming_the_missing_vars() {
        // Only the base URL + bucket set: the three NyxID creds are missing.
        let err = ChronoStorageConfig::from_vars(&vars(&[
            ("FKST_STORAGE_BASE_URL", "https://storage.example"),
            ("FKST_STORAGE_BUCKET", "fkst-logs"),
        ]))
        .expect_err("partial config must fail closed");
        assert!(matches!(err, AppError::Config(_)));
        let msg = err.to_string();
        assert!(msg.contains("FKST_NYXID_TOKEN_URL"), "{msg}");
        assert!(msg.contains("FKST_NYXID_CLIENT_ID"), "{msg}");
        assert!(msg.contains("FKST_NYXID_CLIENT_SECRET"), "{msg}");
        // The vars that WERE set must not be named as missing.
        assert!(!msg.contains("FKST_STORAGE_BASE_URL"), "{msg}");
    }

    #[test]
    fn blank_values_are_treated_as_unset() {
        // All-blank behaves exactly like all-unset (disabled), not as a partial
        // misconfiguration.
        assert!(ChronoStorageConfig::from_vars(&vars(&[
            ("FKST_STORAGE_BASE_URL", "   "),
            ("FKST_STORAGE_BUCKET", ""),
            ("FKST_NYXID_TOKEN_URL", "  "),
            ("FKST_NYXID_CLIENT_ID", ""),
            ("FKST_NYXID_CLIENT_SECRET", " "),
        ]))
        .expect("all-blank is valid")
        .is_none());
    }

    #[test]
    fn client_secret_is_redacted_in_debug_output() {
        let config = ChronoStorageConfig::from_vars(&full())
            .expect("valid")
            .expect("enabled");
        let debug = format!("{config:?}");
        assert!(
            !debug.contains("sa-secret"),
            "Debug leaked the client secret: {debug}"
        );
        assert!(debug.contains("<redacted>"), "{debug}");
        // Non-secret fields are still visible for diagnostics.
        assert!(debug.contains("fkst-logs"), "{debug}");
    }
}
