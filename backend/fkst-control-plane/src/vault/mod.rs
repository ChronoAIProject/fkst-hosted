//! Per-session secret & variable vault (issue #100), database-free (#138).
//!
//! A fkst-hosted-owned, **in-memory** key–value store for the env **variables**
//! (non-secret) and **secrets** an engine run needs. Secrets are supplied inline
//! at goal trigger, held by the controller, and resolved into the per-session
//! env profile at spawn. The module splits into cohesive units:
//! - [`model`] — the `EnvKind` distinction, the `EnvScopeRef` scope pointer, the
//!   in-memory `ResolvedEntry`, and the env-var-key rule (in `fkst-shared`).
//! - [`service`] — write-side validation (key rule, reserved-key denylist,
//!   value/entry caps), the in-memory store, and the consumer read/resolve API
//!   `list_for_scope`.
//!
//! At-rest envelope encryption was removed in the DB-free pivot (sa:db-free):
//! secrets are in-memory only and reach the worker over the TLS controller↔worker
//! channel. Re-introduce a `KeyProvider` seam here if at-transit-rest encryption
//! is later required.
//!
//! Security contract: a secret value is never logged, never returned over HTTP,
//! and never persisted. Secrets live in `secrecy` types that redact in `Debug`
//! and zeroize on drop.

// `model` was extracted to `fkst-shared` (issue #145); re-export it so
// `crate::vault::model::…` keeps resolving for the service and callers.
pub mod model;
pub mod service;

pub use model::{EnvKind, EnvScopeRef, RepoRef, ResolvedEntry};
pub use service::{VaultLimits, VaultService};
