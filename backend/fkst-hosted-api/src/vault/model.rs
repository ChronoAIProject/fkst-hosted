//! Vault data model: the persisted `VaultEntry` document, the typed
//! variable/secret distinction, the envelope-encrypted `EncryptedBlob`, the
//! non-secret `EnvScopeRef` pointer (consumed by #102), the in-memory
//! `ResolvedEntry`, and the redacting request/response DTOs.
//!
//! Redaction discipline: ANY type that carries (or could carry) a secret value
//! has a hand-written `Debug` that prints `<redacted>` for the value-bearing
//! fields — mirroring the `github_token` / GitHub-App-PEM discipline so a
//! secret can never leak through `{:?}` into a log line.

use std::fmt;

use secrecy::SecretString;
use serde::{Deserialize, Serialize};

pub use crate::models::RepoRef;

/// Anchored env-var-key rule: a leading letter or underscore, then letters,
/// digits, or underscores. Mirrors the POSIX-ish convention `codex` and the
/// engine expect for environment variable names.
const ENV_KEY_PATTERN: &str = "^[A-Za-z_][A-Za-z0-9_]*$";

/// Algorithm tag stamped into every `EncryptedBlob`. Versioned so a future
/// envelope scheme (e.g. a KMS-wrapped DEK) can coexist with v1 blobs.
pub const ENVELOPE_ALG: &str = "AES-256-GCM/envelope-v1";

/// Canonical scope key for owner-wide (global) entries.
pub const SCOPE_KEY_GLOBAL: &str = "global";

/// Kind of a vault entry. `variable` is non-secret config stored in plaintext
/// and returned over HTTP; `secret` is envelope-encrypted at rest and never
/// returned. Serializes lowercase on the wire and in BSON.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum EnvKind {
    Variable,
    Secret,
}

/// Envelope-encrypted secret value: AES-256-GCM ciphertext under a per-secret
/// DEK, the DEK wrapped by the `KeyProvider`'s KEK. The plaintext is never
/// stored; only `masked_hint` (display-only) hints at it.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct EncryptedBlob {
    /// AES-256-GCM ciphertext of the secret value (includes the GCM tag).
    pub ciphertext: Vec<u8>,
    /// 96-bit GCM nonce, freshly random per encrypt.
    pub nonce: [u8; 12],
    /// The per-secret DEK, wrapped (encrypted) by the KEK.
    pub wrapped_dek: Vec<u8>,
    /// Identifier of the KEK that wrapped `wrapped_dek` (for future rotation).
    pub key_id: String,
    /// Algorithm tag; see [`ENVELOPE_ALG`].
    pub alg: String,
}

// The ciphertext/wrapped-DEK bytes are not the plaintext, but a redacting Debug
// keeps the discipline uniform: no secret-adjacent material ever renders.
impl fmt::Debug for EncryptedBlob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("EncryptedBlob")
            .field("ciphertext", &"<redacted>")
            .field("nonce", &"<redacted>")
            .field("wrapped_dek", &"<redacted>")
            .field("key_id", &self.key_id)
            .field("alg", &self.alg)
            .finish()
    }
}

/// Non-secret pointer to the scope an entry lives in: either owner-wide
/// (`global`) or a specific repo. Consumed by #102 to build a per-session env
/// profile. Exactly one of `global == true` or `repo == Some(_)` is meaningful
/// for a stored entry; `scope_key` derives the canonical index key.
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

    /// Canonical, index-friendly string for this scope: `"global"` or
    /// `"repo:<owner>/<name>"`. This is what the unique index keys on so global
    /// and repo entries of the same `key` index distinctly. A repo pointer
    /// always wins over the `global` flag (a malformed both-set ref resolves to
    /// the repo so it can never collide with the global slot of the same key).
    pub fn scope_key(&self) -> String {
        match &self.repo {
            Some(repo) => format!("repo:{}/{}", repo.owner, repo.name),
            None => SCOPE_KEY_GLOBAL.to_string(),
        }
    }
}

/// `vault_entries` collection document. `_id` is a UUID stored as BSON Binary
/// subtype 4 (mirrors `SessionDoc`). The denormalized `scope_key` is what the
/// unique index `(owner_user_id, scope_key, key)` keys on. `Debug` is
/// hand-written below to redact the value fields.
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultEntry {
    #[serde(rename = "_id")]
    pub id: bson::Uuid,
    pub owner_user_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub org_id: Option<String>,
    pub scope: EnvScopeRef,
    /// Denormalized canonical scope (see [`EnvScopeRef::scope_key`]). Persisted
    /// so the unique index can span it without a computed-key expression.
    pub scope_key: String,
    pub key: String,
    pub kind: EnvKind,
    /// `Some` only for a `variable` (non-secret plaintext config).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_plain: Option<String>,
    /// `Some` only for a `secret` (envelope-encrypted at rest).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_enc: Option<EncryptedBlob>,
    /// Display-only hint for a secret (`"…last4"`); never the value itself.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub masked_hint: Option<String>,
    pub created_at: bson::DateTime,
    pub updated_at: bson::DateTime,
    pub created_by: String,
}

// `value_plain` is non-secret by definition, but a `variable` can still hold
// sensitive-looking config; redact both value fields so neither a stored
// variable value nor any encrypted material can render through `{:?}`.
impl fmt::Debug for VaultEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("VaultEntry")
            .field("id", &self.id)
            .field("owner_user_id", &self.owner_user_id)
            .field("org_id", &self.org_id)
            .field("scope", &self.scope)
            .field("scope_key", &self.scope_key)
            .field("key", &self.key)
            .field("kind", &self.kind)
            .field(
                "value_plain",
                &self.value_plain.as_ref().map(|_| "<redacted>"),
            )
            .field("value_enc", &self.value_enc)
            .field("masked_hint", &self.masked_hint)
            .field("created_at", &self.created_at)
            .field("updated_at", &self.updated_at)
            .field("created_by", &self.created_by)
            .finish()
    }
}

/// A resolved entry handed to consumers (#102): the env-var `key` and its
/// in-memory `value` (a secret decrypted to `SecretString`, or a variable as
/// is). `SecretString` redacts in `Debug` and zeroizes on drop. This type is
/// NEVER serialized over HTTP.
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

/// Build the display-only masked hint for a secret value: `"…last4"`, where the
/// tail is the last up-to-4 characters. A short value reveals fewer characters
/// rather than padding, so it never implies a longer secret than was stored.
pub fn masked_hint(value: &str) -> String {
    let tail: String = value
        .chars()
        .rev()
        .take(4)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
        .collect();
    format!("…{tail}")
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
    fn masked_hint_shows_last_four() {
        assert_eq!(masked_hint("sk-1234567890"), "…7890");
    }

    #[test]
    fn masked_hint_short_value_reveals_fewer() {
        assert_eq!(masked_hint("ab"), "…ab");
        assert_eq!(masked_hint(""), "…");
    }

    #[test]
    fn masked_hint_handles_multibyte() {
        // Last four chars (code points), not bytes — never split a code point.
        // The hint is the ellipsis prefix plus the last-4 tail.
        assert_eq!(masked_hint("你好世界𝕊"), "…好世界𝕊");
    }

    #[test]
    fn encrypted_blob_debug_redacts_material() {
        let blob = EncryptedBlob {
            ciphertext: vec![1, 2, 3],
            nonce: [9u8; 12],
            wrapped_dek: vec![4, 5, 6],
            key_id: "local-v1".to_string(),
            alg: ENVELOPE_ALG.to_string(),
        };
        let rendered = format!("{blob:?}");
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("local-v1"));
        assert!(
            !rendered.contains("[1, 2, 3]"),
            "ciphertext leaked: {rendered}"
        );
        assert!(
            !rendered.contains("[4, 5, 6]"),
            "wrapped dek leaked: {rendered}"
        );
    }

    #[test]
    fn vault_entry_debug_redacts_value_plain() {
        let entry = VaultEntry {
            id: bson::Uuid::new(),
            owner_user_id: "u1".to_string(),
            org_id: None,
            scope: EnvScopeRef::global(),
            scope_key: "global".to_string(),
            key: "FEATURE_FLAG".to_string(),
            kind: EnvKind::Variable,
            value_plain: Some("super-sensitive-config".to_string()),
            value_enc: None,
            masked_hint: None,
            created_at: bson::DateTime::now(),
            updated_at: bson::DateTime::now(),
            created_by: "u1".to_string(),
        };
        let rendered = format!("{entry:?}");
        assert!(
            !rendered.contains("super-sensitive-config"),
            "value_plain leaked: {rendered}"
        );
        assert!(rendered.contains("<redacted>"));
        assert!(rendered.contains("FEATURE_FLAG"), "key should be visible");
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

    #[test]
    fn vault_entry_round_trips_through_bson() {
        let entry = VaultEntry {
            id: bson::Uuid::new(),
            owner_user_id: "u1".to_string(),
            org_id: Some("org-1".to_string()),
            scope: EnvScopeRef::repo("acme", "site"),
            scope_key: "repo:acme/site".to_string(),
            key: "TOKEN".to_string(),
            kind: EnvKind::Secret,
            value_plain: None,
            value_enc: Some(EncryptedBlob {
                ciphertext: vec![1, 2, 3],
                nonce: [7u8; 12],
                wrapped_dek: vec![4, 5, 6],
                key_id: "local-v1".to_string(),
                alg: ENVELOPE_ALG.to_string(),
            }),
            masked_hint: Some("…last".to_string()),
            created_at: bson::DateTime::from_millis(1_700_000_000_000),
            updated_at: bson::DateTime::from_millis(1_700_000_000_000),
            created_by: "u1".to_string(),
        };
        let raw = bson::to_document(&entry).expect("serialize");
        let back: VaultEntry = bson::from_document(raw).expect("deserialize");
        assert_eq!(back, entry);
    }
}
