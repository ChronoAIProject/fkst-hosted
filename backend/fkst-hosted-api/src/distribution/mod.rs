//! Session distribution + redo-on-failover takeover: the scheduling and
//! recovery layer of the pool-manager.
//!
//! fkst-hosted runs at most one live engine session per package, on exactly
//! one pod, behind the `leases` collection ([`crate::leases`]). This module
//! decides *which* healthy pod runs a *new* session (least-loaded placement)
//! and *recovers* a session whose holder pod died by **redoing it from
//! scratch** on a healthy pod under a fresh fencing token — never letting two
//! pods run the same package at once. On pod loss there is no state
//! transfer: the redo re-runs the engine, which reads GitHub truth to skip
//! already-completed work (engine concern, not re-implemented here).
//!
//! # Deviations from the issue spec (consensus decisions)
//!
//! - **Lease atomicity owner**: the spec sketches raw `$$NOW`
//!   aggregation-pipeline `findOneAndUpdate` shapes inline in this layer.
//!   They are NOT re-implemented here — [`crate::leases::LeaseStore`] stays
//!   the single owner of lease atomicity (acquire / renew / release), and
//!   this module composes those primitives. Consequently every "now"
//!   comparison uses the **application clock** (`bson::DateTime::now()`),
//!   exactly as the landed lease store does, with
//!   `FKST_TAKEOVER_GRACE_SECS` absorbing bounded cross-pod clock skew
//!   instead of Mongo server time (`$$NOW`). Deployments assume NTP-synced
//!   nodes; the server-time variant remains the documented follow-up on the
//!   lease store itself.
//! - **Fencing tokens are relative-monotonic and equality-only**: the spec
//!   describes a token strictly monotonic across a package's *entire* lease
//!   history. The landed `#23` contract is monotonic only for the lifetime
//!   of a continuous lease *document* (release deletes the document and the
//!   next acquire restarts at 1), so consumers compare tokens by **equality
//!   only** — never by ordering. Takeover still installs a strictly greater
//!   token than the expired lease it supersedes (the document survives, the
//!   counter continues), which is what the redo guarantee needs.
//! - **Module path**: the spec places this under `pool_manager/distribution`;
//!   it lives at `distribution/` beside the sibling `leases/` module (the
//!   crate has no `pool_manager` umbrella module).
//!
//! # Health posture (v1)
//!
//! The production `HealthView` is `SelfOnlyHealth`: the deployment is
//! single-replica, so the healthy-pod set is exactly this pod. The trait
//! seam is the integration point for the real pod registry / heartbeat
//! source of truth (`pm-health`), which is downstream work.

pub mod config;

pub use config::DistributionConfig;
