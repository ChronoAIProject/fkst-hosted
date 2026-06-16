//! fkst-control-plane library crate (formerly fkst-hosted-api).
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod auth;
pub mod authz;
pub mod config;
// Controller side of the internal worker protocol (#134): the in-memory worker
// registry + the shared-secret-guarded internal router.
pub mod controller;
pub mod db;
pub mod distribution;
// The engine integration was extracted to the `fkst-engine` crate (issue #151)
// so both the control-plane and the worker can drive it. Re-exported here under
// the same `engine` name so every existing `crate::engine::*` /
// `fkst_control_plane::engine::*` path keeps resolving unchanged.
pub use fkst_engine as engine;
pub mod error;
pub mod github_app;
pub mod github_hub;
pub mod goals;
pub mod journal;
pub mod leases;
// Role-neutral leaves extracted to `fkst-shared` (issue #145). Re-exported here
// so every existing `crate::{llm,models,nyxid}::…` path and test still resolves.
pub use fkst_shared::llm;
pub use fkst_shared::models;
pub use fkst_shared::nyxid;
// `ornn` and `vault` are split modules: their role-neutral leaves
// (`ornn::types`, `vault::model`) live in `fkst-shared`; the control-plane
// halves (the client/injector, the encrypting service + Mongo repo) stay here.
pub mod ornn;
pub mod reconcile;
pub mod router;
pub mod routes;
pub mod sessions;
pub mod state;
pub mod vault;
