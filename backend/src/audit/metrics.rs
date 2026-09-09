//! Bounded delivery telemetry for the audit pipeline.
//!
//! Every series here carries CLOSED-ENUM labels only (epic `OPS-04`): actor,
//! session, repository, request, and event ids are structured-log fields, never
//! Prometheus labels — one high-cardinality label would turn an audit trail into
//! an unbounded time-series bill, and would re-expose exactly the identifiers the
//! read side authorizes.
//!
//! The handle is a cheap `Arc` of atomics, cloned into the worker, the sink, and
//! the HTTP state. `/metrics` renders an immutable [`AuditMetricsSnapshot`], so
//! scraping can never perturb delivery.
//!
//! Naming note: a PostHog capture `200` means **accepted by capture**, not proven
//! query-visible, so the success label is `accepted` everywhere — never
//! `delivered` or `persisted` (epic `AUD-07`).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Outcome of admitting one event to the pipeline.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EnqueueResult {
    /// Admitted to the bounded queue.
    Accepted,
    /// The bounded queue was full; the event was dropped rather than blocking
    /// the product request.
    Full,
    /// The pipeline is disabled; the event was intentionally not recorded.
    Disabled,
}

/// Outcome of one HTTP attempt, and of a whole batch.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeliveryResult {
    /// PostHog capture accepted the payload (not "query-visible").
    Accepted,
    /// A transient condition (network, `408`, `429`, `5xx`).
    Retryable,
    /// A permanent payload/authentication/configuration failure.
    Permanent,
}

/// Why an event never reached PostHog.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DropReason {
    /// The bounded admission queue was full.
    QueueFull,
    /// The record violated the event contract.
    Invalid,
    /// The projected event exceeded the configured size cap.
    Oversized,
    /// Retries were exhausted while the failure was still retryable.
    Retryable,
    /// A permanent delivery failure.
    Permanent,
    /// Admission was closed, or the drain deadline elapsed with events queued.
    Shutdown,
}

impl EnqueueResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Full => "full",
            Self::Disabled => "disabled",
        }
    }
}

impl DeliveryResult {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Retryable => "retryable",
            Self::Permanent => "permanent",
        }
    }
}

impl DropReason {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::QueueFull => "queue_full",
            Self::Invalid => "invalid",
            Self::Oversized => "oversized",
            Self::Retryable => "retryable",
            Self::Permanent => "permanent",
            Self::Shutdown => "shutdown",
        }
    }
}

/// Process-local counters. One atomic per closed-enum label value, so a read is
/// lock-free and a write cannot contend with the HTTP path.
#[derive(Debug, Default)]
struct Counters {
    queue_depth: AtomicU64,
    enqueued_accepted: AtomicU64,
    enqueued_full: AtomicU64,
    enqueued_disabled: AtomicU64,
    batches_accepted: AtomicU64,
    batches_retryable: AtomicU64,
    batches_permanent: AtomicU64,
    attempts_accepted: AtomicU64,
    attempts_retryable: AtomicU64,
    attempts_permanent: AtomicU64,
    /// Sum of attempt durations, in milliseconds (rendered as seconds).
    delivery_duration_millis: AtomicU64,
    delivery_duration_count: AtomicU64,
    dropped_queue_full: AtomicU64,
    dropped_invalid: AtomicU64,
    dropped_oversized: AtomicU64,
    dropped_retryable: AtomicU64,
    dropped_permanent: AtomicU64,
    dropped_shutdown: AtomicU64,
    shutdown_remaining: AtomicU64,
    context_conflicts: AtomicU64,
}

/// Cheaply clonable writer/reader handle.
#[derive(Clone, Debug, Default)]
pub struct AuditMetrics {
    counters: Arc<Counters>,
}

impl AuditMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Publish the current bounded queue depth (a gauge, so it is set, not added).
    pub fn set_queue_depth(&self, depth: u64) {
        self.counters.queue_depth.store(depth, Ordering::Relaxed);
    }

    pub fn record_enqueued(&self, result: EnqueueResult) {
        let counter = match result {
            EnqueueResult::Accepted => &self.counters.enqueued_accepted,
            EnqueueResult::Full => &self.counters.enqueued_full,
            EnqueueResult::Disabled => &self.counters.enqueued_disabled,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_batch(&self, result: DeliveryResult) {
        let counter = match result {
            DeliveryResult::Accepted => &self.counters.batches_accepted,
            DeliveryResult::Retryable => &self.counters.batches_retryable,
            DeliveryResult::Permanent => &self.counters.batches_permanent,
        };
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_delivery_attempt(&self, result: DeliveryResult, elapsed: Duration) {
        let counter = match result {
            DeliveryResult::Accepted => &self.counters.attempts_accepted,
            DeliveryResult::Retryable => &self.counters.attempts_retryable,
            DeliveryResult::Permanent => &self.counters.attempts_permanent,
        };
        counter.fetch_add(1, Ordering::Relaxed);
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        self.counters
            .delivery_duration_millis
            .fetch_add(millis, Ordering::Relaxed);
        self.counters
            .delivery_duration_count
            .fetch_add(1, Ordering::Relaxed);
    }

    /// Record `count` events that will never reach PostHog. Never silent: the
    /// caller also emits a structured log naming the bounded reason.
    pub fn record_dropped(&self, reason: DropReason, count: u64) {
        let counter = match reason {
            DropReason::QueueFull => &self.counters.dropped_queue_full,
            DropReason::Invalid => &self.counters.dropped_invalid,
            DropReason::Oversized => &self.counters.dropped_oversized,
            DropReason::Retryable => &self.counters.dropped_retryable,
            DropReason::Permanent => &self.counters.dropped_permanent,
            DropReason::Shutdown => &self.counters.dropped_shutdown,
        };
        counter.fetch_add(count, Ordering::Relaxed);
    }

    /// Record conflicting writes to a request's audit context.
    ///
    /// A conflict means two places each believed they owned one field, which is
    /// a programmer error rather than a runtime condition: the request still
    /// succeeds with the first verified value, and this counter (plus the
    /// field-named log the slot emits) is how the mistake becomes visible.
    /// Unlabelled on purpose — the field name is high-signal but would be an
    /// unbounded label as the argument contract grows (epic `OPS-04`).
    pub fn record_context_conflicts(&self, count: u64) {
        self.counters
            .context_conflicts
            .fetch_add(count, Ordering::Relaxed);
    }

    /// Publish how many events were still queued when the drain deadline expired.
    pub fn set_shutdown_remaining(&self, remaining: u64) {
        self.counters
            .shutdown_remaining
            .store(remaining, Ordering::Relaxed);
    }

    /// An immutable read projection for `/metrics`.
    pub fn snapshot(&self) -> AuditMetricsSnapshot {
        let c = &self.counters;
        let load = |value: &AtomicU64| value.load(Ordering::Relaxed);
        AuditMetricsSnapshot {
            queue_depth: load(&c.queue_depth),
            enqueued_accepted: load(&c.enqueued_accepted),
            enqueued_full: load(&c.enqueued_full),
            enqueued_disabled: load(&c.enqueued_disabled),
            batches_accepted: load(&c.batches_accepted),
            batches_retryable: load(&c.batches_retryable),
            batches_permanent: load(&c.batches_permanent),
            attempts_accepted: load(&c.attempts_accepted),
            attempts_retryable: load(&c.attempts_retryable),
            attempts_permanent: load(&c.attempts_permanent),
            delivery_duration_seconds_sum: load(&c.delivery_duration_millis) as f64 / 1000.0,
            delivery_duration_count: load(&c.delivery_duration_count),
            dropped_queue_full: load(&c.dropped_queue_full),
            dropped_invalid: load(&c.dropped_invalid),
            dropped_oversized: load(&c.dropped_oversized),
            dropped_retryable: load(&c.dropped_retryable),
            dropped_permanent: load(&c.dropped_permanent),
            dropped_shutdown: load(&c.dropped_shutdown),
            shutdown_remaining: load(&c.shutdown_remaining),
            context_conflicts: load(&c.context_conflicts),
        }
    }
}

/// A consistent read projection of the audit delivery counters.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AuditMetricsSnapshot {
    pub queue_depth: u64,
    pub enqueued_accepted: u64,
    pub enqueued_full: u64,
    pub enqueued_disabled: u64,
    pub batches_accepted: u64,
    pub batches_retryable: u64,
    pub batches_permanent: u64,
    pub attempts_accepted: u64,
    pub attempts_retryable: u64,
    pub attempts_permanent: u64,
    pub delivery_duration_seconds_sum: f64,
    pub delivery_duration_count: u64,
    pub dropped_queue_full: u64,
    pub dropped_invalid: u64,
    pub dropped_oversized: u64,
    pub dropped_retryable: u64,
    pub dropped_permanent: u64,
    pub dropped_shutdown: u64,
    pub shutdown_remaining: u64,
    pub context_conflicts: u64,
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
