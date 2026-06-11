//! Pool coordination core: per-package single-owner lease with a monotonic
//! fencing token.
//!
//! fkst-hosted runs engine sessions as child processes on pods. The engine is
//! single-instance by contract with zero cross-host fencing, so fkst-hosted
//! itself must guarantee at the orchestration layer that **at most one pod
//! owns a given package at any time**. This module is that mutual-exclusion
//! substrate: the lease lifecycle primitives (`acquire` / `renew` /
//! `release`) implemented as atomic conditional MongoDB updates over the
//! `leases` collection, plus a monotonic fencing token per package and a pod
//! identity concept. A pod must refuse to spawn `supervise` for a package
//! unless it currently holds that package's lease.
//!
//! # Deviations from the issue spec (consensus decisions)
//!
//! - **Module name**: the spec names this module `pool/`; it lives here as
//!   `leases/` because the module is the lease store and its types — the
//!   pool *manager* (heartbeat loop, process spawning) is downstream work
//!   and will get its own module.
//! - **Lease model**: the spec sketches a second `Lease` model with
//!   `session_id` stored as a *string* UUID. This module instead reuses the
//!   landed [`crate::models::LeaseDoc`], whose `session_id` is a
//!   [`bson::Uuid`] (BSON Binary subtype 4). Binary-on-both-sides is the
//!   clean join against `sessions._id` (also Binary subtype 4): a string on
//!   one side would silently never match `find_one({_id})` lookups. There is
//!   exactly one authoritative `leases` document shape in this crate.
//!
//! # MongoDB server requirement
//!
//! `acquire` derives the next fencing token with an aggregation-pipeline
//! update (`$add` / `$ifNull` over the existing document) so the bump is part
//! of the same atomic operation. Pipeline-style updates require **MongoDB
//! >= 4.2** (local dev and the integration tests pin `mongo:7`).
//!
//! # Fencing-token boundary (reset on release) and the equality-only rule
//!
//! `fencing_token` increases by exactly 1 on every successful `acquire`
//! (fresh insert, self-reacquire, or takeover of an expired lease) and never
//! changes on `renew`. `release` *deletes* the lease document, so the next
//! `acquire` starts a fresh document whose first token is `1`: monotonicity
//! is guaranteed only for the lifetime of a continuous lease document, NOT
//! globally across a release. Because the counter can restart, consumers
//! MUST compare fencing tokens by **equality only** (held token `==`
//! observed token), exactly as `renew` / `release` / `holds_current` do in
//! their filters. An ordering comparison (`>=` / `<=`) against a
//! post-release token would falsely treat a stale pre-release token as still
//! valid. This boundary is safe for equality-only consumers: a holder that
//! released has stopped acting, so no stale consumer can be confused by the
//! counter restarting.
//!
//! # Error mapping (deferred, documented intent)
//!
//! No HTTP endpoint surfaces leases in this issue, so [`PoolError`] is NOT
//! yet converted into [`crate::error::AppError`]. The intended mapping for
//! the downstream issue that wires leases into the API edge:
//! `PoolError::Config` -> `AppError::Config` and `PoolError::Mongo` ->
//! `AppError::Mongo` (lease contention is an outcome, not an error, and maps
//! to `409 Conflict` at the edge).

pub mod config;
pub mod error;
pub mod store;

pub use config::PoolConfig;
pub use error::PoolError;
pub use store::{AcquireOutcome, LeaseStore, ReleaseOutcome, RenewOutcome, IDX_LEASES_HOLDER_POD};
