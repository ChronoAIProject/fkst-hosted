//! fkst-hosted-api library crate.
//!
//! Hosts the hosted backend's public modules (config, error, router, state,
//! routes) so both the binary entrypoint and the integration tests can build
//! the application without a real TCP bind.

pub mod config;
pub mod db;
pub mod distribution;
pub mod engine;
pub mod error;
pub mod journal;
pub mod leases;
pub mod models;
pub mod packages;
pub mod router;
pub mod routes;
pub mod sessions;
pub mod state;
