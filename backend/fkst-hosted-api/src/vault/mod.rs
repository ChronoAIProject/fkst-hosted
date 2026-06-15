//! Per-session secret & variable vault (issue #100).
//!
//! A fkst-hosted-owned, encrypted-at-rest key–value store for the env
//! **variables** (non-secret) and **secrets** an engine run needs. The module
//! splits into cohesive units:
//! - [`model`] — the persisted `VaultEntry`, the `EnvKind` distinction, the
//!   envelope `EncryptedBlob`, the `EnvScopeRef` pointer (consumed by #102), the
//!   in-memory `ResolvedEntry`, and the redacting DTO helpers.
//! - [`crypto`] — the `KeyProvider` swap seam and the envelope AES-256-GCM
//!   encrypt/decrypt.
//! - [`repo`] — MongoDB CRUD + the unique-per-scope index.
//! - [`service`] — write-side validation, encrypt-on-write / decrypt-on-read,
//!   and the consumer read/resolve API `list_for_scope`.
//!
//! Security contract: a secret value is never logged, never returned over HTTP,
//! and never stored in plaintext. Secrets live in `secrecy` types that redact in
//! `Debug` and zeroize on drop; plaintext and DEK buffers are wiped after use.

pub mod crypto;
pub mod model;
pub mod repo;
pub mod service;

pub use crypto::{KeyProvider, KmsKeyProvider, LocalKeyProvider};
pub use model::{
    EncryptedBlob, EnvKind, EnvScopeRef, RepoRef, ResolvedEntry, VaultEntry, ENVELOPE_ALG,
};
pub use repo::VaultRepo;
pub use service::{VaultLimits, VaultService, WriteRequest};
