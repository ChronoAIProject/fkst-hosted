//! Kubernetes integration for pod-per-session execution (milestone #9).
//!
//! The control plane spawns one Kubernetes Job per substrate session and
//! watches it to completion. This module owns the API client; the Job/Secret
//! launcher and the watcher build on it in later issues. It is inert unless
//! `FKST_POD_DISPATCH=true` — the control plane is Kubernetes-free by default.

pub mod client;
pub mod env_store;
pub mod env_validator;
pub(crate) mod isolation;
pub mod launcher;
pub mod session_launcher;
// Model B (issue #359 §5.4, PR5b): the in-place per-session installation-token
// rotation loop that keeps a long-lived session pod's mounted `github-token`
// current (server-side patch of the per-session Secret). Gated on pod dispatch.
pub mod token_rotation;
pub mod watch;

pub use client::{KubeClient, KubeError};
pub use launcher::{LaunchError, LaunchOutcome, PodSessionLauncher, SessionSecrets};
pub use session_launcher::{
    build_session_pod, build_session_secret, create_session_pod, session_github_token_json,
    session_object_name, SessionPodOutcome, SessionPodSpec,
};
pub use token_rotation::run_token_rotation_loop;
pub use watch::{job_disposition, JobDisposition, JobWatcher};
