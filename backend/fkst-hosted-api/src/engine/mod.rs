//! Engine integration: the session runner that turns a stored fkst package
//! into a live `fkst-framework` process and back.
//!
//! This module is the ONLY code path in fkst-hosted that touches the engine,
//! and it does so strictly via the engine's CLI contract as pinned by the
//! issue #17 spike (`docs/spikes/issue-17-engine-host-contract.md`):
//! materialize a plain temp dir (no git), run the `conformance` pre-flight,
//! spawn `supervise` in its own process group, derive status from process
//! liveness + stderr ready markers, and stop via SIGTERM to the group.
//!
//! The runner is Mongo-agnostic and pure: callers load the package document,
//! build a [`PreparedPackage`], and persist whatever the runner returns.

pub mod config;
pub mod error;
pub mod materialize;

pub use config::EngineConfig;
pub use error::RunnerError;
pub use materialize::PreparedPackage;
