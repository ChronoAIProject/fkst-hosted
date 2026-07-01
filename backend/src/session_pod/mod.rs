//! The Model B in-pod `run-substrate` entrypoint (issue #359 §5).
//!
//! A substrate SESSION is served by one long-lived, hard-isolated Pod whose
//! container runs the control-plane image with the `run-substrate` arg (built by
//! `k8s::session_launcher`). This module is that entrypoint: from the injected
//! `FKST_*` env + the mounted rotating creds it fetches the workspace packages +
//! the target repo, wires the rotating GitHub token into BOTH `git` (a credential
//! helper) and `gh` (a PATH shim), renders the codex config, and execs
//! `fkst-framework supervise` — forwarding SIGTERM so a reconciler pod-delete
//! drains supervise gracefully.
//!
//! The launch DECISIONS live in [`plan`] (pure, unit-tested); [`driver`] is the
//! thin effectful shell whose full correctness is verified on a live cluster.
//!
//! This is ADDITIVE (issue #359 PR4): nothing dispatches `run-substrate` as the
//! default path yet, and Model A (`crate::runner`'s `run-session`) is untouched.

mod driver;
mod plan;

pub use driver::run_substrate_from_env;
