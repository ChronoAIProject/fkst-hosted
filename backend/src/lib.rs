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
pub mod goals;
// Role-neutral leaves extracted to `fkst-shared` (issue #145). Re-exported here
// so every existing `crate::{models,nyxid}::…` path and test still resolves.
pub mod models;
pub mod nyxid;
pub mod protocol;
// Runtime OpenAPI 3 document (no static spec): assembled from the live
// `#[utoipa::path]` handlers + `ToSchema` types and served at GET /openapi.json.
pub mod k8s;
pub mod nyxid_connect;
pub mod openapi;
pub mod router;
pub mod routes;
// In-pod `run-session` subcommand (milestone #9): drives ONE substrate session
// to completion in a Kubernetes Job, mapping the engine's terminal status onto
// the process exit code. No ClaimMap/CAS, no `/internal/v1`, no heartbeat.
pub mod runner;
pub mod session_spec;
pub mod sessions;
pub mod startup;
pub mod state;
