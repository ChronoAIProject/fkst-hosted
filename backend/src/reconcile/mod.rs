//! Model B reconciler core (issue #359 §4.3, PR5a).
//!
//! The PURE, cluster-free half of the reconciler: the desired-state types + the
//! event→action planner ([`desired`]) and the trigger-issue → registration parse
//! ([`registry`]). Nothing here performs Kubernetes or GitHub I/O — the effectful
//! `reconcile_repo` loop + the action executor are PR5b and live elsewhere.

pub mod desired;
pub mod registry;

pub use desired::{
    config_hash, plan_repo, KillReason, LivePod, PodLiveness, ReconcileAction, SessionDef,
    SessionRegistration,
};
pub use registry::parse_registration;
