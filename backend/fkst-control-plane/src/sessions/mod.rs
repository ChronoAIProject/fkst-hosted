//! Sessions domain: persistence (the `sessions` collection) and the
//! single-pod orchestration service that drives one engine process per
//! session document.
//!
//! Layering:
//! - [`repo`] is the only writer of session documents; every status change
//!   funnels through its compare-and-swap [`repo::SessionRepo::transition`]
//!   so concurrent stop requests and driver progress can never clobber each
//!   other.
//! - The document shape itself ([`crate::models::SessionDoc`]) is owned by
//!   the shared models module; this module owns behavior.

pub mod codex_provider;
pub mod dispatch;
pub mod nyxid_token;
pub mod redispatch;
pub mod repo;
pub mod service;

pub use redispatch::DispatchRedispatch;
pub use repo::SessionRepo;
pub use service::{GoalTriggerInfo, GoalTriggerResult, SessionOwner, SessionService};

// Controller-side dispatch resolution (#151), dormant until the activation
// increment wires it into placement. The resolver runs through
// [`SessionService::resolve_dispatch`]; its error type is surfaced here.
pub use dispatch::DispatchError;
