//! Vault data model: the typed variable/secret distinction ([`EnvKind`]), the
//! non-secret [`EnvScopeRef`] pointer (the in-memory store's scope key), the
//! in-memory [`ResolvedEntry`] consumers receive, and the env-var-key rule.
//!
//! Database-free pivot (sa:db-free, #138): the persisted `VaultEntry` document,
//! the envelope-encrypted `EncryptedBlob`, the at-rest algorithm tag, and the
//! display-only `masked_hint` were removed — secrets are now in-memory only and
//! reach the worker over the TLS controller↔worker channel, so there is no
//! at-rest document or crypto here. The surviving types are pure value objects
//! (no secret material), plus `ResolvedEntry`, whose `SecretString` value
//! redacts in `Debug` and zeroizes on drop.

use std::fmt;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub use crate::models::RepoRef;

/// Anchored env-var-key rule: a leading letter or underscore, then letters,
/// digits, or underscores. Mirrors the POSIX-ish convention `codex` and the
/// engine expect for environment variable names.
const ENV_KEY_PATTERN: &str = "^[A-Za-z_][A-Za-z0-9_]*$";

/// Kind of an inline env entry. `variable` is non-secret config; `secret` is a
/// credential injected into the engine env in memory only. Serializes lowercase
/// on the wire (the trigger DTO tags each inline entry variable-vs-secret).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[derive(utoipa::ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum EnvKind {
    Variable,
    Secret,
}

/// Non-secret pointer to the scope an entry lives in: either owner-wide
/// (`global`) or a specific repo. Consumed by #102 to build a per-session env
/// profile, and the in-memory secret store keys on [`Self::scope_key`]. Exactly
/// one of `global == true` or `repo == Some(_)` is meaningful. Still serialized
/// (it is persisted on `SessionDoc.env_scope` so a failover rebuild re-resolves
/// the identical scope).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EnvScopeRef {
    /// `Some` for a repo-scoped entry; `None` for a global one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repo: Option<RepoRef>,
    /// `true` for an owner-wide entry. A repo-scoped entry sets this `false`.
    pub global: bool,
}

impl EnvScopeRef {
    /// Owner-wide scope.
    pub fn global() -> Self {
        Self {
            repo: None,
            global: true,
        }
    }

    /// Repo-specific scope.
    pub fn repo(owner: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            repo: Some(RepoRef {
                owner: owner.into(),
                name: name.into(),
            }),
            global: false,
        }
    }

    /// Canonical scope string: `"global"` or `"repo:<owner>/<name>"`. This is
    /// the in-memory secret-store map key, so global and repo entries of the
    /// same `key` resolve distinctly. A repo pointer always wins over the
    /// `global` flag (a malformed both-set ref resolves to the repo so it can
    /// never collide with the global slot of the same key).
    pub fn scope_key(&self) -> String {
        match &self.repo {
            Some(repo) => format!("repo:{}/{}", repo.owner, repo.name),
            None => "global".to_string(),
        }
    }
}

/// A resolved entry handed to consumers (#102): the env-var `key` and its
/// in-memory `value`. `SecretString` redacts in `Debug` and zeroizes on drop.
/// This type is NEVER serialized over HTTP.
#[derive(Clone)]
pub struct ResolvedEntry {
    pub key: String,
    pub value: SecretString,
}

impl fmt::Debug for ResolvedEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ResolvedEntry")
            .field("key", &self.key)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// Validate an env-var key against [`ENV_KEY_PATTERN`]. Returns the reason
/// (for a `422`) on failure.
pub fn validate_key(key: &str) -> Result<(), String> {
    // A single compiled regex per process; the pattern is a constant so the
    // compile is infallible.
    static KEY_RE: once_cell::sync::Lazy<regex::Regex> =
        once_cell::sync::Lazy::new(|| regex::Regex::new(ENV_KEY_PATTERN).expect("valid regex"));
    if KEY_RE.is_match(key) {
        Ok(())
    } else {
        Err(format!("invalid env var key: must match {ENV_KEY_PATTERN}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use secrecy::ExposeSecret;

    #[test]
    fn env_kind_serializes_lowercase() {
        assert_eq!(
            bson::to_bson(&EnvKind::Variable).unwrap(),
            bson::Bson::String("variable".to_string())
        );
        assert_eq!(
            bson::to_bson(&EnvKind::Secret).unwrap(),
            bson::Bson::String("secret".to_string())
        );
    }

    #[test]
    fn scope_key_is_global_for_global_scope() {
        assert_eq!(EnvScopeRef::global().scope_key(), "global");
    }

    #[test]
    fn scope_key_encodes_repo() {
        assert_eq!(
            EnvScopeRef::repo("acme", "site").scope_key(),
            "repo:acme/site"
        );
    }

    #[test]
    fn scope_key_prefers_repo_when_both_set() {
        // Defensive: a both-set ref must collapse to the repo slot so it can
        // never collide with the global slot of the same key.
        let scope = EnvScopeRef {
            repo: Some(RepoRef {
                owner: "acme".to_string(),
                name: "site".to_string(),
            }),
            global: true,
        };
        assert_eq!(scope.scope_key(), "repo:acme/site");
    }

    #[test]
    fn validate_key_accepts_valid_names() {
        for ok in ["OPENAI_API_KEY", "_underscore", "A", "x1", "MY_VAR_2"] {
            assert!(validate_key(ok).is_ok(), "{ok} should be valid");
        }
    }

    #[test]
    fn validate_key_rejects_invalid_names() {
        for bad in [
            "1LEADS_WITH_DIGIT",
            "has-dash",
            "has space",
            "",
            "dot.key",
            "$VAR",
        ] {
            assert!(validate_key(bad).is_err(), "{bad} should be invalid");
        }
    }

    #[test]
    fn resolved_entry_debug_redacts_value() {
        let entry = ResolvedEntry {
            key: "OPENAI_API_KEY".to_string(),
            value: SecretString::from("sk-leaky".to_string()),
        };
        let rendered = format!("{entry:?}");
        assert!(!rendered.contains("sk-leaky"), "value leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
        // The value is still recoverable for the consumer.
        assert_eq!(entry.value.expose_secret(), "sk-leaky");
    }
}
