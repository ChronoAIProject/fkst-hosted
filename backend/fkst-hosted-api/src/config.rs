//! Typed runtime configuration loaded from environment variables.
//!
//! Two envy passes over the same snapshot of variables:
//! 1. the `FKST_HOSTED_*`-prefixed HTTP/server settings, and
//! 2. the unprefixed MongoDB settings (`MONGODB_URI` is required, fail-closed).

use std::fmt;

use serde::Deserialize;

use crate::db::redact_mongodb_uri;
use crate::error::AppError;

/// Prefix shared by every HTTP/server configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_";

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
#[derive(Deserialize)]
struct MongoVars {
    mongodb_uri: String,
    #[serde(default = "defaults::mongodb_db")]
    mongodb_db: String,
    #[serde(default = "defaults::mongodb_server_selection_timeout_ms")]
    mongodb_server_selection_timeout_ms: u64,
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

        let mongo: MongoVars = envy::from_iter(vars).map_err(|e| match e {
            // Name the exact env var so the fail-closed startup error is
            // actionable (envy reports the lowercase field name).
            envy::Error::MissingValue(field) => {
                AppError::Config(format!("{} must be set", field.to_uppercase()))
            }
            other => AppError::Config(other.to_string()),
        })?;

        Ok(Config {
            port: http.port,
            bind_addr: http.bind_addr,
            log_level: http.log_level,
            request_timeout_secs: http.request_timeout_secs,
            mongodb_uri: mongo.mongodb_uri,
            mongodb_db: mongo.mongodb_db,
            mongodb_server_selection_timeout_ms: mongo.mongodb_server_selection_timeout_ms,
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
        let config = Config::from_vars(vars(&[URI])).expect("defaults should deserialize");
        assert_eq!(config.port, 8080);
        assert_eq!(config.bind_addr, "0.0.0.0");
        assert_eq!(config.log_level, "info");
        assert_eq!(config.request_timeout_secs, 30);
        assert_eq!(config.mongodb_uri, "mongodb://localhost:27017");
        assert_eq!(config.mongodb_db, "fkst_hosted");
        assert_eq!(config.mongodb_server_selection_timeout_ms, 5000);
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = Config::from_vars(vars(&[URI])).expect("defaults should deserialize");
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
        let config = Config::from_vars(vars(&[URI, ("FKST_HOSTED_PORT", "9090")])).unwrap();
        assert_eq!(config.port, 9090);
    }

    #[test]
    fn bind_addr_is_overridable() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_HOSTED_BIND_ADDR", "127.0.0.1")])).unwrap();
        assert_eq!(config.bind_addr, "127.0.0.1");
    }

    #[test]
    fn log_level_is_overridable() {
        let config = Config::from_vars(vars(&[URI, ("FKST_HOSTED_LOG_LEVEL", "debug")])).unwrap();
        assert_eq!(config.log_level, "debug");
    }

    #[test]
    fn request_timeout_secs_is_overridable() {
        let config =
            Config::from_vars(vars(&[URI, ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "5")])).unwrap();
        assert_eq!(config.request_timeout_secs, 5);
    }

    #[test]
    fn mongodb_uri_is_read_from_env() {
        let config =
            Config::from_vars(vars(&[("MONGODB_URI", "mongodb://mongo.svc:27017")])).unwrap();
        assert_eq!(config.mongodb_uri, "mongodb://mongo.svc:27017");
    }

    #[test]
    fn mongodb_db_is_overridable() {
        let config = Config::from_vars(vars(&[URI, ("MONGODB_DB", "other_db")])).unwrap();
        assert_eq!(config.mongodb_db, "other_db");
    }

    #[test]
    fn mongodb_server_selection_timeout_ms_is_overridable() {
        let config =
            Config::from_vars(vars(&[URI, ("MONGODB_SERVER_SELECTION_TIMEOUT_MS", "750")]))
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
    fn zero_request_timeout_is_a_config_error() {
        let err = Config::from_vars(vars(&[URI, ("FKST_HOSTED_REQUEST_TIMEOUT_SECS", "0")]))
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
    fn debug_output_redacts_mongodb_credentials() {
        let config = Config {
            mongodb_uri: "mongodb://user:hunter2@mongo.svc:27017".to_string(),
            ..Config::default()
        };
        let rendered = format!("{config:?}");
        assert!(!rendered.contains("hunter2"), "debug leaked the password");
        assert!(rendered.contains("mongodb://<redacted>@mongo.svc:27017"));
    }
}
