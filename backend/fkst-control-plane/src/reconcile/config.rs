//! Reconcile-sweep configuration (env prefix shares the established
//! `FKST_HOSTED_` namespace, loaded through the same injectable key/value
//! seam as the sibling configs and validated fail-closed at startup).
//!
//! Two knobs only — the reduced-scope orphan sweep needs nothing more:
//! - `FKST_HOSTED_RECONCILE_MIN_AGE_SECS` (default 300 = 5 min): a safety
//!   threshold so a freshly-created in-flight session dir is never swept even
//!   if it momentarily looks orphaned.
//! - `FKST_HOSTED_RECONCILE_DRY_RUN` (default false): when set, the sweep
//!   records what it WOULD delete but removes nothing (audit mode).

use std::time::Duration;

use crate::error::AppError;

/// Minimum age before an orphan dir is eligible for sweeping. Must be `> 0`.
pub const ENV_MIN_AGE_SECS: &str = "FKST_HOSTED_RECONCILE_MIN_AGE_SECS";
/// Dry-run switch: record-only, delete nothing.
pub const ENV_DRY_RUN: &str = "FKST_HOSTED_RECONCILE_DRY_RUN";

const DEFAULT_MIN_AGE_SECS: u64 = 300;

/// Sanity ceiling (24 hours) shared with the sibling duration knobs.
const MAX_SECS: u64 = 86_400;

/// Configuration for the boot-time orphan temp-dir sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReconcileConfig {
    /// An orphan dir is only swept once its mtime is at least this old, so an
    /// in-flight session's just-created dir is never removed mid-flight.
    pub min_age: Duration,
    /// Record-only mode: the report lists would-be sweeps but nothing is
    /// removed from disk.
    pub dry_run: bool,
}

impl Default for ReconcileConfig {
    fn default() -> Self {
        Self {
            min_age: Duration::from_secs(DEFAULT_MIN_AGE_SECS),
            dry_run: false,
        }
    }
}

impl ReconcileConfig {
    /// Build (and fail-closed validate) a `ReconcileConfig` from
    /// environment-style key/value pairs. Mirrors the sibling configs'
    /// `from_vars` seam so unit tests feed explicit pairs instead of mutating
    /// the process environment.
    pub fn from_vars(
        vars: impl IntoIterator<Item = (String, String)>,
    ) -> Result<ReconcileConfig, AppError> {
        let mut min_age_raw = None;
        let mut dry_run_raw = None;
        for (key, value) in vars {
            match key.as_str() {
                ENV_MIN_AGE_SECS => min_age_raw = Some(value),
                ENV_DRY_RUN => dry_run_raw = Some(value),
                _ => {}
            }
        }

        let min_age_secs = parse_secs(ENV_MIN_AGE_SECS, min_age_raw, DEFAULT_MIN_AGE_SECS)?;
        // A zero min-age would make the sweep eligible to delete a dir created
        // the same instant a session started — exactly the in-flight dir the
        // threshold exists to protect. Reject loudly.
        if min_age_secs == 0 {
            return Err(AppError::Config(format!(
                "{ENV_MIN_AGE_SECS} must be at least 1 second"
            )));
        }

        let dry_run = parse_bool(ENV_DRY_RUN, dry_run_raw)?;

        let config = ReconcileConfig {
            min_age: Duration::from_secs(min_age_secs),
            dry_run,
        };
        tracing::info!(
            min_age_secs = config.min_age.as_secs(),
            dry_run = config.dry_run,
            "reconcile configured"
        );
        Ok(config)
    }

    /// Load the configuration from the process environment.
    pub fn load_from_env() -> Result<ReconcileConfig, AppError> {
        Self::from_vars(std::env::vars())
    }
}

/// Parse a non-negative whole-seconds value with a default, rejecting
/// non-numeric and absurdly large (> 24h) values with an error naming the
/// variable. Zero-rejection is relationship-specific and lives in `from_vars`.
fn parse_secs(name: &'static str, raw: Option<String>, default: u64) -> Result<u64, AppError> {
    let Some(raw) = raw else {
        return Ok(default);
    };
    let value: u64 = raw.trim().parse().map_err(|_| {
        AppError::Config(format!(
            "{name} must be a non-negative whole number, got {raw:?}"
        ))
    })?;
    if value > MAX_SECS {
        return Err(AppError::Config(format!(
            "{name} must be at most {MAX_SECS}, got {value}"
        )));
    }
    Ok(value)
}

/// Parse a loose boolean (`true`/`false`/`1`/`0`, case-insensitive). Absent =>
/// `false`. Anything else is a config error naming the variable.
fn parse_bool(name: &'static str, raw: Option<String>) -> Result<bool, AppError> {
    let Some(raw) = raw else {
        return Ok(false);
    };
    match raw.trim().to_ascii_lowercase().as_str() {
        "true" | "1" | "yes" | "on" => Ok(true),
        "false" | "0" | "no" | "off" | "" => Ok(false),
        other => Err(AppError::Config(format!(
            "{name} must be a boolean (true/false), got {other:?}"
        ))),
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
    fn defaults_apply_with_no_vars() {
        let config = ReconcileConfig::from_vars(vars(&[])).expect("defaults load");
        assert_eq!(config.min_age, Duration::from_secs(300));
        assert!(!config.dry_run);
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = ReconcileConfig::from_vars(vars(&[])).expect("defaults load");
        assert_eq!(ReconcileConfig::default(), from_env);
    }

    #[test]
    fn min_age_is_overridable() {
        let config =
            ReconcileConfig::from_vars(vars(&[(ENV_MIN_AGE_SECS, "60")])).expect("config loads");
        assert_eq!(config.min_age, Duration::from_secs(60));
    }

    #[test]
    fn dry_run_accepts_loose_truthy_values() {
        for raw in ["true", "1", "yes", "on", "TRUE", "On"] {
            let config = ReconcileConfig::from_vars(vars(&[(ENV_DRY_RUN, raw)]))
                .unwrap_or_else(|e| panic!("{raw:?} must parse: {e}"));
            assert!(config.dry_run, "{raw:?} must be truthy");
        }
        for raw in ["false", "0", "no", "off", ""] {
            let config = ReconcileConfig::from_vars(vars(&[(ENV_DRY_RUN, raw)]))
                .unwrap_or_else(|e| panic!("{raw:?} must parse: {e}"));
            assert!(!config.dry_run, "{raw:?} must be falsy");
        }
    }

    #[test]
    fn zero_min_age_is_a_config_error_naming_the_var() {
        let err = ReconcileConfig::from_vars(vars(&[(ENV_MIN_AGE_SECS, "0")]))
            .expect_err("zero min-age must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(ENV_MIN_AGE_SECS), "got: {err}");
    }

    #[test]
    fn non_numeric_min_age_is_a_config_error() {
        let err = ReconcileConfig::from_vars(vars(&[(ENV_MIN_AGE_SECS, "soon")]))
            .expect_err("non-numeric must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(ENV_MIN_AGE_SECS));
    }

    #[test]
    fn over_ceiling_min_age_is_a_config_error() {
        let err = ReconcileConfig::from_vars(vars(&[(ENV_MIN_AGE_SECS, "86401")]))
            .expect_err("over ceiling must fail");
        assert!(matches!(err, AppError::Config(_)));
    }

    #[test]
    fn bad_dry_run_value_is_a_config_error() {
        let err = ReconcileConfig::from_vars(vars(&[(ENV_DRY_RUN, "maybe")]))
            .expect_err("non-boolean must fail");
        assert!(matches!(err, AppError::Config(_)));
        assert!(err.to_string().contains(ENV_DRY_RUN));
    }
}
