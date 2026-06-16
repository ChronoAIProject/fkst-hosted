//! Vault service: the orchestration layer over [`VaultRepo`] and a
//! [`KeyProvider`]. Owns the write-side validation (key rule, reserved-key
//! denylist, value/entry caps), the encrypt-on-write / decrypt-on-read of
//! secrets, and the consumer-facing read/resolve API ([`Self::list_for_scope`]).
//!
//! Validation lives here (not in the HTTP edge) so the rules apply to every
//! caller — the management API today and any future internal writer. Secret
//! values are encrypted before they reach Mongo and only ever decrypted into a
//! `SecretString`; they are never logged and never returned over HTTP.

use std::collections::BTreeMap;
use std::sync::Arc;

use secrecy::SecretString;
use zeroize::Zeroizing;

use super::crypto::{self, KeyProvider};
use super::model::{
    masked_hint, validate_key, EncryptedBlob, EnvKind, EnvScopeRef, ResolvedEntry, VaultEntry,
};
use super::repo::VaultRepo;
use crate::engine::config::is_reserved_env_key;
use crate::error::AppError;

/// A validated write request handed to [`VaultService::upsert`]. The HTTP edge
/// builds this from its DTO; the value is held in a `Zeroizing` buffer so it is
/// wiped after the service has encrypted/stored it.
pub struct WriteRequest {
    pub owner_user_id: String,
    pub org_id: Option<String>,
    pub scope: EnvScopeRef,
    pub key: String,
    pub kind: EnvKind,
    /// The raw value. Wiped on drop so a plaintext secret does not linger.
    pub value: Zeroizing<String>,
}

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

/// Clonable handle to the vault service: a `VaultRepo` plus the (shared,
/// swappable) `KeyProvider`. Lives in `AppState`.
#[derive(Clone)]
pub struct VaultService {
    repo: VaultRepo,
    keys: Arc<dyn KeyProvider>,
    limits: VaultLimits,
}

impl std::fmt::Debug for VaultService {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // The repo and key provider hold no renderable secret state, but keep
        // the output terse and value-free.
        f.debug_struct("VaultService")
            .field("key_id", &self.keys.key_id())
            .field("limits", &self.limits)
            .finish()
    }
}

impl VaultService {
    /// Build the service from its dependencies.
    pub fn new(repo: VaultRepo, keys: Arc<dyn KeyProvider>, limits: VaultLimits) -> Self {
        Self { repo, keys, limits }
    }

    /// Convenience constructor over the default local KEK provider, the shipped
    /// at-rest backend: bind a fresh [`VaultRepo`] to `db` and key it with a
    /// base64-encoded 32-byte master key. Returns a config error if the key is
    /// not valid base64-encoded 32 bytes (fail closed).
    pub fn with_local_key(
        db: &mongodb::Database,
        master_key_b64: &str,
        limits: VaultLimits,
    ) -> Result<Self, AppError> {
        let provider = super::crypto::LocalKeyProvider::from_base64(master_key_b64)?;
        Ok(Self::new(VaultRepo::new(db), Arc::new(provider), limits))
    }

    /// The underlying repository (for index creation at boot).
    pub fn repo(&self) -> &VaultRepo {
        &self.repo
    }

    /// Validate a write against the key rule, the reserved-key denylist, and
    /// the value-size cap. Returns the `422` reason on failure. Shared by the
    /// upsert path; broken out so the rules are unit-testable without a DB.
    fn validate_write(&self, req: &WriteRequest) -> Result<(), AppError> {
        validate_key(&req.key).map_err(AppError::Unprocessable)?;
        // Reserved keys are platform-owned (single source of truth in
        // `engine::config`): a user must never set one (it would shadow a
        // platform var or an allow-listed host var in an engine run).
        if is_reserved_env_key(&req.key) {
            return Err(AppError::Unprocessable(format!(
                "env var key is reserved by the platform and cannot be set: {}",
                req.key
            )));
        }
        if req.value.len() > self.limits.value_byte_cap {
            return Err(AppError::Unprocessable(format!(
                "value exceeds the {}-byte cap",
                self.limits.value_byte_cap
            )));
        }
        Ok(())
    }

    /// Create or update an entry. Encrypts a `secret` value (storing only the
    /// envelope + masked hint) or stores a `variable` value as plaintext. The
    /// per-scope entry cap is enforced before an insert grows the scope; an
    /// update of an existing key never trips the cap.
    ///
    /// Returns the stored [`VaultEntry`] (the HTTP edge redacts it).
    pub async fn upsert(&self, req: WriteRequest) -> Result<VaultEntry, AppError> {
        self.validate_write(&req)?;

        // Cap check: only a NEW key in this scope consumes a slot. Counting +
        // the conditional check is a benign TOCTOU at this scale (the unique
        // index still prevents duplicate keys); the cap is a soft guard against
        // runaway growth, not a hard concurrency invariant.
        let existing = self
            .repo
            .list_by_scope(&req.owner_user_id, &req.scope)
            .await?;
        let is_new_key = !existing.iter().any(|e| e.key == req.key);
        if is_new_key && existing.len() >= self.limits.entries_per_scope_cap {
            return Err(AppError::Unprocessable(format!(
                "scope already holds the maximum of {} entries",
                self.limits.entries_per_scope_cap
            )));
        }

        let (value_plain, value_enc, hint) = match req.kind {
            EnvKind::Variable => (Some(req.value.to_string()), None, None),
            EnvKind::Secret => {
                let blob = crypto::encrypt(self.keys.as_ref(), req.value.as_bytes())?;
                (None, Some(blob), Some(masked_hint(&req.value)))
            }
        };

        let now = bson::DateTime::now();
        let entry = VaultEntry {
            id: bson::Uuid::new(),
            owner_user_id: req.owner_user_id,
            org_id: req.org_id,
            scope_key: req.scope.scope_key(),
            scope: req.scope,
            key: req.key,
            kind: req.kind,
            value_plain,
            value_enc,
            masked_hint: hint,
            created_at: now,
            updated_at: now,
            created_by: String::new(), // set below from owner (single principal in v1)
        };
        // `created_by` mirrors the owner; the field exists for a future
        // org-shared write path where writer != owner.
        let entry = VaultEntry {
            created_by: entry.owner_user_id.clone(),
            ..entry
        };

        let stored = self.repo.upsert(entry).await?;
        tracing::info!(
            owner = %stored.owner_user_id,
            scope_key = %stored.scope_key,
            key = %stored.key,
            kind = ?stored.kind,
            "vault entry upserted"
        );
        Ok(stored)
    }

    /// Fetch one entry by id without an owner filter (for the HTTP authz path,
    /// which needs the entry's ownership fields before deciding).
    pub async fn get(&self, id: bson::Uuid) -> Result<Option<VaultEntry>, AppError> {
        self.repo.get(id).await
    }

    /// List an owner's entries in a scope (redacted at the HTTP edge — this
    /// returns the raw stored documents, secrets still encrypted).
    pub async fn list_in_scope(
        &self,
        owner_user_id: &str,
        scope: &EnvScopeRef,
    ) -> Result<Vec<VaultEntry>, AppError> {
        self.repo.list_by_scope(owner_user_id, scope).await
    }

    /// Delete an entry by id, scoped to its owner. `Ok(false)` when absent.
    pub async fn delete(&self, id: bson::Uuid, owner_user_id: &str) -> Result<bool, AppError> {
        let deleted = self.repo.delete_owned(id, owner_user_id).await?;
        if deleted {
            tracing::info!(owner = %owner_user_id, id = %id, "vault entry deleted");
        }
        Ok(deleted)
    }

    /// Decrypt a single stored secret blob (helper for consumers that already
    /// hold an entry). Variables are not encrypted, so this is secret-only.
    pub fn decrypt(&self, blob: &EncryptedBlob) -> Result<SecretString, AppError> {
        crypto::decrypt(self.keys.as_ref(), blob)
    }

    /// Resolve the effective env for a session: every entry the owner has at
    /// `global` scope, overlaid by `scope.repo`'s entries (repo wins on a key
    /// collision), each decrypted (secret) or passed through (variable), with
    /// exactly one [`ResolvedEntry`] per key.
    ///
    /// This is the API #102 consumes to build a per-session env profile. It is
    /// NEVER serialized over HTTP — the returned `SecretString`s carry decrypted
    /// values in memory only.
    ///
    /// Ordered merge: (1) collect global entries into a key→entry map; (2) if a
    /// repo is given, overlay that repo's entries into the same map (repo
    /// replaces global for the same key); (3) materialize one resolved entry per
    /// key. A `BTreeMap` keeps the output deterministic (key-sorted).
    pub async fn list_for_scope(
        &self,
        owner_user_id: &str,
        _org_id: Option<&str>,
        scope: &EnvScopeRef,
    ) -> Result<Vec<ResolvedEntry>, AppError> {
        // 1. Global layer.
        let mut merged: BTreeMap<String, VaultEntry> = BTreeMap::new();
        for entry in self
            .repo
            .list_by_scope(owner_user_id, &EnvScopeRef::global())
            .await?
        {
            merged.insert(entry.key.clone(), entry);
        }

        // 2. Repo overlay (repo wins on collision).
        if scope.repo.is_some() {
            for entry in self.repo.list_by_scope(owner_user_id, scope).await? {
                merged.insert(entry.key.clone(), entry);
            }
        }

        // 3. Decrypt / pass through, one resolved entry per key.
        let mut resolved = Vec::with_capacity(merged.len());
        for (key, entry) in merged {
            let value = match entry.kind {
                EnvKind::Variable => SecretString::from(entry.value_plain.unwrap_or_default()),
                EnvKind::Secret => {
                    let blob = entry.value_enc.ok_or_else(|| {
                        AppError::Unprocessable(format!("secret entry {key} has no ciphertext"))
                    })?;
                    self.decrypt(&blob)?
                }
            };
            resolved.push(ResolvedEntry { key, value });
        }
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
    use crate::vault::crypto::LocalKeyProvider;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine as _;
    use secrecy::ExposeSecret;

    fn test_provider() -> Arc<dyn KeyProvider> {
        Arc::new(LocalKeyProvider::from_base64(&BASE64.encode([3u8; 32])).expect("provider"))
    }

    fn limits() -> VaultLimits {
        VaultLimits {
            value_byte_cap: 64 * 1024,
            entries_per_scope_cap: 100,
        }
    }

    fn req(kind: EnvKind, key: &str, value: &str) -> WriteRequest {
        WriteRequest {
            owner_user_id: "u1".to_string(),
            org_id: None,
            scope: EnvScopeRef::global(),
            key: key.to_string(),
            kind,
            value: Zeroizing::new(value.to_string()),
        }
    }

    // `validate_write` is a method on the service but needs no DB; build a
    // service over an UNCONNECTED collection handle (the driver connects
    // lazily, and these tests never issue a DB op) for the pure validation
    // tests.
    async fn validation_service() -> VaultService {
        let db = mongodb::Client::with_uri_str("mongodb://localhost:27017")
            .await
            .expect("client")
            .database("vault_unit_test");
        VaultService::new(VaultRepo::new(&db), test_provider(), limits())
    }

    #[tokio::test]
    async fn validate_rejects_invalid_key() {
        let svc = validation_service().await;
        let err = svc
            .validate_write(&req(EnvKind::Variable, "1bad", "v"))
            .expect_err("must reject");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn validate_rejects_reserved_keys() {
        let svc = validation_service().await;
        for key in ["FKST_FOO", "GITHUB_TOKEN", "PATH"] {
            let err = svc
                .validate_write(&req(EnvKind::Secret, key, "v"))
                .expect_err("reserved must reject");
            assert!(matches!(err, AppError::Unprocessable(_)), "{key}: {err:?}");
        }
    }

    #[tokio::test]
    async fn validate_rejects_oversized_value() {
        let db = mongodb::Client::with_uri_str("mongodb://localhost:27017")
            .await
            .expect("client")
            .database("vault_unit_test");
        let svc = VaultService::new(
            VaultRepo::new(&db),
            test_provider(),
            VaultLimits {
                value_byte_cap: 8,
                entries_per_scope_cap: 100,
            },
        );
        let err = svc
            .validate_write(&req(EnvKind::Secret, "K", "0123456789"))
            .expect_err("oversized must reject");
        assert!(matches!(err, AppError::Unprocessable(_)), "got {err:?}");
    }

    #[tokio::test]
    async fn validate_accepts_ordinary_key_and_value() {
        let svc = validation_service().await;
        assert!(svc
            .validate_write(&req(EnvKind::Secret, "OPENAI_API_KEY", "sk-x"))
            .is_ok());
    }

    #[test]
    fn decrypt_round_trips_through_service_helper() {
        let svc_keys = test_provider();
        let blob = crypto::encrypt(svc_keys.as_ref(), b"hello").expect("encrypt");
        let recovered = crypto::decrypt(svc_keys.as_ref(), &blob).expect("decrypt");
        assert_eq!(recovered.expose_secret(), "hello");
    }
}
