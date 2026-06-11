//! Engine runner configuration loaded from `FKST_HOSTED_ENGINE_*` variables.
//!
//! Mirrors the [`crate::config`] pattern: a single envy pass over an explicit
//! key/value snapshot (`from_vars` is the testable seam), serde defaults for
//! every key, and loud rejection of dangerous zero timeouts.

use std::path::PathBuf;

use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared by every engine-runner configuration environment variable.
const ENV_PREFIX: &str = "FKST_HOSTED_ENGINE_";

/// Default values, shared by serde defaults and `EngineConfig::default`.
mod defaults {
    use std::path::PathBuf;

    pub(super) fn framework_bin() -> PathBuf {
        PathBuf::from("/usr/local/bin/fkst-framework")
    }

    pub(super) fn temp_root() -> PathBuf {
        std::env::temp_dir()
    }

    pub(super) fn candidate_prefix() -> String {
        "candidate/".to_string()
    }

    pub(super) fn candidate_from_sep() -> String {
        "::".to_string()
    }

    pub(super) fn stop_grace_secs() -> u64 {
        10
    }

    pub(super) fn conformance_timeout_secs() -> u64 {
        60
    }

    pub(super) fn ready_timeout_secs() -> u64 {
        30
    }

    pub(super) fn error_capture_bytes() -> usize {
        8192
    }

    pub(super) fn log_tail_lines() -> usize {
        200
    }
}

/// Engine-runner configuration (env prefix `FKST_HOSTED_ENGINE_`).
///
/// Every key has a default, so the runner boots with zero engine config.
/// The two candidate HostFacts (`candidate_prefix`, `candidate_from_sep`) are
/// always materialized into the package's `fkst.env` — they are not required
/// by the engine to *start* (spike #17, Q1) but packages that call the
/// candidate-git SDK resolve them lazily at runtime.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineConfig {
    /// Path to the bundled engine binary.
    /// Env: `FKST_HOSTED_ENGINE_FRAMEWORK_BIN`.
    /// Default `/usr/local/bin/fkst-framework`.
    #[serde(default = "defaults::framework_bin")]
    pub framework_bin: PathBuf,
    /// Parent directory for the ephemeral `fkst-pkg-*` / `fkst-rt-*` dirs.
    /// Env: `FKST_HOSTED_ENGINE_TEMP_ROOT`. Default: the OS temp dir.
    #[serde(default = "defaults::temp_root")]
    pub temp_root: PathBuf,
    /// `FKST_CANDIDATE_PREFIX` HostFact value written to `fkst.env`.
    /// Env: `FKST_HOSTED_ENGINE_CANDIDATE_PREFIX`. Default `candidate/`.
    #[serde(default = "defaults::candidate_prefix")]
    pub candidate_prefix: String,
    /// `FKST_CANDIDATE_FROM_SEP` HostFact value written to `fkst.env`.
    /// Env: `FKST_HOSTED_ENGINE_CANDIDATE_FROM_SEP`. Default `::`.
    #[serde(default = "defaults::candidate_from_sep")]
    pub candidate_from_sep: String,
    /// SIGTERM -> SIGKILL grace window in seconds when stopping a session.
    /// Env: `FKST_HOSTED_ENGINE_STOP_GRACE_SECS`. Default 10, must be >= 1.
    #[serde(default = "defaults::stop_grace_secs")]
    pub stop_grace_secs: u64,
    /// Wall-clock cap on the `conformance` pre-flight in seconds.
    /// Env: `FKST_HOSTED_ENGINE_CONFORMANCE_TIMEOUT_SECS`. Default 60,
    /// must be >= 1.
    #[serde(default = "defaults::conformance_timeout_secs")]
    pub conformance_timeout_secs: u64,
    /// Wall-clock cap on the supervise ready-wait in seconds.
    /// Env: `FKST_HOSTED_ENGINE_READY_TIMEOUT_SECS`. Default 30, must be >= 1.
    #[serde(default = "defaults::ready_timeout_secs")]
    pub ready_timeout_secs: u64,
    /// Max bytes of captured stderr surfaced in runner errors (truncated
    /// lossily at a char boundary).
    /// Env: `FKST_HOSTED_ENGINE_ERROR_CAPTURE_BYTES`. Default 8192.
    #[serde(default = "defaults::error_capture_bytes")]
    pub error_capture_bytes: usize,
    /// Max lines tailed from the engine's `framework-child` logs (also byte
    /// capped). Env: `FKST_HOSTED_ENGINE_LOG_TAIL_LINES`. Default 200.
    #[serde(default = "defaults::log_tail_lines")]
    pub log_tail_lines: usize,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            framework_bin: defaults::framework_bin(),
            temp_root: defaults::temp_root(),
            candidate_prefix: defaults::candidate_prefix(),
            candidate_from_sep: defaults::candidate_from_sep(),
            stop_grace_secs: defaults::stop_grace_secs(),
            conformance_timeout_secs: defaults::conformance_timeout_secs(),
            ready_timeout_secs: defaults::ready_timeout_secs(),
            error_capture_bytes: defaults::error_capture_bytes(),
            log_tail_lines: defaults::log_tail_lines(),
        }
    }
}

impl EngineConfig {
    /// Deserialize an `EngineConfig` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment (mirrors [`crate::config::Config::from_vars`]).
    pub fn from_vars(
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<EngineConfig, AppError> {
        let config: EngineConfig = envy::prefixed(ENV_PREFIX)
            .from_iter(vars)
            .map_err(|e| AppError::Config(e.to_string()))?;

        // A zero grace would jump straight to SIGKILL and a zero timeout
        // would fail every conformance run / ready-wait instantly — total
        // breakage from a one-character misconfiguration. Reject loudly,
        // naming the exact env var so the startup error is actionable.
        for (value, var) in [
            (config.stop_grace_secs, "FKST_HOSTED_ENGINE_STOP_GRACE_SECS"),
            (
                config.conformance_timeout_secs,
                "FKST_HOSTED_ENGINE_CONFORMANCE_TIMEOUT_SECS",
            ),
            (
                config.ready_timeout_secs,
                "FKST_HOSTED_ENGINE_READY_TIMEOUT_SECS",
            ),
        ] {
            if value == 0 {
                return Err(AppError::Config(format!("{var} must be at least 1")));
            }
        }

        Ok(config)
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<EngineConfig, AppError> {
        Self::from_vars(std::env::vars())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vars(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn defaults_apply_with_no_engine_vars() {
        let config = EngineConfig::from_vars(vars(&[])).expect("defaults should deserialize");
        assert_eq!(
            config.framework_bin,
            PathBuf::from("/usr/local/bin/fkst-framework")
        );
        assert_eq!(config.temp_root, std::env::temp_dir());
        assert_eq!(config.candidate_prefix, "candidate/");
        assert_eq!(config.candidate_from_sep, "::");
        assert_eq!(config.stop_grace_secs, 10);
        assert_eq!(config.conformance_timeout_secs, 60);
        assert_eq!(config.ready_timeout_secs, 30);
        assert_eq!(config.error_capture_bytes, 8192);
        assert_eq!(config.log_tail_lines, 200);
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = EngineConfig::from_vars(vars(&[])).expect("defaults should deserialize");
        let from_default = EngineConfig::default();
        assert_eq!(from_default.framework_bin, from_env.framework_bin);
        assert_eq!(from_default.temp_root, from_env.temp_root);
        assert_eq!(from_default.candidate_prefix, from_env.candidate_prefix);
        assert_eq!(from_default.candidate_from_sep, from_env.candidate_from_sep);
        assert_eq!(from_default.stop_grace_secs, from_env.stop_grace_secs);
        assert_eq!(
            from_default.conformance_timeout_secs,
            from_env.conformance_timeout_secs
        );
        assert_eq!(from_default.ready_timeout_secs, from_env.ready_timeout_secs);
        assert_eq!(
            from_default.error_capture_bytes,
            from_env.error_capture_bytes
        );
        assert_eq!(from_default.log_tail_lines, from_env.log_tail_lines);
    }

    #[test]
    fn framework_bin_is_overridable() {
        let config = EngineConfig::from_vars(vars(&[(
            "FKST_HOSTED_ENGINE_FRAMEWORK_BIN",
            "/opt/engine/fkst-framework",
        )]))
        .unwrap();
        assert_eq!(
            config.framework_bin,
            PathBuf::from("/opt/engine/fkst-framework")
        );
    }

    #[test]
    fn temp_root_is_overridable() {
        let config =
            EngineConfig::from_vars(vars(&[("FKST_HOSTED_ENGINE_TEMP_ROOT", "/var/scratch")]))
                .unwrap();
        assert_eq!(config.temp_root, PathBuf::from("/var/scratch"));
    }

    #[test]
    fn candidate_host_facts_are_overridable() {
        let config = EngineConfig::from_vars(vars(&[
            ("FKST_HOSTED_ENGINE_CANDIDATE_PREFIX", "cand/"),
            ("FKST_HOSTED_ENGINE_CANDIDATE_FROM_SEP", "--"),
        ]))
        .unwrap();
        assert_eq!(config.candidate_prefix, "cand/");
        assert_eq!(config.candidate_from_sep, "--");
    }

    #[test]
    fn timeouts_and_caps_are_overridable() {
        let config = EngineConfig::from_vars(vars(&[
            ("FKST_HOSTED_ENGINE_STOP_GRACE_SECS", "3"),
            ("FKST_HOSTED_ENGINE_CONFORMANCE_TIMEOUT_SECS", "5"),
            ("FKST_HOSTED_ENGINE_READY_TIMEOUT_SECS", "7"),
            ("FKST_HOSTED_ENGINE_ERROR_CAPTURE_BYTES", "1024"),
            ("FKST_HOSTED_ENGINE_LOG_TAIL_LINES", "50"),
        ]))
        .unwrap();
        assert_eq!(config.stop_grace_secs, 3);
        assert_eq!(config.conformance_timeout_secs, 5);
        assert_eq!(config.ready_timeout_secs, 7);
        assert_eq!(config.error_capture_bytes, 1024);
        assert_eq!(config.log_tail_lines, 50);
    }

    #[test]
    fn zero_stop_grace_is_a_config_error_naming_the_env_var() {
        let err = EngineConfig::from_vars(vars(&[("FKST_HOSTED_ENGINE_STOP_GRACE_SECS", "0")]))
            .expect_err("zero grace must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err
            .to_string()
            .contains("FKST_HOSTED_ENGINE_STOP_GRACE_SECS"));
    }

    #[test]
    fn zero_conformance_timeout_is_a_config_error_naming_the_env_var() {
        let err = EngineConfig::from_vars(vars(&[(
            "FKST_HOSTED_ENGINE_CONFORMANCE_TIMEOUT_SECS",
            "0",
        )]))
        .expect_err("zero timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err
            .to_string()
            .contains("FKST_HOSTED_ENGINE_CONFORMANCE_TIMEOUT_SECS"));
    }

    #[test]
    fn zero_ready_timeout_is_a_config_error_naming_the_env_var() {
        let err = EngineConfig::from_vars(vars(&[("FKST_HOSTED_ENGINE_READY_TIMEOUT_SECS", "0")]))
            .expect_err("zero timeout must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err
            .to_string()
            .contains("FKST_HOSTED_ENGINE_READY_TIMEOUT_SECS"));
    }

    #[test]
    fn non_numeric_timeout_is_a_config_error() {
        let err = EngineConfig::from_vars(vars(&[("FKST_HOSTED_ENGINE_STOP_GRACE_SECS", "soon")]))
            .expect_err("non-numeric grace must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn unrelated_prefixed_vars_do_not_break_loading() {
        // The runner shares the process env with the HTTP config; foreign
        // FKST_HOSTED_* keys (different prefix tail) must not interfere.
        let config = EngineConfig::from_vars(vars(&[
            ("FKST_HOSTED_PORT", "9090"),
            ("MONGODB_URI", "mongodb://localhost:27017"),
        ]))
        .expect("foreign keys must be ignored");
        assert_eq!(config.stop_grace_secs, 10);
    }
}
