//! The audit pipeline: a versioned event contract plus a swappable delivery sink.
//!
//! ```text
//! product request
//!   -> (later) outer audit middleware
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
//! - [`metrics`] keeps delivery telemetry closed-enum-labelled (epic `OPS-04`).
//!
//! This issue deliberately implements the contract and the delivery path only:
//! there is no request middleware, no argument extraction, no query API, and no
//! durable relay yet — each is a separate issue that plugs into these seams.

use std::sync::Arc;

use crate::error::AppError;

pub mod config;
pub mod event;
pub mod identity;
pub mod metrics;
pub mod posthog;
pub mod projection;
pub mod sink;
pub mod validate;
pub mod worker;

/// Shared fixtures for this module's unit tests (never compiled into the binary).
#[cfg(test)]
#[path = "test_support.rs"]
mod test_support;

pub use config::AuditConfig;
pub use event::{
    Actor, ActorKind, ApiRequestCompletedV1, ArgumentsParseStatus, AuditOutcome,
    AuthenticationMethod, Correlation, Principal, PrincipalKind, RequestIdentity, RequestResult,
    RequestTiming, ServiceIdentity,
};
pub use identity::{AuditActor, AuditIdentity, AuditIdentitySlot, AuditPrincipal};
pub use metrics::{AuditMetrics, AuditMetricsSnapshot};
pub use projection::{CaptureEvent, EventLimits};
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
