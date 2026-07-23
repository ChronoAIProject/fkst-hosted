//! Shared validation for saved and disposable session environments.
//!
//! Both entry points accept the same environment-variable namespace and the
//! same configured size limits. Keeping those rules here prevents the direct
//! session-create API from drifting from the named-profile API.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use regex::Regex;

use crate::config::Config;
use crate::error::AppError;
use crate::reserved_env::{is_reserved_env_key, LLM_ENV_KEY};

fn env_key_regex() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new("^[A-Za-z_][A-Za-z0-9_]*$").expect("static env key regex"))
}

/// True when `key` is shaped like a process-environment variable name.
pub(crate) fn valid_env_key(key: &str) -> bool {
    env_key_regex().is_match(key)
}

/// Reject malformed or platform-owned environment-variable names.
pub(crate) fn validate_key(key: &str) -> Result<(), AppError> {
    if !valid_env_key(key) {
        return Err(AppError::Unprocessable(format!(
            "invalid env var name {key:?}: must match ^[A-Za-z_][A-Za-z0-9_]*$"
        )));
    }
    if is_reserved_env_key(key) || key == LLM_ENV_KEY {
        return Err(AppError::Unprocessable(format!(
            "env var name {key:?} is reserved and cannot be set"
        )));
    }
    Ok(())
}

/// Validate one variable or secret map against the shared key/value limits.
pub(crate) fn validate_entries(
    entries: &BTreeMap<String, String>,
    config: &Config,
) -> Result<(), AppError> {
    if entries.len() > config.vault_entries_per_scope_cap {
        return Err(AppError::Unprocessable(format!(
            "too many entries: {} exceeds the per-scope cap of {}",
            entries.len(),
            config.vault_entries_per_scope_cap
        )));
    }
    for (key, value) in entries {
        validate_key(key)?;
        if value.len() > config.vault_value_byte_cap {
            return Err(AppError::Unprocessable(format!(
                "value for {key:?} is {} bytes, exceeding the cap of {}",
                value.len(),
                config.vault_value_byte_cap
            )));
        }
    }
    Ok(())
}

/// Validate ordered install commands. Saved profiles require at least one
/// command because their PUT path validates an installation runtime. A
/// disposable environment may omit installs when it contains variables or
/// secrets, so callers choose `require_one` explicitly.
pub(crate) fn validate_install(
    install: &[String],
    config: &Config,
    require_one: bool,
) -> Result<(), AppError> {
    let count = install.len();
    if (require_one && count == 0) || count > config.env.install_max_commands {
        let minimum = usize::from(require_one);
        return Err(AppError::Unprocessable(format!(
            "install must list between {minimum} and {} commands (got {count})",
            config.env.install_max_commands
        )));
    }
    for (i, command) in install.iter().enumerate() {
        if command.trim().is_empty() {
            return Err(AppError::Unprocessable(format!(
                "install command {} must not be blank",
                i + 1
            )));
        }
        if command.len() > config.env.install_max_command_bytes {
            return Err(AppError::Unprocessable(format!(
                "install command {} is {} bytes, exceeding the cap of {}",
                i + 1,
                command.len(),
                config.env.install_max_command_bytes
            )));
        }
    }
    Ok(())
}
