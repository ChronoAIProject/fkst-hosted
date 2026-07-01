//! Kubernetes integration for the Model B pod-per-session reconciler (issue #359).
//!
//! The control plane runs one long-lived Pod per substrate session and reconciles
//! it against the repo's open trigger issues. This module owns the API client; the
//! session-Pod/Secret builders and the token-rotation loop build on it. It is inert
//! unless `FKST_POD_DISPATCH=true` — the control plane is Kubernetes-free by default.

pub mod client;
pub mod env_store;
pub mod env_validator;
pub(crate) mod isolation;
pub mod session_launcher;
// Model B (issue #359 §5.4, PR5b): the in-place per-session installation-token
// rotation loop that keeps a long-lived session pod's mounted `github-token`
// current (server-side patch of the per-session Secret). Gated on pod dispatch.
pub mod token_rotation;

pub use client::{KubeClient, KubeError};
pub use session_launcher::{
    build_session_pod, build_session_secret, create_session_pod, session_github_token_json,
    session_object_name, LaunchError, SessionPodOutcome, SessionPodSpec,
};
pub use token_rotation::run_token_rotation_loop;
