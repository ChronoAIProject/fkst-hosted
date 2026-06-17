//! fkst-shared: role-neutral types and transport clients shared by both the
//! control-plane and the worker deployables.
//!
//! This crate is the structural foundation of the control-plane/worker split
//! (issue #145): code that BOTH roles need lives here so neither role has to
//! depend on the other. It is intentionally free of the `mongodb` driver and
//! of `axum` — the worker links this crate, and keeping those out here is what
//! makes "the worker never touches the database" a compiler-enforced fact.
//!
//! What lives here:
//! - [`models`] — wire/domain document shapes (`RepoRef`, `SessionDoc`);
//!   `bson`-shaped but driver-free.
//! - [`nyxid`] — the NyxID credential-proxy transport client.
//! - [`llm`] — the `LlmGateway` seam and its NyxID-backed implementation.
//! - [`ornn`] — the Ornn pin DTOs (`types`); the on-disk injector and the
//!   `AppError`-coupled client stay control-plane for now.
//! - [`vault`] — the vault data model (`model`); the in-memory vault service
//!   stays control-plane.
//! - [`protocol`] — the internal controller<->worker wire types (#134).

pub mod llm;
pub mod models;
pub mod nyxid;
pub mod ornn;
pub mod protocol;
pub mod vault;
