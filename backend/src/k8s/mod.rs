//! Kubernetes integration for the Model B pod-per-session reconciler (issue #359).
//!
//! The control plane runs one long-lived Pod per substrate session and reconciles
//! it against the repo's open trigger issues. This module owns the API client; the
//! session-Pod/Secret builders and the token-rotation loop build on it. It is inert
//! unless `FKST_POD_DISPATCH=true` — the control plane is Kubernetes-free by default.

pub mod client;
pub(crate) mod engine_env;
pub mod env_store;
pub mod env_validator;
// Pure builders for the env-validation Pod + its spec ConfigMap (issue #338), driven
// by the direct-Kubernetes backend's validation verb. Kept as a standalone module so
// the spec-shaping (which needs no cluster) stays testable in isolation.
pub(crate) mod env_validator_pod;
// Package-AGNOSTIC session-health signal: the pure evaluator ([`health_eval`]) and
// the scrape loop ([`health_scrape`]) that flags/clears a degraded session on its
// trigger issue (the recent-log read now lives on the session backend). Gated on pod
// dispatch.
pub mod health_eval;
pub mod health_scrape;
pub(crate) mod isolation;
pub mod session_launcher;
// Model B (issue #359 §5.4, PR5b): the in-place per-session installation-token
// rotation loop that keeps a long-lived session pod's mounted `github-token`
// current (server-side patch of the per-session Secret). Gated on pod dispatch.
pub mod token_rotation;

pub use client::{KubeClient, KubeError};
pub use health_scrape::run_health_scrape_loop;
pub use session_launcher::{
    build_session_pod, build_session_secret, create_session_pod, session_github_token_json,
    session_object_name, LaunchError, SessionPodOutcome, SessionPodSpec,
};
pub use token_rotation::run_token_rotation_loop;
