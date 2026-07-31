//! The audit pipeline: a versioned event contract plus a swappable delivery sink.
//!
//! ```text
//! product request
//!   -> outer audit middleware           [request]    one terminal record per request
//!   -> ApiRequestCompletedV1            [event]      the versioned contract
//!   -> AuditHandle::submit              [mod]        non-blocking admission
//!        -> AuditSink                   [sink]       the swappable boundary
//!             -> DisabledSink                        no-op, no worker, no network
//!             -> PostHogSink            [worker]     bounded queue + batching + retry
//!                  -> PostHogClient     [posthog]    public capture/batch API
//! ```
//!
//! Module split and why it is this split:
//!
//! - [`event`] owns the domain contract and nothing else, so the wire format can
//!   change without touching identity semantics;
//! - [`validate`] and [`projection`] separate "is this record legal" from "how
//!   does it look on the PostHog wire", because the first is an authorization and
//!   redaction concern and the second is a transport concern;
//! - [`sink`] is the seam the durable relay will replace, so no handler ever
//!   depends on PostHog;
//! - [`worker`] and [`posthog`] hold everything that can fail at runtime, behind
//!   bounded queues, bounded retries, and bounded logs;
//! - [`identity`] carries the credential-free actor/principal pair from whoever
//!   proved it to the middleware that writes the record, so no route has to
//!   invent its own notion of "who is calling";
//! - [`request`] owns the HTTP lifecycle — request-id normalization, the verified
//!   OpenAPI operation catalog, the per-request context, terminal-outcome
//!   derivation, and the outermost middleware — so no handler ever calls a sink;
//! - [`arguments`] owns the per-operation redaction boundary: one sealed, typed,
//!   allowlisted safe DTO per operation, so a record's `arguments` can only ever
//!   contain properties someone deliberately named;
//! - [`metrics`] keeps delivery telemetry closed-enum-labelled (epic `OPS-04`).
//!
//! Still separate issues plugging into these seams: the endpoint-specific safe
//! argument contract, the scoped query API, and the durable relay.

use std::sync::Arc;

use crate::error::AppError;

pub mod arguments;
pub mod config;
pub mod event;
pub mod identity;
pub mod metrics;
pub mod posthog;
pub mod projection;
pub mod request;
pub mod sink;
pub mod validate;
pub mod worker;

/// Shared fixtures for this module's unit tests (never compiled into the binary).
#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

pub use arguments::{BoundedAuditArguments, InvalidInput, SafeArgumentSpec, ToSafeAuditArguments};
pub use config::AuditConfig;
pub use event::{
    Actor, ActorKind, ApiRequestCompletedV1, ArgumentsParseStatus, AuditOutcome,
    AuthenticationMethod, Correlation, Principal, PrincipalKind, RequestIdentity, RequestResult,
    RequestTiming, ServiceIdentity,
};
pub use identity::{AuditActor, AuditIdentity, AuditIdentitySlot, AuditPrincipal};
pub use metrics::{AuditMetrics, AuditMetricsSnapshot};
pub use projection::{CaptureEvent, EventLimits};
pub use request::{
    audit_requests, AuditMiddleware, AuditRequestContext, CatalogError, OperationCatalog,
    OperationPolicy, SafeHttpSpan,
};
pub use sink::{AuditSink, DisabledSink, DrainReport, RecordingSink, SubmitError};
pub use validate::EventError;

/// The cloneable audit handle carried on the application state.
///
/// It owns the admission decision plus the telemetry, so the [`AuditSink`]
/// implementations stay pure transport and every sink gets identical, bounded
/// observability for free.
#[derive(Clone, Debug)]
pub struct AuditHandle {
    sink: Arc<dyn AuditSink>,
    metrics: AuditMetrics,
}

impl AuditHandle {
    /// Wrap an arbitrary sink (used by the constructors below and by tests).
    pub fn new(sink: Arc<dyn AuditSink>, metrics: AuditMetrics) -> Self {
        Self { sink, metrics }
    }

    /// The no-op handle: no worker, no network, no per-event allocation.
    pub fn disabled() -> Self {
        Self::new(Arc::new(DisabledSink), AuditMetrics::new())
    }

    /// A handle backed by an in-memory recorder, plus the recorder itself, for
    /// tests of the layers above the sink boundary.
    pub fn recording() -> (Self, RecordingSink) {
        let sink = RecordingSink::default();
        (Self::new(Arc::new(sink.clone()), AuditMetrics::new()), sink)
    }

    /// Build the configured handle, starting the delivery worker when the
    /// feature is enabled. A disabled deployment starts no task at all.
    pub fn from_config(config: &AuditConfig) -> Result<Self, AppError> {
        let metrics = AuditMetrics::new();
        if !config.enabled {
            tracing::info!("audit capture disabled (FKST_POSTHOG_ENABLED not set)");
            return Ok(Self::new(Arc::new(DisabledSink), metrics));
        }
        let client = posthog::PostHogClient::from_config(config)?;
        let sink = worker::spawn(config, client, metrics.clone());
        tracing::info!(
            environment = %config.environment,
            queue_capacity = config.queue_capacity,
            batch_size = config.batch_size,
            "audit capture enabled"
        );
        Ok(Self::new(Arc::new(sink), metrics))
    }

    /// Admit one completed record. Never blocks and never awaits.
    ///
    /// The error is returned so a caller may react, and is ALSO counted and
    /// logged here so an ignoring caller can never make a drop invisible.
    pub fn submit(&self, event: ApiRequestCompletedV1) -> Result<(), SubmitError> {
        match self.sink.submit(event) {
            Ok(()) => {
                // A disabled deployment reports `disabled` rather than a
                // spurious `accepted`, so a dashboard cannot mistake a switched
                // off pipeline for a healthy one.
                self.metrics.record_enqueued(if self.sink.is_delivering() {
                    metrics::EnqueueResult::Accepted
                } else {
                    metrics::EnqueueResult::Disabled
                });
                Ok(())
            }
            Err(SubmitError::QueueFull) => {
                self.metrics.record_enqueued(metrics::EnqueueResult::Full);
                self.metrics
                    .record_dropped(metrics::DropReason::QueueFull, 1);
                tracing::warn!("audit queue full; dropping the newest event");
                Err(SubmitError::QueueFull)
            }
            Err(SubmitError::ShuttingDown) => {
                self.metrics
                    .record_dropped(metrics::DropReason::Shutdown, 1);
                tracing::warn!("audit admission closed; dropping the event");
                Err(SubmitError::ShuttingDown)
            }
        }
    }

    /// Count conflicting writes to one request's audit context.
    ///
    /// The slot that rejected the write already logged the offending FIELD name;
    /// this is the bounded, unlabelled counter that makes the mistake visible on
    /// a dashboard without turning a field name into a Prometheus label.
    pub fn record_context_conflicts(&self, count: u64) {
        self.metrics.record_context_conflicts(count);
    }

    /// Whether events actually go anywhere.
    pub fn is_delivering(&self) -> bool {
        self.sink.is_delivering()
    }

    /// A consistent read projection for `/metrics`, with the queue gauge read
    /// straight from the sink so it is exact at scrape time.
    pub fn metrics_snapshot(&self) -> AuditMetricsSnapshot {
        let mut snapshot = self.metrics.snapshot();
        snapshot.queue_depth = self.sink.queue_depth();
        snapshot
    }

    /// Stop admission and flush within the configured deadline.
    pub async fn drain(&self) -> DrainReport {
        self.sink.drain().await
    }
}

impl Default for AuditHandle {
    fn default() -> Self {
        Self::disabled()
    }
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
