//! Typed runtime configuration loaded from environment variables.
//!
//! Two envy passes over the same snapshot of variables:
//! 1. the `FKST_HOSTED_*`-prefixed HTTP/server settings, and
//! 2. the unprefixed MongoDB settings (`MONGODB_URI` is required, fail-closed).

use std::fmt;

use secrecy::SecretString;
use serde::Deserialize;

use crate::auth::{AuthMode, NyxIdAuthSettings};
use crate::db::redact_mongodb_uri;
use crate::error::AppError;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

/// Prefix of the journaling variables (`FKST_JOURNAL_*` / `FKST_RAISED_*`).
const JOURNAL_ENV_PREFIX: &str = "FKST_";

/// Default values, shared by serde defaults and `Config::default`.
mod defaults {
    pub(super) fn port() -> u16 {
        8080
    }

    pub(super) fn bind_addr() -> String {
        "0.0.0.0".to_string()
    }

    pub(super) fn log_level() -> String {
        "info".to_string()
    }

    pub(super) fn request_timeout_secs() -> u64 {
        30
    }

    pub(super) fn mongodb_uri_placeholder() -> String {
        "mongodb://localhost:27017".to_string()
    }

    pub(super) fn mongodb_db() -> String {
        "fkst_hosted".to_string()
    }

    pub(super) fn mongodb_server_selection_timeout_ms() -> u64 {
        5000
    }

    pub(super) fn journal_flush_interval_ms() -> u64 {
        2000
    }

    pub(super) fn journal_flush_max_batch() -> usize {
        50
    }

    pub(super) fn journal_github_enabled() -> bool {
        true
    }

    pub(super) fn journal_issue_comments() -> bool {
        false
    }

    pub(super) fn journal_cas_max_retries() -> u32 {
        5
    }

    pub(super) fn journal_github_branch() -> String {
        "main".to_string()
    }

    pub(super) fn raised_identity_pointers() -> String {
        "/department,/source,/name,/corr".to_string()
    }

    pub(super) fn raised_max_line_bytes() -> usize {
        1_048_576
    }

    pub(super) fn auth_enabled() -> bool {
        // Default true: fail-closed at startup. Explicit `false` is a conscious
        // local-dev choice (the operator must set `FKST_AUTH_ENABLED=false` to
        // disable authentication).
        true
    }

    pub(super) fn auth_issuer() -> String {
        "nyxid".to_string()
    }

    pub(super) fn auth_jwks_cache_ttl_secs() -> u64 {
        300
    }
}

/// `FKST_HOSTED_*`-prefixed variables (HTTP/server settings).
#[derive(Debug, Deserialize)]
struct HttpVars {
    #[serde(default = "defaults::port")]
    port: u16,
    #[serde(default = "defaults::bind_addr")]
    bind_addr: String,
    #[serde(default = "defaults::log_level")]
    log_level: String,
    #[serde(default = "defaults::request_timeout_secs")]
    request_timeout_secs: u64,
}

/// Unprefixed MongoDB variables. `MONGODB_URI` has no default: a backend
/// without a store is misconfigured and must fail closed at startup.
/// `GITHUB_TOKEN` rides this unprefixed pass too (secret; optional —
/// without it GitHub journaling degrades to Mongo-only with a warn).
#[derive(Deserialize)]
struct MongoVars {
    mongodb_uri: String,
    #[serde(default = "defaults::mongodb_db")]
    mongodb_db: String,
    #[serde(default = "defaults::mongodb_server_selection_timeout_ms")]
    mongodb_server_selection_timeout_ms: u64,
    #[serde(default)]
    github_token: Option<String>,
}

/// `FKST_JOURNAL_*` / `FKST_RAISED_*` variables (journaling settings; envy
/// pass with the `FKST_` prefix).
#[derive(Debug, Deserialize)]
struct JournalVars {
    #[serde(default = "defaults::journal_flush_interval_ms")]
    journal_flush_interval_ms: u64,
    #[serde(default = "defaults::journal_flush_max_batch")]
    journal_flush_max_batch: usize,
    #[serde(default = "defaults::journal_github_enabled")]
    journal_github_enabled: bool,
    #[serde(default = "defaults::journal_issue_comments")]
    journal_issue_comments: bool,
    #[serde(default = "defaults::journal_cas_max_retries")]
    journal_cas_max_retries: u32,
    #[serde(default = "defaults::journal_github_branch")]
    journal_github_branch: String,
    #[serde(default)]
    journal_github_repo: Option<String>,
    #[serde(default = "defaults::raised_identity_pointers")]
    raised_identity_pointers: String,
    #[serde(default = "defaults::raised_max_line_bytes")]
    raised_max_line_bytes: usize,
}

/// `FKST_AUTH_*`-prefixed variables (authentication settings; envy pass with
/// the `FKST_` prefix).
#[derive(Debug, Deserialize)]
struct AuthVars {
    #[serde(default = "defaults::auth_enabled")]
    auth_enabled: bool,
    #[serde(default)]
    auth_nyxid_base_url: Option<String>,
    #[serde(default = "defaults::auth_issuer")]
    auth_issuer: String,
    #[serde(default)]
    auth_audience: Option<String>,
    #[serde(default = "defaults::auth_jwks_cache_ttl_secs")]
    auth_jwks_cache_ttl_secs: u64,
}

/// Runtime configuration assembled from both envy passes.
#[derive(Clone)]
pub struct Config {
    /// TCP port the HTTP server binds. Env: `FKST_HOSTED_PORT`. Default 8080.
    pub port: u16,
    /// Bind address. Env: `FKST_HOSTED_BIND_ADDR`. Default "0.0.0.0".
    pub bind_addr: String,
    /// tracing-subscriber `EnvFilter` directive. Env: `FKST_HOSTED_LOG_LEVEL`.
    /// Default "info".
    pub log_level: String,
    /// Per-request timeout in seconds for the tower-http `TimeoutLayer`.
    /// Env: `FKST_HOSTED_REQUEST_TIMEOUT_SECS`. Default 30.
    pub request_timeout_secs: u64,
    /// MongoDB connection string (may embed credentials — never log it in
    /// full). Env: `MONGODB_URI`. Required, fail-closed.
    pub mongodb_uri: String,
    /// Logical MongoDB database name. Env: `MONGODB_DB`.
    /// Default "fkst_hosted".
    pub mongodb_db: String,
    /// Driver server-selection timeout in milliseconds; bounds the startup
    /// ping and `/health` so an unreachable Mongo fails fast.
    /// Env: `MONGODB_SERVER_SELECTION_TIMEOUT_MS`. Default 5000.
    pub mongodb_server_selection_timeout_ms: u64,
    /// Max debounce (ms) before flushing buffered completions to GitHub.
    /// Env: `FKST_JOURNAL_FLUSH_INTERVAL_MS`. Default 2000.
    pub journal_flush_interval_ms: u64,
    /// Flush early when this many new completions are buffered.
    /// Env: `FKST_JOURNAL_FLUSH_MAX_BATCH`. Default 50.
    pub journal_flush_max_batch: usize,
    /// Master switch for GitHub journaling (Mongo journaling is always on).
    /// Env: `FKST_JOURNAL_GITHUB_ENABLED`. Default true.
    pub journal_github_enabled: bool,
    /// Enable the optional issue-comment mirroring (dormant by default).
    /// Env: `FKST_JOURNAL_ISSUE_COMMENTS`. Default false.
    pub journal_issue_comments: bool,
    /// Max optimistic-concurrency retries on the GitHub Contents write per
    /// flush. Env: `FKST_JOURNAL_CAS_MAX_RETRIES`. Default 5.
    pub journal_cas_max_retries: u32,
    /// Branch the journal file lives on.
    /// Env: `FKST_JOURNAL_GITHUB_BRANCH`. Default "main".
    pub journal_github_branch: String,
    /// `owner/name` of the journal repo; absent => GitHub journaling is
    /// disabled (Mongo-only, warn). Env: `FKST_JOURNAL_GITHUB_REPO`.
    pub journal_github_repo: Option<String>,
    /// Comma-separated JSON pointers forming raised-event identity.
    /// Env: `FKST_RAISED_IDENTITY_POINTERS`.
    /// Default "/department,/source,/name,/corr".
    pub raised_identity_pointers: String,
    /// Max stdout line length parsed; longer lines are truncated + counted
    /// as malformed. Env: `FKST_RAISED_MAX_LINE_BYTES`. Default 1048576.
    pub raised_max_line_bytes: usize,
    /// GitHub API token (SECRET — env/secret manager only; never logged,
    /// redacted from Debug). Env: `GITHUB_TOKEN`. Optional: absent =>
    /// GitHub journaling is disabled (Mongo-only, warn).
    pub github_token: Option<SecretString>,
    /// Authentication mode: disabled (local dev) or enabled with NyxID
    /// settings. Env: `FKST_AUTH_ENABLED` (default true = fail-closed).
    pub auth: AuthMode,
}

// Hand-written so the URI (which may embed credentials) is always printed
// through the redaction helper — `{:?}` can never leak a password.
impl fmt::Debug for Config {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Config")
            .field("port", &self.port)
            .field("bind_addr", &self.bind_addr)
            .field("log_level", &self.log_level)
            .field("request_timeout_secs", &self.request_timeout_secs)
            .field("mongodb_uri", &redact_mongodb_uri(&self.mongodb_uri))
            .field("mongodb_db", &self.mongodb_db)
            .field(
                "mongodb_server_selection_timeout_ms",
                &self.mongodb_server_selection_timeout_ms,
            )
            .field("journal_flush_interval_ms", &self.journal_flush_interval_ms)
            .field("journal_flush_max_batch", &self.journal_flush_max_batch)
            .field("journal_github_enabled", &self.journal_github_enabled)
            .field("journal_issue_comments", &self.journal_issue_comments)
            .field("journal_cas_max_retries", &self.journal_cas_max_retries)
            .field("journal_github_branch", &self.journal_github_branch)
            .field("journal_github_repo", &self.journal_github_repo)
            .field("raised_identity_pointers", &self.raised_identity_pointers)
            .field("raised_max_line_bytes", &self.raised_max_line_bytes)
            // The token value never reaches any Debug/log output.
            .field(
                "github_token",
                &self.github_token.as_ref().map(|_| "<redacted>"),
            )
            .field("auth", &self.auth)
            .finish()
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: defaults::port(),
            bind_addr: defaults::bind_addr(),
            log_level: defaults::log_level(),
            request_timeout_secs: defaults::request_timeout_secs(),
            mongodb_uri: defaults::mongodb_uri_placeholder(),
            mongodb_db: defaults::mongodb_db(),
            mongodb_server_selection_timeout_ms: defaults::mongodb_server_selection_timeout_ms(),
            journal_flush_interval_ms: defaults::journal_flush_interval_ms(),
            journal_flush_max_batch: defaults::journal_flush_max_batch(),
            journal_github_enabled: defaults::journal_github_enabled(),
            journal_issue_comments: defaults::journal_issue_comments(),
            journal_cas_max_retries: defaults::journal_cas_max_retries(),
            journal_github_branch: defaults::journal_github_branch(),
            journal_github_repo: None,
            raised_identity_pointers: defaults::raised_identity_pointers(),
            raised_max_line_bytes: defaults::raised_max_line_bytes(),
            github_token: None,
            auth: AuthMode::Disabled,
        }
    }
}

impl Config {
    /// Deserialize a `Config` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment. The pairs are collected once and fed to both
    /// envy passes (prefixed HTTP vars, unprefixed Mongo vars).
    pub fn from_vars(vars: impl IntoIterator<Item = (String, String)>) -> Result<Config, AppError> {
        let vars: Vec<(String, String)> = vars.into_iter().collect();

        let http: HttpVars = envy::prefixed(ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        // A zero timeout would make every request time out (408) — a total
        // outage from a one-character misconfiguration. Reject it loudly.
        if http.request_timeout_secs == 0 {
            return Err(AppError::Config(
                "FKST_HOSTED_REQUEST_TIMEOUT_SECS must be at least 1".to_string(),
            ));
        }

        let journal: JournalVars = envy::prefixed(JOURNAL_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        // A zero interval would force a GitHub round-trip per record and a
        // zero retry budget would fail every flush instantly — reject both
        // loudly, mirroring the timeout guards above.
        if journal.journal_flush_interval_ms == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_FLUSH_INTERVAL_MS must be at least 1".to_string(),
            ));
        }
        if journal.journal_flush_max_batch == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_FLUSH_MAX_BATCH must be at least 1".to_string(),
            ));
        }
        if journal.journal_cas_max_retries == 0 {
            return Err(AppError::Config(
                "FKST_JOURNAL_CAS_MAX_RETRIES must be at least 1".to_string(),
            ));
        }
        if journal.raised_max_line_bytes == 0 {
            return Err(AppError::Config(
                "FKST_RAISED_MAX_LINE_BYTES must be at least 1".to_string(),
            ));
        }

        let mongo: MongoVars = envy::from_iter(vars.clone()).map_err(|e| match e {
            // Name the exact env var so the fail-closed startup error is
            // actionable (envy reports the lowercase field name).
            envy::Error::MissingValue(field) => {
                AppError::Config(format!("{} must be set", field.to_uppercase()))
            }
            other => AppError::Config(other.to_string()),
        })?;
        // A zero selection timeout would make every Mongo operation fail
        // instantly (or fall back to a driver default) — reject it loudly,
        // mirroring the request-timeout guard above.
        if mongo.mongodb_server_selection_timeout_ms == 0 {
            return Err(AppError::Config(
                "MONGODB_SERVER_SELECTION_TIMEOUT_MS must be at least 1".to_string(),
            ));
        }

        // Authentication settings pass (FKST_AUTH_* with the FKST_ prefix).
        let auth: AuthVars = envy::prefixed(JOURNAL_ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;
        if auth.auth_jwks_cache_ttl_secs == 0 {
            return Err(AppError::Config(
                "FKST_AUTH_JWKS_CACHE_TTL_SECS must be at least 1".to_string(),
            ));
        }
        let auth_mode = if auth.auth_enabled {
            let base_url = match auth.auth_nyxid_base_url {
                Some(url) => url.trim_end_matches('/').to_string(),
                None => {
                    return Err(AppError::Config(
                        "FKST_AUTH_NYXID_BASE_URL must be set when FKST_AUTH_ENABLED=true"
                            .to_string(),
                    ));
                }
            };
            let audience = auth.auth_audience.unwrap_or_else(|| base_url.clone());
            AuthMode::Enabled(NyxIdAuthSettings {
                base_url,
                issuer: auth.auth_issuer,
                audience,
                jwks_cache_ttl: std::time::Duration::from_secs(auth.auth_jwks_cache_ttl_secs),
            })
        } else {
            AuthMode::Disabled
        };

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            mongodb_uri: mongo.mongodb_uri,
            mongodb_db: mongo.mongodb_db,
            mongodb_server_selection_timeout_ms: mongo.mongodb_server_selection_timeout_ms,
            journal_flush_interval_ms: journal.journal_flush_interval_ms,
            journal_flush_max_batch: journal.journal_flush_max_batch,
            journal_github_enabled: journal.journal_github_enabled,
            journal_issue_comments: journal.journal_issue_comments,
            journal_cas_max_retries: journal.journal_cas_max_retries,
            journal_github_branch: journal.journal_github_branch,
            journal_github_repo: journal.journal_github_repo,
            raised_identity_pointers: journal.raised_identity_pointers,
            raised_max_line_bytes: journal.raised_max_line_bytes,
            github_token: mongo.github_token.map(SecretString::from),
            auth: auth_mode,
        })
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<Config, AppError> {
        Self::from_vars(std::env::vars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal valid URI: every `from_vars` test input needs `MONGODB_URI`.
    const URI: (&str, &str) = ("MONGODB_URI", "mongodb://localhost:27017");

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_when_only_mongodb_uri_is_set() {
        let config = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        assert_eq!(config.port, 8080);
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.log_level, "info");
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.mongodb_uri, "mongodb://localhost:27017");
        assert_eq!(config.mongodb_db, "fkst_hosted");
        assert_eq!(config.mongodb_server_selection_timeout_ms, 5000);
        assert!(matches!(config.auth, AuthMode::Disabled));
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")]))
            .expect("defaults should deserialize");
        let from_default = Config::default();
        assert_eq!(from_default.port, from_env.port);
        assert_eq!(from_default.bind_addr, from_env.bind_addr);
        assert_eq!(from_default.log_level, from_env.log_level);
        assert_eq!(
            from_default.request_timeout_secs,
            from_env.request_timeout_secs
        );
        assert_eq!(from_default.mongodb_uri, from_env.mongodb_uri);
        assert_eq!(from_default.mongodb_db, from_env.mongodb_db);
        assert_eq!(
            from_default.mongodb_server_selection_timeout_ms,
            from_env.mongodb_server_selection_timeout_ms
        );
    }

    #[test]
    fn missing_mongodb_uri_is_a_config_error_naming_the_env_var() {
        let err = Config::from_vars(vars(&[])).expect_err("missing MONGODB_URI must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("MONGODB_URI"),
            "error must name MONGODB_URI, got: {err}"
        );
    }

    #[test]
    fn port_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_PORT", "9090"),
        ]))
        .unwrap();
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn bind_addr_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_BIND_ADDR", "127.0.0.1"),
        ]))
        .unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1");
    }

    #[test]
    fn log_level_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_LOG_LEVEL", "debug"),
        ]))
        .unwrap();
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn request_timeout_secs_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5"),
        ]))
        .unwrap();
        assert_eq!(config.request_timeout_secs, 5);
    }

    #[test]
    fn mongodb_uri_is_read_from_env() {
        let config = Config::from_vars(vars(&[
            ("MONGODB_URI", "mongodb://mongo.svc:27017"),
            ("FKST_AUTH_ENABLED", "false"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_uri, "mongodb://mongo.svc:27017");
    }

    #[test]
    fn mongodb_db_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_DB", "other_db"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_db, "other_db");
    }

    #[test]
    fn mongodb_server_selection_timeout_ms_is_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "750"),
        ]))
        .unwrap();
        assert_eq!(config.mongodb_server_selection_timeout_ms, 750);
    }

    #[test]
    fn non_numeric_mongodb_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "soon"),
        ]))
        .expect_err("non-numeric timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn zero_mongodb_selection_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "0"),
        ]))
        .expect_err("zero selection timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string()
                .contains("MONGODB_SERVER_SELECTION_TIMEOUT_MS"),
            "error must name the env var, got: {err}"
        );
    }

    #[test]
    fn zero_request_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0"),
        ]))
        .expect_err("zero timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains("FKST_HOSTED_REQUEST_TIMEOUT_SECS"));
    }

    #[test]
    fn non_numeric_port_is_a_config_error() {
        let err = Config::from_vars(vars(&[URI, ("FKST_HOSTED_PORT", "abc")]))
            .expect_err("non-numeric port must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn journal_defaults_apply_when_unset() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false")])).expect("defaults");
        assert_eq!(config.journal_flush_interval_ms, 2000);
        assert_eq!(config.journal_flush_max_batch, 50);
        assert!(config.journal_github_enabled);
        assert!(!config.journal_issue_comments);
        assert_eq!(config.journal_cas_max_retries, 5);
        assert_eq!(config.journal_github_branch, "main");
        assert_eq!(config.journal_github_repo, None);
        assert_eq!(
            config.raised_identity_pointers,
            "/department,/source,/name,/corr"
        );
        assert_eq!(config.raised_max_line_bytes, 1_048_576);
        assert!(config.github_token.is_none());
    }

    #[test]
    fn journal_vars_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("FKST_JOURNAL_FLUSH_INTERVAL_MS", "500"),
            ("FKST_JOURNAL_FLUSH_MAX_BATCH", "10"),
            ("FKST_JOURNAL_GITHUB_ENABLED", "false"),
            ("FKST_JOURNAL_ISSUE_COMMENTS", "true"),
            ("FKST_JOURNAL_CAS_MAX_RETRIES", "9"),
            ("FKST_JOURNAL_GITHUB_BRANCH", "journal"),
            ("FKST_JOURNAL_GITHUB_REPO", "acme/pkg-repo"),
            ("FKST_RAISED_IDENTITY_POINTERS", "/dept,/evt"),
            ("FKST_RAISED_MAX_LINE_BYTES", "2048"),
        ]))
        .expect("overrides");
        assert_eq!(config.journal_flush_interval_ms, 500);
        assert_eq!(config.journal_flush_max_batch, 10);
        assert!(!config.journal_github_enabled);
        assert!(config.journal_issue_comments);
        assert_eq!(config.journal_cas_max_retries, 9);
        assert_eq!(config.journal_github_branch, "journal");
        assert_eq!(config.journal_github_repo.as_deref(), Some("acme/pkg-repo"));
        assert_eq!(config.raised_identity_pointers, "/dept,/evt");
        assert_eq!(config.raised_max_line_bytes, 2048);
    }

    #[test]
    fn zero_journal_knobs_are_config_errors_naming_the_var() {
        for (var, value) in [
            ("FKST_JOURNAL_FLUSH_INTERVAL_MS", "0"),
            ("FKST_JOURNAL_FLUSH_MAX_BATCH", "0"),
            ("FKST_JOURNAL_CAS_MAX_RETRIES", "0"),
            ("FKST_RAISED_MAX_LINE_BYTES", "0"),
        ] {
            let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "false"), (var, value)]))
                .expect_err("zero must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }

    #[test]
    fn non_boolean_journal_switch_is_a_config_error() {
        let err = Config::from_vars(vars(&[URI, ("FKST_JOURNAL_GITHUB_ENABLED", "yep")]))
            .expect_err("non-boolean must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn github_token_is_read_and_never_appears_in_debug() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "false"),
            ("GITHUB_TOKEN", "ghp_sneaky_value"),
        ]))
        .expect("token config");
        assert!(config.github_token.is_some());
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("ghp_sneaky_value"), "token leaked");
        assert!(rendered.contains("<redacted>"));
    }

    #[test]
    fn debug_output_redacts_mongodb_credentials() {
        let config = Config {
            mongodb_uri: "mongodb://user:hunter2@mongo.svc:27017".to_string(),
            ..Config::default()
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("hunter2"), "debug leaked the password");
        assert!(rendered.contains("mongodb://<redacted>@mongo.svc:27017"));
    }

    // ---- auth configuration tests ----------------------------------------------

    #[test]
    fn auth_enabled_without_base_url_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[URI, ("FKST_AUTH_ENABLED", "true")]))
            .expect_err("enabled without base URL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_AUTH_NYXID_BASE_URL"),
            "error must name the variable, got: {err}"
        );
    }

    #[test]
    fn auth_enabled_with_base_url_builds_enabled_mode() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com/"),
        ]))
        .expect("enabled with base URL");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                // Trailing slash must be trimmed.
                assert_eq!(settings.base_url, "https://nyxid.example.com");
                assert_eq!(settings.issuer, "nyxid");
                // Audience defaults to base_url (after trim).
                assert_eq!(settings.audience, "https://nyxid.example.com");
                assert_eq!(settings.jwks_cache_ttl, std::time::Duration::from_secs(300));
            }
            AuthMode::Disabled => panic!("expected Enabled, got Disabled"),
        }
    }

    #[test]
    fn auth_issuer_and_audience_are_overridable() {
        let config = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com"),
            ("FKST_AUTH_ISSUER", "custom-issuer"),
            ("FKST_AUTH_AUDIENCE", "my-audience"),
            ("FKST_AUTH_JWKS_CACHE_TTL_SECS", "600"),
        ]))
        .expect("auth overrides");
        match config.auth {
            AuthMode::Enabled(ref settings) => {
                assert_eq!(settings.issuer, "custom-issuer");
                assert_eq!(settings.audience, "my-audience");
                assert_eq!(settings.jwks_cache_ttl, std::time::Duration::from_secs(600));
            }
            AuthMode::Disabled => panic!("expected Enabled"),
        }
    }

    #[test]
    fn zero_jwks_cache_ttl_is_a_config_error_naming_the_variable() {
        let err = Config::from_vars(vars(&[
            URI,
            ("FKST_AUTH_ENABLED", "true"),
            ("FKST_AUTH_NYXID_BASE_URL", "https://nyxid.example.com"),
            ("FKST_AUTH_JWKS_CACHE_TTL_SECS", "0"),
        ]))
        .expect_err("zero JWKS cache TTL must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(
            err.to_string().contains("FKST_AUTH_JWKS_CACHE_TTL_SECS"),
            "error must name the variable, got: {err}"
        );
    }
}
