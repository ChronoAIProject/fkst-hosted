//! Reserved-env "keep-module": the symbols the platform needs to decide which
//! env keys a user may never set, and which env key the engine reads the LLM
//! credential from.
//!
//! [`is_reserved_env_key`] originally lived in `engine/config.rs` and
//! [`LLM_ENV_KEY`] in `sessions/codex_provider/mod.rs` — both of which the Model B
//! migration deletes. They (and the reserved-key TABLES they read —
//! [`RESERVED_ENV_PREFIX`], [`RESERVED_ENV_NAME_PREFIXES`], [`RESERVED_ENV_KEYS`],
//! [`ENGINE_ENV_ALLOWLIST`]) were relocated here (issue #359 §7/§9) so they stay
//! alive and callable after `engine/` is gone. This module is now the single,
//! self-contained source of truth for the reserved-env policy; behaviour is
//! unchanged from Model A (a verbatim relocation, not a rewrite).

/// Host environment variables copied from the fkst-hosted parent process into
/// every engine child *if present in the parent* (issue #101). After
/// `Command::env_clear()` the child inherits nothing; these are the only host
/// vars allowed back in, because `codex` and the toolchain genuinely need them
/// (`HOME`/`CODEX_HOME` for codex config discovery, `PATH` to find binaries,
/// the locale/TZ/TLS vars for correct runtime behaviour). Anything else in the
/// pod environment (including any ambient secret) is deliberately dropped so a
/// secret in the pod env can never leak into a session.
pub const ENGINE_ENV_ALLOWLIST: &[&str] = &[
    "PATH",
    "HOME",
    "CODEX_HOME",
    "LANG",
    "LC_ALL",
    "TMPDIR",
    "TZ",
    "SSL_CERT_FILE",
    "SSL_CERT_DIR",
];

/// Exact keys that a user-supplied `env_profile` may never set because the
/// platform owns them (the goal-session GitHub wiring + #107 git credential
/// delivery). Combined with the [`RESERVED_ENV_PREFIX`],
/// [`RESERVED_ENV_NAME_PREFIXES`] and [`ENGINE_ENV_ALLOWLIST`] in
/// [`is_reserved_env_key`].
pub const RESERVED_ENV_KEYS: &[&str] = &[
    "GITHUB_TOKEN",
    "GH_TOKEN",
    "FKST_GITHUB_TOKEN_FILE",
    "FKST_GOAL_FILE",
    "GIT_CONFIG_COUNT",
];

/// Key-name PREFIXES the platform owns dynamically. `GIT_CONFIG_KEY_<n>` /
/// `GIT_CONFIG_VALUE_<n>` are git's env-config protocol (#107): their count is
/// not fixed, so the whole `GIT_CONFIG_` family is reserved by prefix rather
/// than by exact name (a user value here could redirect the credential helper).
pub const RESERVED_ENV_NAME_PREFIXES: &[&str] = &["GIT_CONFIG_"];

/// Every platform-managed variable shares this prefix, so a user `env_profile`
/// can never shadow one (e.g. `FKST_RUNTIME_ROOT`, `FKST_DURABLE_ROOT`).
pub const RESERVED_ENV_PREFIX: &str = "FKST_";

/// `env_key` the engine's codex reads the LLM API key from.
///
/// MUST be `LLM_API_KEY`, NOT `FKST_LLM_API_KEY`: the engine's
/// `is_reserved_env_key` strips any `FKST_`-prefixed env var, so an `FKST_`-named
/// key would be silently dropped and the engine would 401. `FKST_LLM_API_KEY` is
/// the CONTROL-PLANE config var name only.
pub const LLM_ENV_KEY: &str = "LLM_API_KEY";

/// Whether a key is platform-owned and must be dropped from a user-supplied
/// `env_profile` before it is applied to an engine child. A key is reserved
/// when it starts with [`RESERVED_ENV_PREFIX`] or any [`RESERVED_ENV_NAME_PREFIXES`]
/// entry, is listed in [`RESERVED_ENV_KEYS`], or names an [`ENGINE_ENV_ALLOWLIST`]
/// host var — so a user entry can never shadow an allow-listed host var or a
/// platform var.
///
/// Shared with the vault write-validator (#100) and the env-injection path
/// (#102) so there is a single source of truth for "keys a user may not set".
pub fn is_reserved_env_key(key: &str) -> bool {
    key.starts_with(RESERVED_ENV_PREFIX)
        || RESERVED_ENV_NAME_PREFIXES
            .iter()
            .any(|prefix| key.starts_with(prefix))
        || RESERVED_ENV_KEYS.contains(&key)
        || ENGINE_ENV_ALLOWLIST.contains(&key)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fkst_prefixed_keys_are_reserved() {
        assert!(is_reserved_env_key("FKST_RUNTIME_ROOT"));
        assert!(is_reserved_env_key("FKST_DURABLE_ROOT"));
        assert!(is_reserved_env_key("FKST_ANYTHING_AT_ALL"));
        assert!(is_reserved_env_key("FKST_"));
    }

    #[test]
    fn explicit_reserved_keys_are_reserved() {
        for key in RESERVED_ENV_KEYS {
            assert!(is_reserved_env_key(key), "{key} must be reserved");
        }
        assert!(is_reserved_env_key("GITHUB_TOKEN"));
    }

    #[test]
    fn allow_list_names_are_reserved_so_user_cannot_shadow_them() {
        // A user env_profile must never override an allow-listed host var.
        for key in ENGINE_ENV_ALLOWLIST {
            assert!(is_reserved_env_key(key), "{key} must be reserved");
        }
        assert!(is_reserved_env_key("PATH"));
        assert!(is_reserved_env_key("HOME"));
        assert!(is_reserved_env_key("CODEX_HOME"));
    }

    #[test]
    fn git_credential_delivery_keys_are_reserved() {
        // #107: the git credential-delivery env must never be user-overridable,
        // or a user value could redirect the credential helper / leak the token.
        assert!(is_reserved_env_key("GH_TOKEN"));
        assert!(is_reserved_env_key("GIT_CONFIG_COUNT"));
        assert!(is_reserved_env_key("GIT_CONFIG_KEY_0"));
        assert!(is_reserved_env_key("GIT_CONFIG_VALUE_0"));
        assert!(is_reserved_env_key("GIT_CONFIG_KEY_99"));
        assert!(is_reserved_env_key("FKST_GITHUB_MINT_NONCE"));
    }

    #[test]
    fn ordinary_user_keys_are_not_reserved() {
        assert!(!is_reserved_env_key("OPENAI_API_KEY"));
        assert!(!is_reserved_env_key("FOO"));
        assert!(!is_reserved_env_key("MY_SECRET"));
        // Case matters: only the exact upper-case platform names are reserved.
        assert!(!is_reserved_env_key("fkst_lowercase"));
        assert!(!is_reserved_env_key("github_token"));
    }
}
