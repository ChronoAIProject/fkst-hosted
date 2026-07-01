//! The pod-per-session handoff contract (Model B, issue #359).
//!
//! Two pure pieces the control plane and the in-pod `run-substrate` entrypoint
//! share so they can never disagree:
//!
//! - [`spec::derive_session_id`] — the deterministic per-session id the
//!   reconciler and the `fkst-sess-<id>` Pod/Secret name are keyed on.
//! - [`creds::CredsLayout`] — the on-disk file layout of *the credentials*,
//!   mounted into the pod as a 0400 Kubernetes Secret volume (the rotating GitHub
//!   App token and the static LLM API key). The control plane writes the files;
//!   the pod reads them.

pub mod creds;
pub mod spec;

pub use creds::CredsLayout;
pub use spec::derive_session_id;
