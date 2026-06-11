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

pub mod repo;
pub mod service;

pub use repo::SessionRepo;
pub use service::SessionService;
