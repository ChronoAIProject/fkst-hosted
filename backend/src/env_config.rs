//! Typed configuration for the named-environment / install-validation feature.
//!
//! A single envy pass over the `FKST_ENV_*` prefix, mirroring the defaults +
//! fail-closed style of [`crate::config`]. The knobs bound how many named
//! environments a user may hold, how large an install script may be, and the
//! deadline / concurrency / poll cadence of the isolated validation pod.
//!
//! Nothing here is wired to behaviour yet (this is the config surface only);
//! later issues read these values when they build the validation pipeline.
//!
//! Every knob is a hard bound whose zero value is a misconfiguration: a
//! zero cap, deadline, concurrency, or poll interval would either disable the
//! feature silently or spin a pod that can never make progress. We therefore
//! fail closed at startup, naming the offending variable, rather than defer the
//! surprise to the first request.

use serde::Deserialize;

use crate::error::AppError;

/// Prefix shared by every named-environment configuration variable.
const ENV_PREFIX: &str = "FKST_ENV_";

/// Default values, shared by serde defaults and [`EnvConfig::default`].
mod defaults {
    pub(super) fn max_per_user() -> usize {
        // Ceiling on named environments a single user may own. Generous but
        // bounded, so one user cannot exhaust the store.
        20
    }

    pub(super) fn install_max_commands() -> usize {
        // Upper bound on install-script commands validated per environment.
        50
    }

    pub(super) fn install_max_command_bytes() -> usize {
        // Upper bound on the byte length of a single install command.
        4096
    }

    pub(super) fn install_stderr_tail_bytes() -> usize {
        // How many trailing bytes of a failed command's stderr we surface.
        4096
    }

    pub(super) fn validate_deadline_secs() -> i64 {
        // Hard wall for the isolated validation pod. 5 minutes. `i64` matches
        // the k8s `activeDeadlineSeconds` field the pod path will feed it into.
        300
    }

    pub(super) fn validate_max_concurrent() -> usize {
        // How many validation pods may run at once across the control plane.
        4
    }

    pub(super) fn validate_poll_interval_secs() -> u64 {
        // How often the control plane polls a validation pod for completion.
        2
    }
}

/// `FKST_ENV_*`-prefixed variables (named-environment / install validation).
#[derive(Debug, Deserialize)]
struct EnvVars {
    #[serde(default = "defaults::max_per_user")]
    max_per_user: usize,
    #[serde(default = "defaults::install_max_commands")]
    install_max_commands: usize,
    #[serde(default = "defaults::install_max_command_bytes")]
    install_max_command_bytes: usize,
    #[serde(default = "defaults::install_stderr_tail_bytes")]
    install_stderr_tail_bytes: usize,
    #[serde(default = "defaults::validate_deadline_secs")]
    validate_deadline_secs: i64,
    #[serde(default = "defaults::validate_max_concurrent")]
    validate_max_concurrent: usize,
    #[serde(default = "defaults::validate_poll_interval_secs")]
    validate_poll_interval_secs: u64,
}

/// Named-environment / install-validation configuration.
#[derive(Clone, Debug)]
pub struct EnvConfig {
    /// Max named environments a single user may own. Env:
    /// `FKST_ENV_MAX_PER_USER`. Default 20; must be >= 1.
    pub max_per_user: usize,
    /// Max install-script commands validated per environment. Env:
    /// `FKST_ENV_INSTALL_MAX_COMMANDS`. Default 50; must be >= 1.
    pub install_max_commands: usize,
    /// Max byte length of a single install command. Env:
    /// `FKST_ENV_INSTALL_MAX_COMMAND_BYTES`. Default 4096; must be >= 1.
    pub install_max_command_bytes: usize,
    /// Trailing bytes of a failed command's stderr surfaced to the user. Env:
    /// `FKST_ENV_INSTALL_STDERR_TAIL_BYTES`. Default 4096; must be >= 1.
    pub install_stderr_tail_bytes: usize,
    /// Hard deadline for the isolated validation pod, seconds. Env:
    /// `FKST_ENV_VALIDATE_DEADLINE_SECS`. Default 300; must be >= 1.
    pub validate_deadline_secs: i64,
    /// Max concurrently-running validation pods. Env:
    /// `FKST_ENV_VALIDATE_MAX_CONCURRENT`. Default 4; must be >= 1.
    pub validate_max_concurrent: usize,
    /// Interval between validation-pod completion polls, seconds. Env:
    /// `FKST_ENV_VALIDATE_POLL_INTERVAL_SECS`. Default 2; must be >= 1.
    pub validate_poll_interval_secs: u64,
}

impl Default for EnvConfig {
    fn default() -> Self {
        Self {
            max_per_user: defaults::max_per_user(),
            install_max_commands: defaults::install_max_commands(),
            install_max_command_bytes: defaults::install_max_command_bytes(),
            install_stderr_tail_bytes: defaults::install_stderr_tail_bytes(),
            validate_deadline_secs: defaults::validate_deadline_secs(),
            validate_max_concurrent: defaults::validate_max_concurrent(),
            validate_poll_interval_secs: defaults::validate_poll_interval_secs(),
        }
    }
}

impl EnvConfig {
    /// Deserialize an `EnvConfig` from environment-style key/value pairs.
    ///
    /// Testable seam: unit tests feed explicit pairs instead of mutating the
    /// process environment. Shares the caller's already-collected `vars`
    /// snapshot (see [`crate::config::Config::from_vars`]).
    pub(crate) fn from_vars(vars: &[(String, String)]) -> Result<EnvConfig, AppError> {
        let env: EnvVars = envy::prefixed(ENV_PREFIX)
            .from_iter(vars.iter().cloned())
            .map_err(|e| AppError::Config(e.to_string()))?;

        // Fail closed on any zero bound: a zero cap silently disables a limit,
        // a zero deadline/poll interval yields a pod that can never make
        // progress, and a zero concurrency lets no validation run at all. Each
        // check names its variable so the operator can fix it immediately.
        if env.max_per_user == 0 {
            return Err(AppError::Config(
                "FKST_ENV_MAX_PER_USER must be at least 1".to_string(),
            ));
        }
        if env.install_max_commands == 0 {
            return Err(AppError::Config(
                "FKST_ENV_INSTALL_MAX_COMMANDS must be at least 1".to_string(),
            ));
        }
        if env.install_max_command_bytes == 0 {
            return Err(AppError::Config(
                "FKST_ENV_INSTALL_MAX_COMMAND_BYTES must be at least 1".to_string(),
            ));
        }
        if env.install_stderr_tail_bytes == 0 {
            return Err(AppError::Config(
                "FKST_ENV_INSTALL_STDERR_TAIL_BYTES must be at least 1".to_string(),
            ));
        }
        if env.validate_deadline_secs < 1 {
            return Err(AppError::Config(
                "FKST_ENV_VALIDATE_DEADLINE_SECS must be at least 1".to_string(),
            ));
        }
        if env.validate_max_concurrent == 0 {
            return Err(AppError::Config(
                "FKST_ENV_VALIDATE_MAX_CONCURRENT must be at least 1".to_string(),
            ));
        }
        if env.validate_poll_interval_secs == 0 {
            return Err(AppError::Config(
                "FKST_ENV_VALIDATE_POLL_INTERVAL_SECS must be at least 1".to_string(),
            ));
        }

        Ok(EnvConfig {
            max_per_user: env.max_per_user,
            install_max_commands: env.install_max_commands,
            install_max_command_bytes: env.install_max_command_bytes,
            install_stderr_tail_bytes: env.install_stderr_tail_bytes,
            validate_deadline_secs: env.validate_deadline_secs,
            validate_max_concurrent: env.validate_max_concurrent,
            validate_poll_interval_secs: env.validate_poll_interval_secs,
        })
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
    fn defaults_apply_when_nothing_is_set() {
        let config = EnvConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
        assert_eq!(config.max_per_user, 20);
        assert_eq!(config.install_max_commands, 50);
        assert_eq!(config.install_max_command_bytes, 4096);
        assert_eq!(config.install_stderr_tail_bytes, 4096);
        assert_eq!(config.validate_deadline_secs, 300);
        assert_eq!(config.validate_max_concurrent, 4);
        assert_eq!(config.validate_poll_interval_secs, 2);
    }

    #[test]
    fn default_impl_matches_env_defaults() {
        let from_env = EnvConfig::from_vars(&vars(&[])).expect("defaults should deserialize");
        let from_default = EnvConfig::default();
        assert_eq!(from_default.max_per_user, from_env.max_per_user);
        assert_eq!(
            from_default.install_max_commands,
            from_env.install_max_commands
        );
        assert_eq!(
            from_default.install_max_command_bytes,
            from_env.install_max_command_bytes
        );
        assert_eq!(
            from_default.install_stderr_tail_bytes,
            from_env.install_stderr_tail_bytes
        );
        assert_eq!(
            from_default.validate_deadline_secs,
            from_env.validate_deadline_secs
        );
        assert_eq!(
            from_default.validate_max_concurrent,
            from_env.validate_max_concurrent
        );
        assert_eq!(
            from_default.validate_poll_interval_secs,
            from_env.validate_poll_interval_secs
        );
    }

    #[test]
    fn every_knob_is_overridable() {
        let config = EnvConfig::from_vars(&vars(&[
            ("FKST_ENV_MAX_PER_USER", "5"),
            ("FKST_ENV_INSTALL_MAX_COMMANDS", "10"),
            ("FKST_ENV_INSTALL_MAX_COMMAND_BYTES", "256"),
            ("FKST_ENV_INSTALL_STDERR_TAIL_BYTES", "512"),
            ("FKST_ENV_VALIDATE_DEADLINE_SECS", "600"),
            ("FKST_ENV_VALIDATE_MAX_CONCURRENT", "8"),
            ("FKST_ENV_VALIDATE_POLL_INTERVAL_SECS", "3"),
        ]))
        .expect("overrides should deserialize");
        assert_eq!(config.max_per_user, 5);
        assert_eq!(config.install_max_commands, 10);
        assert_eq!(config.install_max_command_bytes, 256);
        assert_eq!(config.install_stderr_tail_bytes, 512);
        assert_eq!(config.validate_deadline_secs, 600);
        assert_eq!(config.validate_max_concurrent, 8);
        assert_eq!(config.validate_poll_interval_secs, 3);
    }

    #[test]
    fn zero_bounds_are_config_errors_naming_the_var() {
        // Every knob is a hard bound; a zero for any of them fails closed and
        // the error must name the offending variable.
        for var in [
            "FKST_ENV_MAX_PER_USER",
            "FKST_ENV_INSTALL_MAX_COMMANDS",
            "FKST_ENV_INSTALL_MAX_COMMAND_BYTES",
            "FKST_ENV_INSTALL_STDERR_TAIL_BYTES",
            "FKST_ENV_VALIDATE_DEADLINE_SECS",
            "FKST_ENV_VALIDATE_MAX_CONCURRENT",
            "FKST_ENV_VALIDATE_POLL_INTERVAL_SECS",
        ] {
            let err = EnvConfig::from_vars(&vars(&[(var, "0")])).expect_err("zero must fail");
            assert!(matches!(err, AppError::Config(_)));
            assert!(err.to_string().contains(var), "error must name {var}");
        }
    }
}
