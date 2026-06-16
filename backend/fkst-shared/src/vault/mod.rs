//! Vault role-neutral data model (issue #145 extraction).
//!
//! Only the persisted shapes live in the shared crate: the `VaultEntry`, the
//! `EnvKind` distinction, the envelope `EncryptedBlob`, the `EnvScopeRef`
//! pointer, the in-memory `ResolvedEntry`, and the redacting helpers. The
//! encrypting service, the AES-256-GCM crypto, and the Mongo repo stay
//! control-plane — the worker gets only a read-only resolve seam and NEVER
//! holds the KEK.

pub mod model;

pub use model::{
    EncryptedBlob, EnvKind, EnvScopeRef, RepoRef, ResolvedEntry, VaultEntry, ENVELOPE_ALG,
};
