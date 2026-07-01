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
//! [`codex`] renders the operator-pinned codex config and [`creds_helper`] owns
//! the in-pod git credential-helper wiring (both relocated here when the Model-A
//! `engine`/`sessions` modules were deleted).

mod codex;
mod creds_helper;
mod driver;
mod plan;

pub use driver::run_substrate_from_env;
