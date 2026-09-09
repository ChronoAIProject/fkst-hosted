//! The row-authorized live sandbox inventory (issue #5675, epic `SBOX-01`..`06`).
//!
//! ```text
//! AuthenticatedViewer          [session_access]  verified identity + role
//!   -> ViewerScope             [session_access]  accessible | all, sealed
//!        -> SandboxFilters     [filters]         closed, exact, POST-authorization
//!        -> RowAuthorizer      [authorize]       one registry decision per row
//!             -> SessionBackend::list_runtime_inventory()   ONE backend list
//!                  -> service::run                          the normative order
//!                       -> order / warning / metrics
//!                            -> SandboxInventoryResponse    [routes::operations]
//! ```
//!
//! ## The one deliberate source-level exception
//!
//! Every other authorized surface in this milestone pushes the viewer predicate
//! INTO the source before its `LIMIT`. This one cannot: neither Kubernetes nor
//! the OpenSandbox lifecycle API can express "sessions this GitHub user created
//! or was granted", because that authority lives in GitHub issues. So the
//! complete managed fleet is read into THIS process and authorized here — and
//! the invariant that replaces source-side filtering is the ORDER in
//! [`service::run`]: authorization runs before user filters, before ordering,
//! before `item_count`, before warning projection, before the result ceiling, and
//! before serialization. Nothing derived from a hidden row can therefore reach a
//! response, a count, a warning, or a metric.
//!
//! ## Runtime metadata is never authority
//!
//! An annotation or a sandbox metadata value is writable by anyone with namespace
//! access. It is display and correlation data. The only session authority is the
//! GitHub-derived [`crate::session_access::SessionAccessRegistry`], and a runtime
//! with no session id, a malformed one, or no registry context is HIDDEN in
//! `accessible` and visible only to a verified global administrator in `all`.
//!
//! ## Module split
//!
//! - [`config`] — the two public ceilings and the bounded route budget;
//! - [`filters`] — the closed filter vocabulary, applied only after authorization;
//! - [`authorize`] — one registry + capability decision per row;
//! - [`order`] — the documented closed status ordering and its tie-breakers;
//! - [`warning`] — internal inventory warnings mapped to bounded public codes;
//! - [`metrics`] — closed-label telemetry with no per-requester series;
//! - [`service`] — the normative pipeline that ties them together.

pub mod authorize;
pub mod config;
pub mod filters;
pub mod metrics;
pub mod order;
pub mod service;
pub mod warning;

/// Shared, credential-free fixtures for this module's unit tests.
#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use authorize::RowAuthorizer;
pub use config::SandboxInventoryConfig;
pub use filters::SandboxFilters;
pub use metrics::{
    BackendLabel, InventoryResult, SandboxMetrics, SandboxMetricsSnapshot, SandboxRejectionReason,
    ScopeLabel,
};
pub use service::{run, AuthorizedInventory, AuthorizedRuntime, SandboxInventoryRequest};
pub use warning::SandboxWarningCode;
