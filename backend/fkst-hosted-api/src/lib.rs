//! fkst-hosted-api library crate.
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod auth;
pub mod authz;
pub mod config;
pub mod db;
pub mod distribution;
pub mod engine;
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
