//! Vault service: the in-memory store of inline secrets (database-free pivot,
//! #138). Secrets are supplied inline at goal trigger, held here by the
//! controller as zeroizing `SecretString`s keyed by `(owner_user_id,
//! scope_key)`, and resolved into a per-session env profile at spawn — exactly
//! the [`Self::list_for_scope`] contract `#102`'s consumers already use.
//!
//! There is no persistence and no at-rest crypto: secrets live only in this
//! process map and reach the worker over the TLS controller↔worker channel.
//! Validation (key rule, reserved-key denylist, value/entry caps) lives here so
//! it applies to every writer. Values are never logged and never returned over
//! HTTP — only key NAMES and counts are ever logged.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, RwLock};

use secrecy::{ExposeSecret, SecretString};

use super::model::{validate_key, EnvKind, EnvScopeRef, ResolvedEntry};
use crate::engine::config::is_reserved_env_key;
use crate::error::AppError;

/// Per-scope limits, sourced from config. Defaults applied by `crate::config`.
#[derive(Debug, Clone, Copy)]
pub struct VaultLimits {
    /// Max bytes for a single value.
    pub value_byte_cap: usize,
    /// Max entries an owner may hold in one scope.
    pub entries_per_scope_cap: usize,
}

impl Default for VaultLimits {
    /// The shipped defaults (also the `FKST_HOSTED_VAULT_*` serde defaults):
    /// 64 KiB per value, 100 entries per scope.
    fn default() -> Self {
        Self {
            value_byte_cap: 65_536,
            entries_per_scope_cap: 100,
        }
    }
}

/// One inline entry held in memory: its kind (variable vs secret) and the
/// zeroizing value. `Debug` is hand-written to redact the value so a secret can
/// never render through `{:?}`.
#[derive(Clone)]
struct InlineEntry {
    kind: EnvKind,
    value: SecretString,
}

impl std::fmt::Debug for InlineEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InlineEntry")
            .field("kind", &self.kind)
            .field("value", &"<redacted>")
            .finish()
    }
}

/// In-memory map key: `(owner_user_id, scope.scope_key())`.
type ScopeKey = (String, String);

/// Clonable handle to the in-memory secret store. Lives in `AppState` and is
/// shared with the session driver (it is `Arc`-backed, so a clone shares the
/// same map). The single controller authority is the only writer, so a
/// scope-replace under the lock has no TOCTOU concern.
#[derive(Clone)]
pub struct VaultService {
    entries: Arc<RwLock<HashMap<ScopeKey, BTreeMap<String, InlineEntry>>>>,
    limits: VaultLimits,
}

impl std::fmt::Debug for VaultService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never render values; the scope count is a safe, value-free summary.
        let scopes = self.entries.read().map(|m| m.len()).unwrap_or(0);
        f.debug_struct("VaultService")
            .field("scopes", &scopes)
            .field("limits", &self.limits)
            .finish()
    }
}

impl VaultService {
    /// Build an empty in-memory store with the configured caps.
    pub fn new(limits: VaultLimits) -> Self {
        Self {
            entries: Arc::new(RwLock::new(HashMap::new())),
            limits,
        }
    }

    /// Replace the inline entries for `(owner_user_id, scope)`. Validates every
    /// key (rule + reserved denylist), each value against the byte cap, and the
    /// scope's entry count against the per-scope cap BEFORE mutating, so a
    /// rejected write leaves the prior scope untouched. The single controller
    /// authority is the only writer, so replacing the scope map is race-free.
    ///
    /// Logs key NAMES + count only — never a value.
    pub fn set_inline(
        &self,
        owner_user_id: &str,
        scope: &EnvScopeRef,
        entries: Vec<(String, EnvKind, SecretString)>,
    ) -> Result<(), AppError> {
        if entries.len() > self.limits.entries_per_scope_cap {
            return Err(AppError::Unprocessable(format!(
                "scope exceeds the maximum of {} entries",
                self.limits.entries_per_scope_cap
            )));
        }
        let mut built: BTreeMap<String, InlineEntry> = BTreeMap::new();
        for (key, kind, value) in entries {
            validate_key(&key).map_err(AppError::Unprocessable)?;
            // Reserved keys are platform-owned (single source of truth in
            // `engine::config`): a user must never set one (it would shadow a
            // platform var or an allow-listed host var in an engine run).
            if is_reserved_env_key(&key) {
                return Err(AppError::Unprocessable(format!(
                    "env var key is reserved by the platform and cannot be set: {key}"
                )));
            }
            if value.expose_secret().len() > self.limits.value_byte_cap {
                return Err(AppError::Unprocessable(format!(
                    "value for {key} exceeds the {}-byte cap",
                    self.limits.value_byte_cap
                )));
            }
            built.insert(key, InlineEntry { kind, value });
        }
        let names: Vec<&str> = built.keys().map(String::as_str).collect();
        tracing::info!(
            owner = %owner_user_id,
            scope_key = %scope.scope_key(),
            count = built.len(),
            keys = %names.join(","),
            "inline secrets set for scope"
        );
        self.entries
            .write()
            .expect("vault store poisoned")
            .insert((owner_user_id.to_string(), scope.scope_key()), built);
        Ok(())
    }

    /// Drop a scope's inline secrets (called from the driver teardown on a
    /// genuine terminal exit) so secret material does not linger in memory
    /// beyond the run.
    pub fn clear_inline(&self, owner_user_id: &str, scope: &EnvScopeRef) {
        let removed = self
            .entries
            .write()
            .expect("vault store poisoned")
            .remove(&(owner_user_id.to_string(), scope.scope_key()))
            .is_some();
        if removed {
            tracing::debug!(
                owner = %owner_user_id,
                scope_key = %scope.scope_key(),
                "inline secrets cleared for scope"
            );
        }
    }

    /// Resolve the effective env for a session: every inline entry the owner has
    /// at `global` scope, overlaid by `scope.repo`'s entries (repo wins on a key
    /// collision), with exactly one [`ResolvedEntry`] per key, key-sorted.
    ///
    /// This is the API #102 consumes to build a per-session env profile; it is
    /// NEVER serialized over HTTP. It is `async` only to preserve the callers'
    /// `.await`; the body does no I/O (a bounded read under the sync lock, the
    /// lock dropped before returning — never held across an await).
    pub async fn list_for_scope(
        &self,
        owner_user_id: &str,
        _org_id: Option<&str>,
        scope: &EnvScopeRef,
    ) -> Result<Vec<ResolvedEntry>, AppError> {
        let resolved = {
            let store = self.entries.read().expect("vault store poisoned");
            // 1. Global layer.
            let mut merged: BTreeMap<String, SecretString> = BTreeMap::new();
            let global_key = (owner_user_id.to_string(), EnvScopeRef::global().scope_key());
            if let Some(scope_map) = store.get(&global_key) {
                for (key, entry) in scope_map {
                    merged.insert(key.clone(), entry.value.clone());
                }
            }
            // 2. Repo overlay (repo wins on collision).
            if scope.repo.is_some() {
                let repo_key = (owner_user_id.to_string(), scope.scope_key());
                if let Some(scope_map) = store.get(&repo_key) {
                    for (key, entry) in scope_map {
                        merged.insert(key.clone(), entry.value.clone());
                    }
                }
            }
            merged
                .into_iter()
                .map(|(key, value)| ResolvedEntry { key, value })
                .collect::<Vec<_>>()
        };
        tracing::debug!(
            owner = %owner_user_id,
            scope_key = %scope.scope_key(),
            count = resolved.len(),
            "vault scope resolved"
        );
        Ok(resolved)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits() -> VaultLimits {
        VaultLimits {
            value_byte_cap: 64 * 1024,
            entries_per_scope_cap: 100,
        }
    }

    fn secret(s: &str) -> SecretString {
        SecretString::from(s.to_string())
    }

    fn entry(key: &str, value: &str) -> (String, EnvKind, SecretString) {
        (key.to_string(), EnvKind::Secret, secret(value))
    }

    #[tokio::test]
    async fn set_inline_then_list_resolves_in_order() {
        let svc = VaultService::new(limits());
        // Global layer: A, B. Repo overlay: B (wins), C.
        svc.set_inline(
            "u1",
            &EnvScopeRef::global(),
            vec![entry("A_KEY", "ga"), entry("B_KEY", "gb")],
        )
        .unwrap();
        let repo = EnvScopeRef::repo("acme", "site");
        svc.set_inline(
            "u1",
            &repo,
            vec![entry("B_KEY", "rb"), entry("C_KEY", "rc")],
        )
        .unwrap();

        let resolved = svc.list_for_scope("u1", None, &repo).await.unwrap();
        let kv: Vec<(String, String)> = resolved
            .iter()
            .map(|e| (e.key.clone(), e.value.expose_secret().to_string()))
            .collect();
        // Key-sorted, repo wins B_KEY.
        assert_eq!(
            kv,
            vec![
                ("A_KEY".to_string(), "ga".to_string()),
                ("B_KEY".to_string(), "rb".to_string()),
                ("C_KEY".to_string(), "rc".to_string()),
            ]
        );
    }

    #[tokio::test]
    async fn set_inline_rejects_reserved_key() {
        let svc = VaultService::new(limits());
        for key in ["GITHUB_TOKEN", "PATH", "FKST_FOO"] {
            let err = svc
                .set_inline("u1", &EnvScopeRef::global(), vec![entry(key, "v")])
                .expect_err("reserved must reject");
            assert!(matches!(err, AppError::Unprocessable(_)), "{key}: {err:?}");
        }
    }

    #[tokio::test]
    async fn set_inline_rejects_invalid_key() {
        let svc = VaultService::new(limits());
        let err = svc
            .set_inline("u1", &EnvScopeRef::global(), vec![entry("1bad", "v")])
            .expect_err("invalid key must reject");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn set_inline_enforces_value_cap() {
        let svc = VaultService::new(VaultLimits {
            value_byte_cap: 8,
            entries_per_scope_cap: 100,
        });
        let err = svc
            .set_inline("u1", &EnvScopeRef::global(), vec![entry("K", "0123456789")])
            .expect_err("oversized must reject");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn set_inline_enforces_entries_per_scope_cap() {
        let svc = VaultService::new(VaultLimits {
            value_byte_cap: 64 * 1024,
            entries_per_scope_cap: 2,
        });
        let err = svc
            .set_inline(
                "u1",
                &EnvScopeRef::global(),
                vec![entry("A", "1"), entry("B", "2"), entry("C", "3")],
            )
            .expect_err("too many entries must reject");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn clear_inline_drops_the_scope() {
        let svc = VaultService::new(limits());
        let repo = EnvScopeRef::repo("acme", "site");
        svc.set_inline("u1", &repo, vec![entry("K", "v")]).unwrap();
        assert_eq!(
            svc.list_for_scope("u1", None, &repo).await.unwrap().len(),
            1
        );
        svc.clear_inline("u1", &repo);
        assert!(svc
            .list_for_scope("u1", None, &repo)
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn list_for_scope_is_empty_for_unknown_owner() {
        let svc = VaultService::new(limits());
        let resolved = svc
            .list_for_scope("nobody", None, &EnvScopeRef::global())
            .await
            .unwrap();
        assert!(resolved.is_empty());
    }

    #[test]
    fn inline_entry_debug_redacts_value() {
        let entry = InlineEntry {
            kind: EnvKind::Secret,
            value: secret("sk-leaky"),
        };
        let rendered = format!("{entry:?}");
        assert!(!rendered.contains("sk-leaky"), "value leaked: {rendered}");
        assert!(rendered.contains("<redacted>"));
    }
}
