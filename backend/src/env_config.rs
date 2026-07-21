//! Typed configuration for the named-environment / install-validation feature.
//!
//! A single envy pass over the `FKST_ENV_*` prefix, mirroring the defaults +
//! fail-closed style of [`crate::config`]. The knobs bound how many named
//! environments a user may hold, how large an install script may be, and the
//! deadline / concurrency / poll cadence of the isolated validation pod.
//!
//! Every knob is a hard bound whose zero value is a misconfiguration: a
//! zero cap, deadline, concurrency, or poll interval would either disable the
//! feature silently or spin a pod that can never make progress. We therefore
//! fail closed at startup, naming the offending variable, rather than defer the
//! surprise to the first request.

use base64::Engine;
use secrecy::SecretString;
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
#[derive(Deserialize)]
struct EnvVars {
    #[serde(default)]
    store_namespace: Option<String>,
    #[serde(default)]
    store_legacy_namespace: Option<String>,
    #[serde(default)]
    store_encryption_key: Option<String>,
    #[serde(default)]
    store_encryption_key_file: Option<String>,
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
    /// Namespace-independent Kubernetes Secret store. Unset keeps the legacy
    /// ConfigMap/Secret pair in `FKST_POD_NAMESPACE`.
    pub store_namespace: Option<String>,
    /// Optional namespace containing legacy pairs to migrate once at startup.
    pub store_legacy_namespace: Option<String>,
    /// Base64-encoded 32-byte AES-256-GCM key. Redacted by `SecretString` and
    /// decoded only while constructing the durable store.
    pub store_encryption_key: Option<SecretString>,
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
            store_namespace: None,
            store_legacy_namespace: None,
            store_encryption_key: None,
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

        let optional = |value: Option<String>| {
            value.and_then(|value| {
                let value = value.trim();
                (!value.is_empty()).then(|| value.to_string())
            })
        };
        let store_namespace = optional(env.store_namespace);
        let store_legacy_namespace = optional(env.store_legacy_namespace);
        let inline_key = env.store_encryption_key;
        let key_file = env.store_encryption_key_file;

        if inline_key.is_some() && key_file.is_some() {
            return Err(AppError::Config(
                "set exactly one of FKST_ENV_STORE_ENCRYPTION_KEY or \
                 FKST_ENV_STORE_ENCRYPTION_KEY_FILE"
                    .to_string(),
            ));
        }
        let key = match (inline_key, key_file) {
            (Some(key), None) => {
                let key = key.trim();
                if key.is_empty() {
                    return Err(AppError::Config(
                        "FKST_ENV_STORE_ENCRYPTION_KEY must not be blank".to_string(),
                    ));
                }
                Some(key.to_string())
            }
            (None, Some(path)) => {
                let path = path.trim();
                if path.is_empty() {
                    return Err(AppError::Config(
                        "FKST_ENV_STORE_ENCRYPTION_KEY_FILE must not be blank".to_string(),
                    ));
                }
                let value = std::fs::read_to_string(path).map_err(|error| {
                    AppError::Config(format!(
                        "FKST_ENV_STORE_ENCRYPTION_KEY_FILE could not be read: {error}"
                    ))
                })?;
                let value = value.trim();
                if value.is_empty() {
                    return Err(AppError::Config(
                        "FKST_ENV_STORE_ENCRYPTION_KEY_FILE is empty".to_string(),
                    ));
                }
                Some(value.to_string())
            }
            (None, None) => None,
            (Some(_), Some(_)) => unreachable!("conflicting key sources rejected above"),
        };

        if store_namespace.is_some() {
            let Some(key) = key.as_deref() else {
                return Err(AppError::Config(
                    "FKST_ENV_STORE_ENCRYPTION_KEY or \
                     FKST_ENV_STORE_ENCRYPTION_KEY_FILE must be set when \
                     FKST_ENV_STORE_NAMESPACE is configured"
                        .to_string(),
                ));
            };
            let decoded = base64::engine::general_purpose::STANDARD
                .decode(key)
                .map_err(|_| {
                    AppError::Config(
                        "FKST_ENV_STORE_ENCRYPTION_KEY must be base64-encoded".to_string(),
                    )
                })?;
            if decoded.len() != 32 {
                return Err(AppError::Config(
                    "FKST_ENV_STORE_ENCRYPTION_KEY must decode to exactly 32 bytes".to_string(),
                ));
            }
            if store_namespace == store_legacy_namespace {
                return Err(AppError::Config(
                    "FKST_ENV_STORE_LEGACY_NAMESPACE must differ from \
                     FKST_ENV_STORE_NAMESPACE"
                        .to_string(),
                ));
            }
        } else if key.is_some() || store_legacy_namespace.is_some() {
            return Err(AppError::Config(
                "FKST_ENV_STORE_NAMESPACE must be set when durable-store key or legacy \
                 migration configuration is present"
                    .to_string(),
            ));
        }

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
            store_namespace,
            store_legacy_namespace,
            store_encryption_key: key.map(SecretString::from),
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
#[path = "env_config_tests.rs"]
mod tests;
