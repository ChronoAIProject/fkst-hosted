---
"fkst-hosted": minor
---

Add a fkst-hosted-owned, encrypted-at-rest secret & variable vault (issue #100): a typed key–value store (`variable` vs `secret`) scoped per-owner to `global` or a specific repo, a `/api/v1/vault/*` CRUD API governed by the existing owner/org authorization (secrets are write-only over HTTP — never returned, logged, or stored in plaintext), and a `VaultService::list_for_scope` read/resolve API plus shared `EnvScopeRef`/`EncryptedBlob`/`ResolvedEntry` types for the injection path (#102). Secrets are envelope-encrypted with AES-256-GCM (random per-secret DEK + nonce, DEK wrapped by a `KeyProvider` KEK); the vault is always-on and fails closed at boot when no master key (`FKST_HOSTED_VAULT_MASTER_KEY` / `_PATH`, base64 32 bytes) is configured.
