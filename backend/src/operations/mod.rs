//! The scoped historical-activity query (`GET /api/v1/operations/activity`).
//!
//! ```text
//! AuthenticatedViewer            [session_access]  verified identity + role
//!   -> ViewerScope               [session_access]  server-resolved, sealed
//!   -> SessionCapability gate    [session_access]  one exact lifecycle session
//!        -> ActivityVisibilityConstraint            the mandatory predicate
//!             -> SourceQuery     [source]           + fixed filters + keyset
//!                  -> fixed HogQL          [hogql]     predicate BEFORE LIMIT
//!                  -> relay SQL            (#5678)     the same constraint
//!             -> merge / dedupe / page   [merge]
//!                  -> ActivityPage        [routes::operations]
//! ```
//!
//! Module split and why it is this split:
//!
//! - [`filters`] owns the closed filter vocabulary and its normalization, so the
//!   audit record, the source predicate, and the cursor digest all describe the
//!   same query;
//! - [`hogql`] owns the query TEXT and nothing else — it is the module where "a
//!   request cannot contribute one character to the query" is true or false;
//! - [`source`] is the boundary the durable relay plugs into (issue #5678),
//!   taking the SAME typed constraint, so "did this source get the right
//!   predicate" is a type question;
//! - [`merge`] assembles a page from already-authorized candidates and performs
//!   no authorization of its own;
//! - [`cursor`] binds a page to its query, so a cursor from another
//!   viewer/scope/session/filter is refused rather than silently reset;
//! - [`limits`] and [`metrics`] keep the endpoint bounded and observable without
//!   a single identity-bearing label (epic `OPS-04`).
//!
//! The control plane stays stateless: nothing here caches a result, remembers a
//! caller, or persists a page. PostHog remains the historical projection and the
//! relay remains a delivery outbox (epic `OPS-03`).

use std::sync::Arc;

pub mod config;
pub mod cursor;
pub mod filters;
pub mod hogql;
pub mod limits;
pub mod merge;
pub mod metrics;
pub mod posthog;
pub mod record;
/// The durable relay's implementation of the source boundary (issue #5678):
/// scoped SQL behind an internal HTTP call, carrying the SAME typed constraint.
pub mod relay;
pub mod rows;
/// The row-authorized live sandbox inventory (issue #5675). Independent of the
/// activity query by design: a PostHog outage must never hide live runtime state,
/// and a runtime outage must never falsify history.
pub mod sandbox;
pub mod service;
pub mod source;

/// Shared, credential-free fixtures for this module's unit tests.
#[cfg(test)]
#[path = "test_support.rs"]
pub(crate) mod test_support;

pub use config::ActivityQueryConfig;
pub use cursor::{CursorBinding, CursorKey};
pub use filters::{ActivityFilters, RecordKind, StatusClass, TimeRange};
pub use limits::ActivityConcurrency;
pub use merge::{MergedPage, SourceHealth, SourceStatus};
pub use metrics::{ActivityMetrics, ActivityMetricsSnapshot, QueryResult, RejectionReason};
pub use record::{
    ActivityRecord, ActivitySourceKind, ApiRequestRecord, DeliveryState, SandboxLifecycleRecord,
};
pub use sandbox::{SandboxInventoryConfig, SandboxMetrics, SandboxMetricsSnapshot};
pub use service::{run, ActivityQueryRequest};
pub use source::{ActivitySource, SourceError, SourcePage, SourceQuery};

use crate::audit::AuditConfig;
use crate::error::AppError;

/// The operations-surface state carried on [`crate::state::AppState`].
///
/// The sources, their admission budget, and their telemetry travel together
/// because every operations route needs all three, and bundling them keeps the
/// application state from growing a field per counter — the same argument
/// [`crate::session_access::SessionAccessState`] makes.
#[derive(Clone, Debug, Default)]
pub struct OperationsState {
    /// The PostHog historical projection. `None` when the deployment configures
    /// no read credentials, which is what the endpoint's stable
    /// `503 audit_query_not_configured` is derived from.
    pub posthog: Option<Arc<dyn ActivitySource>>,
    /// The durable audit relay (issue #5678), populated by the startup wiring
    /// whenever the relay's READ half is configured — independently of the
    /// delivery mode, since a `best_effort` deployment still benefits from
    /// seeing its not-yet-verified, incomplete, and dead-letter rows. `None`
    /// leaves PostHog as the only source and the merge's partial-page semantics
    /// carry that fact to the caller.
    pub relay: Option<Arc<dyn ActivitySource>>,
    /// Bounded global + per-principal query admission.
    pub concurrency: ActivityConcurrency,
    /// Bounded, closed-label query telemetry rendered by `/metrics`.
    pub metrics: ActivityMetrics,
    /// Bounded, closed-label live-inventory telemetry rendered by `/metrics`
    /// (issue #5675). It sits beside the activity counters rather than in a
    /// separate state field for the same reason they do: every operations route
    /// needs the whole block, and one bundle keeps the application state from
    /// growing a field per counter family.
    pub sandbox_metrics: SandboxMetrics,
}

impl OperationsState {
    /// Build the state for a deployment, wiring the PostHog source when BOTH the
    /// capture host and the read credentials are configured.
    ///
    /// A missing read credential is not a startup failure: capture must keep
    /// working while an operator stages the query secret, and the endpoint's own
    /// `503 audit_query_not_configured` is the honest answer in the meantime.
    ///
    /// The REVERSE combination is a startup failure. `FKST_POSTHOG_HOST` is the
    /// one variable capture and this read path share, and it is easy to leave
    /// off a control plane that captures through the relay — at which point the
    /// deployment has a project id, a Query-Read-only key, and a permanently
    /// disabled activity API whose `503` is indistinguishable from an
    /// unconfigured key and which no alert covers. Failing the deploy that
    /// introduces it is the only place that mistake is cheap.
    pub fn from_config(audit: &AuditConfig, query: &ActivityQueryConfig) -> Result<Self, AppError> {
        let Some(host) = audit.host.as_deref() else {
            if query.is_configured() {
                return Err(AppError::Config(
                    "FKST_POSTHOG_HOST must be set when FKST_POSTHOG_PROJECT_ID and \
                     FKST_POSTHOG_QUERY_API_KEY are configured: the activity query reads \
                     the same host capture writes to, and without it /operations is \
                     permanently unavailable"
                        .to_string(),
                ));
            }
            tracing::info!("operations: activity query disabled (no FKST_POSTHOG_HOST)");
            return Ok(Self::default());
        };
        if !query.is_configured() {
            tracing::info!(
                "operations: activity query disabled (FKST_POSTHOG_PROJECT_ID / \
                 FKST_POSTHOG_QUERY_API_KEY not both set)"
            );
            return Ok(Self::default());
        }
        // The query resolves against FKST_POSTHOG_QUERY_HOST when set, else the
        // capture host. PostHog Cloud splits the two origins (#5813); a self-hosted
        // deployment leaves it unset and behaves exactly as before.
        let query_host = audit.query_host.as_deref().unwrap_or(host);
        let Some(url) = query.query_url(query_host) else {
            return Ok(Self::default());
        };
        let client = posthog::PosthogQueryClient::new(
            url,
            query.query_api_key.clone(),
            std::time::Duration::from_millis(query.query_timeout_ms),
        )?;
        tracing::info!(
            timeout_ms = query.query_timeout_ms,
            max_range_days = query.activity_max_range_days,
            "operations: activity query enabled"
        );
        Ok(Self {
            posthog: Some(Arc::new(posthog::PosthogActivitySource::new(client))),
            ..Self::default()
        })
    }

    /// The state with an explicit pair of sources (tests and the future relay).
    pub fn with_sources(
        posthog: Option<Arc<dyn ActivitySource>>,
        relay: Option<Arc<dyn ActivitySource>>,
    ) -> Self {
        Self {
            posthog,
            relay,
            ..Self::default()
        }
    }

    /// Whether any source can answer at all.
    pub fn is_configured(&self) -> bool {
        self.posthog.is_some() || self.relay.is_some()
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
