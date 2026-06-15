//! GitHub App configuration: fail-closed env-var loading with PEM normalization.
//!
//! Env vars:
//!   - `FKST_GITHUB_APP_ID`               (required for enablement; unset = disabled)
//!   - `FKST_GITHUB_APP_PRIVATE_KEY_PEM`   (PEM content inline; `\n` escapes normalized)
//!   - `FKST_GITHUB_APP_PRIVATE_KEY_PATH`  (file path; mutually exclusive with _PEM)
//!   - `FKST_GITHUB_APP_SLUG`              (optional; used in install-hint URLs)
//!   - `FKST_GITHUB_APP_WEBHOOK_SECRET`    (optional-but-recommended, issue #108;
//!     when set, the webhook endpoint requires a valid `X-Hub-Signature-256`;
//!     when unset the webhook route is NOT mounted and resolution degrades to
//!     on-demand — a warning is logged at startup)
//!
//! Fail-closed rules:
//!   - ID set without exactly one key source => config error naming the vars.
//!   - Key source set without ID => config error naming the vars.
//!   - Both _PEM and _PATH set => config error.
//!   - PEM that does not parse as a valid RSA key => config error at startup.

use std::fmt;

use secrecy::SecretString;

use crate::error::AppError;

const ENV_APP_ID: &str = "FKST_GITHUB_APP_ID";
const ENV_KEY_PEM: &str = "FKST_GITHUB_APP_PRIVATE_KEY_PEM";
const ENV_KEY_PATH: &str = "FKST_GITHUB_APP_PRIVATE_KEY_PATH";
const ENV_SLUG: &str = "FKST_GITHUB_APP_SLUG";
const ENV_WEBHOOK_SECRET: &str = "FKST_GITHUB_APP_WEBHOOK_SECRET";

/// GitHub App configuration. The PEM is held in a [`SecretString`] and never
/// appears in `Debug` output.
pub struct GithubAppConfig {
    pub app_id: u64,
    pub private_key_pem: SecretString,
    pub app_slug: Option<String>,
    /// Webhook HMAC secret (issue #108). `None` when unset: the webhook route is
    /// not mounted and resolution degrades to on-demand. Held in a
    /// [`SecretString`] and never rendered in `Debug` or logged.
    pub webhook_secret: Option<SecretString>,
    /// API base (default `https://api.github.com`; tests inject for wiremock).
    pub api_base: String,
}

impl fmt::Debug for GithubAppConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GithubAppConfig")
            .field("app_id", &self.app_id)
            .field("private_key_pem", &"<redacted>")
            .field("app_slug", &self.app_slug)
            // Never render the secret itself — only whether one is configured.
            .field(
                "webhook_secret",
                &self.webhook_secret.as_ref().map(|_| "<redacted>"),
            )
            .field("api_base", &self.api_base)
            .finish()
    }
}

/// Normalize `\n` escape sequences in an inline PEM string to actual newlines.
/// Common when the PEM is stored in a single-line env var (e.g. k8s Secret
/// stringData).
fn normalize_pem_escapes(pem: &str) -> String {
    pem.replace("\\n", "\n")
}

/// Validate that the PEM bytes parse as a valid RSA private key (the format
/// `jsonwebtoken::EncodingKey::from_rsa_pem` expects).
fn validate_rsa_pem(pem: &str) -> Result<(), AppError> {
    jsonwebtoken::EncodingKey::from_rsa_pem(pem.as_bytes())
        .map(|_| ())
        .map_err(|e| {
            AppError::Config(format!(
                "{ENV_KEY_PEM} / {ENV_KEY_PATH}: PEM does not parse as a valid RSA private key: {e}"
            ))
        })
}

impl GithubAppConfig {
    /// Build from an arbitrary key-value source (env vars). Returns:
    /// - `Ok(None)` when `FKST_GITHUB_APP_ID` is unset (module disabled).
    /// - `Err(...)` on fail-closed misconfiguration.
    /// - `Ok(Some(config))` on success.
    pub fn from_vars(
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<Option<Self>, AppError> {
        let mut app_id: Option<u64> = None;
        let mut key_pem: Option<String> = None;
        let mut key_path: Option<String> = None;
        let mut slug: Option<String> = None;
        let mut webhook_secret: Option<String> = None;

        for (key, value) in vars {
            match key.as_str() {
                ENV_APP_ID => {
                    if value.is_empty() {
                        continue;
                    }
                    app_id = Some(value.parse::<u64>().map_err(|e| {
                        AppError::Config(format!(
                            "{ENV_APP_ID}: expected a u64, got \"{value}\": {e}"
                        ))
                    })?);
                }
                ENV_KEY_PEM if !value.is_empty() => {
                    key_pem = Some(value);
                }
                ENV_KEY_PATH if !value.is_empty() => {
                    key_path = Some(value);
                }
                ENV_SLUG if !value.is_empty() => {
                    slug = Some(value);
                }
                ENV_WEBHOOK_SECRET if !value.is_empty() => {
                    webhook_secret = Some(value);
                }
                _ => {}
            }
        }

        // No app id => module disabled.
        let Some(app_id) = app_id else {
            // If key vars are set without an ID, that is a misconfiguration.
            if key_pem.is_some() || key_path.is_some() {
                return Err(AppError::Config(format!(
                    "{ENV_KEY_PEM} / {ENV_KEY_PATH} set without {ENV_APP_ID}; set {ENV_APP_ID} or unset the key vars"
                )));
            }
            return Ok(None);
        };

        // ID set: exactly one key source required.
        match (&key_pem, &key_path) {
            (Some(_), Some(_)) => {
                return Err(AppError::Config(format!(
                    "both {ENV_KEY_PEM} and {ENV_KEY_PATH} set; provide exactly one"
                )));
            }
            (None, None) => {
                return Err(AppError::Config(format!(
                    "{ENV_APP_ID} set without {ENV_KEY_PEM} or {ENV_KEY_PATH}; provide exactly one key source"
                )));
            }
            _ => {}
        }

        let pem = if let Some(pem_raw) = key_pem {
            normalize_pem_escapes(&pem_raw)
        } else {
            let path = key_path.expect("at least one key source");
            std::fs::read_to_string(&path).map_err(|e| {
                AppError::Config(format!("{ENV_KEY_PATH}: failed to read {}: {e}", path))
            })?
        };

        validate_rsa_pem(&pem)?;

        Ok(Some(Self {
            app_id,
            private_key_pem: SecretString::from(pem),
            app_slug: slug,
            webhook_secret: webhook_secret.map(SecretString::from),
            api_base: "https://api.github.com".to_string(),
        }))
    }

    /// Load from the process environment.
    pub fn load_from_env() -> Result<Option<Self>, AppError> {
        Self::from_vars(std::env::vars())
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

    fn test_pem() -> String {
        use rand::rngs::OsRng;
        use rsa::pkcs8::{EncodePrivateKey, LineEnding};
        use rsa::RsaPrivateKey;
        let mut rng = OsRng;
        let private_key = RsaPrivateKey::new(&mut rng, 2048).expect("generate test RSA key");
        private_key
            .to_pkcs8_pem(LineEnding::LF)
            .expect("pkcs8 pem")
            .to_string()
    }

    #[test]
    fn absent_id_disables_module() {
        let result = GithubAppConfig::from_vars(vars(&[])).expect("ok");
        assert!(result.is_none(), "no env vars => disabled");
    }

    #[test]
    fn empty_id_disables_module() {
        let result = GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "")])).expect("ok");
        assert!(result.is_none());
    }

    #[test]
    fn pem_env_loads_and_normalizes_escapes() {
        let pem = test_pem();
        // Replace real newlines with literal \n escape sequences.
        let escaped = pem.replace('\n', "\\n");
        let cfg =
            GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "12345"), (ENV_KEY_PEM, &escaped)]))
                .expect("ok")
                .expect("some");
        assert_eq!(cfg.app_id, 12345);
        // The PEM inside the secret should have real newlines.
        assert_eq!(
            cfg.private_key_pem.expose_secret(),
            &pem,
            "PEM must be normalized"
        );
    }

    #[test]
    fn key_path_loads_file() {
        let pem = test_pem();
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("key.pem");
        std::fs::write(&path, &pem).expect("write pem");
        let cfg = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "42"),
            (ENV_KEY_PATH, path.to_str().unwrap()),
        ]))
        .expect("ok")
        .expect("some");
        assert_eq!(cfg.app_id, 42);
        assert_eq!(cfg.private_key_pem.expose_secret(), &pem);
    }

    #[test]
    fn id_without_key_is_config_error() {
        let err = GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "99")])).expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains(ENV_APP_ID), "names APP_ID: {msg}");
        assert!(
            msg.contains(ENV_KEY_PEM) || msg.contains(ENV_KEY_PATH),
            "names key vars: {msg}"
        );
    }

    #[test]
    fn key_without_id_is_config_error() {
        let err = GithubAppConfig::from_vars(vars(&[(ENV_KEY_PEM, "not-a-real-key")]))
            .expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains(ENV_APP_ID), "names APP_ID: {msg}");
    }

    #[test]
    fn both_pem_and_path_is_config_error() {
        let pem = test_pem();
        let err = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "1"),
            (ENV_KEY_PEM, &pem),
            (ENV_KEY_PATH, "/some/path"),
        ]))
        .expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains("both"), "must mention both vars: {msg}");
        assert!(msg.contains(ENV_KEY_PEM), "names PEM var: {msg}");
        assert!(msg.contains(ENV_KEY_PATH), "names PATH var: {msg}");
    }

    #[test]
    fn garbage_pem_fails_at_load() {
        let err = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "1"),
            (ENV_KEY_PEM, "totally not a PEM"),
        ]))
        .expect_err("must fail");
        let msg = format!("{err}");
        assert!(
            msg.contains("does not parse") || msg.contains("RSA"),
            "must mention PEM parse failure: {msg}"
        );
    }

    #[test]
    fn debug_never_contains_pem() {
        let pem = test_pem();
        let cfg = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "123"),
            (ENV_KEY_PEM, &pem),
            (ENV_SLUG, "fkst-hosted-test"),
        ]))
        .expect("ok")
        .expect("some");
        let debug = format!("{cfg:?}");
        assert!(!debug.contains(&pem), "Debug must not contain the PEM");
        assert!(debug.contains("<redacted>"), "Debug must show <redacted>");
        assert!(debug.contains("fkst-hosted-test"), "slug visible");
        assert!(debug.contains("123"), "app_id visible");
    }

    #[test]
    fn slug_is_optional() {
        let pem = test_pem();
        let cfg = GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "7"), (ENV_KEY_PEM, &pem)]))
            .expect("ok")
            .expect("some");
        assert!(cfg.app_slug.is_none());
    }

    #[test]
    fn webhook_secret_is_optional_and_loads_when_set() {
        let pem = test_pem();
        // Absent => None (webhook disabled, degrade to on-demand).
        let cfg = GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "7"), (ENV_KEY_PEM, &pem)]))
            .expect("ok")
            .expect("some");
        assert!(cfg.webhook_secret.is_none());

        // Present => Some, with the raw value recoverable for HMAC verification.
        let cfg = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "7"),
            (ENV_KEY_PEM, &pem),
            (ENV_WEBHOOK_SECRET, "whsec_supersecret"),
        ]))
        .expect("ok")
        .expect("some");
        assert_eq!(
            cfg.webhook_secret
                .as_ref()
                .map(|s| s.expose_secret().to_string()),
            Some("whsec_supersecret".to_string())
        );
    }

    #[test]
    fn debug_never_contains_webhook_secret() {
        let pem = test_pem();
        let cfg = GithubAppConfig::from_vars(vars(&[
            (ENV_APP_ID, "7"),
            (ENV_KEY_PEM, &pem),
            (ENV_WEBHOOK_SECRET, "whsec_leaky_value"),
        ]))
        .expect("ok")
        .expect("some");
        let debug = format!("{cfg:?}");
        assert!(
            !debug.contains("whsec_leaky_value"),
            "Debug leaked webhook secret: {debug}"
        );
        assert!(debug.contains("<redacted>"));
    }

    #[test]
    fn invalid_app_id_format_is_config_error() {
        let err =
            GithubAppConfig::from_vars(vars(&[(ENV_APP_ID, "not-a-number"), (ENV_KEY_PEM, "x")]))
                .expect_err("must fail");
        let msg = format!("{err}");
        assert!(msg.contains(ENV_APP_ID), "names APP_ID: {msg}");
        assert!(msg.contains("u64"), "mentions u64: {msg}");
    }
}
