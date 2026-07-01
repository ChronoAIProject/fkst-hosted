//! fkst-control-plane library crate (formerly fkst-hosted-api).
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod config;
// The engine integration was extracted to the `fkst-engine` crate (issue #151)
// so both the control-plane and the worker can drive it. Re-exported here under
// the same `engine` name so every existing `crate::engine::*` /
// `fkst_control_plane::engine::*` path keeps resolving unchanged.
pub mod engine;
// Named-environment / install-validation config knobs (`FKST_ENV_*`, issue
// #338 §6.1). Config surface only — no behaviour is wired to these yet.
pub mod env_config;
pub mod error;
pub mod github_app;
// GitHub-token identity verification + the `GithubUser` axum extractor (PR4a):
// trades `Authorization: Bearer <github token>` for the verified `{login, id}`
// that keys the per-user environment/secret store.
pub mod github_identity;
pub mod goals;
pub mod models;
pub mod protocol;
// Runtime OpenAPI 3 document (no static spec): assembled from the live
// `#[utoipa::path]` handlers + `ToSchema` types and served at GET /openapi.json.
pub mod k8s;
pub mod openapi;
pub mod router;
pub mod routes;
// In-pod `run-session` subcommand (milestone #9): drives ONE substrate session
// to completion in a Kubernetes Job, mapping the engine's terminal status onto
// the process exit code. No ClaimMap/CAS, no `/internal/v1`, no heartbeat.
pub mod runner;
pub mod session_spec;
pub mod sessions;
pub mod state;
