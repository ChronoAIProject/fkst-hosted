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
pub mod goal_token;
pub mod logs;
pub mod materialize;
pub mod process;
pub mod runner;

pub use config::EngineConfig;
pub use error::RunnerError;
pub use goal_token::{
    git_config_entries, write_token_file, GitConfigEntry, HELPER_SCRIPT_NAME, MINT_NONCE_ENV,
    MINT_REQUEST_SUFFIX, NONCE_FILE_NAME, TOKEN_FILE_NAME,
};
pub use materialize::{materialize_helper_script, PreparedPackage};
pub use process::{is_pid_alive, GoalEnv};
pub use runner::{GoalContext, LiveStatus, RunningSession, SessionRunner, StartSpec};
