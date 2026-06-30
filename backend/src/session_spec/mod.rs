//! The pod-per-session handoff contract (milestone #9).
//!
//! Pod-per-session execution runs one Kubernetes Job per substrate session. The
//! control plane writes two things into the pod and a `run-session` subcommand
//! reads them:
//!
//! - [`spec::SessionSpec`] — the NON-SECRET descriptor of *what* to run (repo,
//!   issue, goal prompt, packages, Ornn pins, the deterministic session id +
//!   log branch). It holds no credentials, so a `{:?}` of it can never leak a
//!   token.
//! - [`creds::CredsLayout`] — the on-disk file layout of *the credentials*,
//!   mounted into the pod as a 0400 Kubernetes Secret volume (the GitHub App
//!   token, the NyxID session token, the NyxID base URL). The control plane
//!   writes the files; the pod reads them and rotates the NyxID token in place.
//!
//! This module is pure types/contract — no behaviour. The writer (the Job +
//! Secret launcher) and the reader (the run-session subcommand) both build on
//! it so they can never disagree on shape or path.

pub mod creds;
pub mod spec;

pub use creds::CredsLayout;
pub use spec::{derive_session_id, SessionGoal, SessionSpec};
