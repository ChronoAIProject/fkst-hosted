//! Sessions domain: the in-memory session store and the API-only bookkeeping
//! service that records session documents (it never runs an engine in-process;
//! pod-per-session execution is rebuilt in milestone #9).
//!
//! Layering:
//! - [`repo`] is the only writer of session documents; every status change
//!   funnels through its compare-and-swap [`repo::SessionRepo::transition`]
//!   so concurrent stop requests can never clobber each other.
//! - The document shape itself ([`crate::models::SessionDoc`]) is owned by
//!   the shared models module; this module owns behavior.

pub mod codex_provider;
pub mod nyxid_token;
pub mod repo;
pub mod service;

pub use repo::SessionRepo;
pub use service::{GoalTriggerInfo, GoalTriggerResult, SessionOwner, SessionService};
