//! fkst-worker library: the worker deployable's testable internals.
//!
//! The worker connects UP to the controller (`CONTROLLER_URL`), self-registers,
//! then heartbeats + pulls work over the internal protocol. It deliberately
//! links neither the `mongodb` driver nor `fkst-control-plane` — the
//! compiler-enforced boundary that keeps it the lean, database-free "hands" of
//! the control-plane/worker split.

pub mod agent;
pub mod config;
pub mod drain;
pub mod engine;
pub mod run;
pub mod server;

pub use agent::{AgentError, WorkerAgent};
pub use config::WorkerConfig;
pub use drain::{run_drain, DrainOutcome, DrainState};
pub use run::run_worker;
