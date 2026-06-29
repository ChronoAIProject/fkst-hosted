//! Kubernetes integration for pod-per-session execution (milestone #9).
//!
//! The control plane spawns one Kubernetes Job per substrate session and
//! watches it to completion. This module owns the API client; the Job/Secret
//! launcher and the watcher build on it in later issues. It is inert unless
//! `FKST_POD_DISPATCH=true` — the control plane is Kubernetes-free by default.

pub mod client;
pub mod launcher;
pub mod refresh;

pub use client::{KubeClient, KubeError};
pub use launcher::{LaunchError, LaunchOutcome, PodSessionLauncher, SessionSecrets};
pub use refresh::{NyxidRefresh, RefreshError, SessionSecretWriter, NYXID_REFRESH_INTERVAL};
