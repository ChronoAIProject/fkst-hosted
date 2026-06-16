//! Vault role-neutral data model (issue #145 extraction).
//!
//! Only the secret-free value objects live in the shared crate: the [`EnvKind`]
//! distinction, the [`EnvScopeRef`] scope pointer, the in-memory
//! [`ResolvedEntry`] consumers receive, and the env-var-key rule. The in-memory
//! secret store (`VaultService`) stays control-plane — the worker gets only a
//! read-only resolve seam and NEVER holds secret material.
//!
//! Database-free pivot (#138): the persisted `VaultEntry`, the envelope
//! `EncryptedBlob`, and the at-rest crypto were removed — secrets are in-memory
//! only and reach the worker over the TLS controller↔worker channel.

pub mod model;

pub use model::{EnvKind, EnvScopeRef, RepoRef, ResolvedEntry};
