//! fkst-control-plane library crate (formerly fkst-hosted-api).
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod auth;
pub mod authz;
pub mod config;
// The engine integration was extracted to the `fkst-engine` crate (issue #151)
// so both the control-plane and the worker can drive it. Re-exported here under
// the same `engine` name so every existing `crate::engine::*` /
// `fkst_control_plane::engine::*` path keeps resolving unchanged.
pub mod engine;
pub mod error;
pub mod github_app;
pub mod github_hub;
pub mod goals;
// Role-neutral leaves extracted to `fkst-shared` (issue #145). Re-exported here
// so every existing `crate::{models,nyxid}::…` path and test still resolves.
pub mod models;
pub mod nyxid;
pub mod protocol;
// `ornn` and `vault` are split modules: their role-neutral leaves
// (`ornn::types`, `vault::model`) live in `fkst-shared`; the control-plane
// halves (the client/injector, the in-memory vault service) stay here.
// Runtime OpenAPI 3 document (no static spec): assembled from the live
// `#[utoipa::path]` handlers + `ToSchema` types and served at GET /openapi.json.
pub mod openapi;
pub mod ornn;
pub mod reconcile;
pub mod router;
pub mod routes;
pub mod sessions;
pub mod startup;
pub mod state;
pub mod vault;
