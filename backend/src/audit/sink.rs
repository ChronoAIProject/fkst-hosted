//! The [`AuditSink`] boundary: where an audited request stops being an HTTP
//! concern and becomes a delivery concern.
//!
//! The boundary exists so the destination can be swapped without touching a
//! single handler. Today the production implementation is the direct PostHog
//! worker ([`super::worker`]); the durable relay (`required` delivery mode)
//! lands behind this same trait, so nothing here may assume "the destination is
//! PostHog" or "delivery is best-effort".
//!
//! Two rules shape the interface:
//!
//! - **`submit` never awaits and never blocks.** Audit pressure must not become
//!   product latency, so admission is a bounded, non-blocking hand-off; overflow
//!   is an explicit error, not a wait.
//! - **`drain` is bounded and reports what it could not deliver.** Shutdown must
//!   stop admission, flush what it can inside the configured deadline, and state
//!   the residue — a graceful shutdown that silently discards records would make
//!   the audit trail untrustworthy exactly when it matters most.

use std::sync::{Arc, Mutex};

use super::event::ApiRequestCompletedV1;

/// Why an event could not be admitted.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SubmitError {
    /// The bounded queue is full. The event is dropped; the product request is
    /// never delayed by audit backpressure.
    #[error("the audit queue is full")]
    QueueFull,
    /// Admission has closed because the process is shutting down.
    #[error("the audit sink is shutting down")]
    ShuttingDown,
}

/// What a graceful drain achieved.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct DrainReport {
    /// Events still queued or unsent when the drain deadline elapsed. `0` means
    /// everything admitted was handed to the destination.
    pub remaining: u64,
}

/// A destination for completed audit records.
#[async_trait::async_trait]
pub trait AuditSink: Send + Sync + std::fmt::Debug {
    /// Non-blocking admission of one completed record.
    fn submit(&self, event: ApiRequestCompletedV1) -> Result<(), SubmitError>;

    /// Current bounded-queue depth (0 for sinks without a queue).
    fn queue_depth(&self) -> u64 {
        0
    }

    /// Whether this sink actually delivers anywhere. `false` for the disabled
    /// no-op sink, which lets the caller record the `disabled` admission result
    /// instead of a spurious `accepted`.
    fn is_delivering(&self) -> bool;

    /// Stop admission and flush within the sink's configured deadline.
    async fn drain(&self) -> DrainReport;
}

/// The no-op sink installed when `FKST_POSTHOG_ENABLED` is false.
///
/// It accepts and discards, makes no network call, starts no task, and allocates
/// nothing per event — so a deployment with auditing off behaves exactly as it
/// did before the feature existed.
#[derive(Clone, Copy, Debug, Default)]
pub struct DisabledSink;

#[async_trait::async_trait]
impl AuditSink for DisabledSink {
    fn submit(&self, _event: ApiRequestCompletedV1) -> Result<(), SubmitError> {
        Ok(())
    }

    fn is_delivering(&self) -> bool {
        false
    }

    async fn drain(&self) -> DrainReport {
        DrainReport::default()
    }
}

/// An in-memory sink that keeps every admitted record, for tests of the layers
/// ABOVE this boundary (middleware, argument extraction, coverage guards).
///
/// It is deliberately part of the shipped crate rather than a `#[cfg(test)]`
/// helper: sibling modules and integration tests both need it, and a test-only
/// implementation cannot be shared across crate test binaries.
#[derive(Clone, Debug)]
pub struct RecordingSink {
    events: Arc<Mutex<Vec<ApiRequestCompletedV1>>>,
    /// Bounded like the real queue, so overflow behaviour is testable.
    capacity: usize,
}

impl RecordingSink {
    /// A recording sink holding at most `capacity` events.
    pub fn new(capacity: usize) -> Self {
        Self {
            events: Arc::new(Mutex::new(Vec::new())),
            capacity: capacity.max(1),
        }
    }

    /// Every event admitted so far, in submission order.
    pub fn events(&self) -> Vec<ApiRequestCompletedV1> {
        self.lock().clone()
    }

    /// How many events were admitted.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// A poisoned mutex only means a test thread panicked while holding it; the
    /// recorded events are still readable, so recover rather than double-panic.
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<ApiRequestCompletedV1>> {
        self.events.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl Default for RecordingSink {
    fn default() -> Self {
        Self::new(1_024)
    }
}

#[async_trait::async_trait]
impl AuditSink for RecordingSink {
    fn submit(&self, event: ApiRequestCompletedV1) -> Result<(), SubmitError> {
        let mut events = self.lock();
        if events.len() >= self.capacity {
            return Err(SubmitError::QueueFull);
        }
        events.push(event);
        Ok(())
    }

    fn queue_depth(&self) -> u64 {
        u64::try_from(self.len()).unwrap_or(u64::MAX)
    }

    fn is_delivering(&self) -> bool {
        true
    }

    async fn drain(&self) -> DrainReport {
        DrainReport::default()
    }
}

#[cfg(test)]
#[path = "sink_tests.rs"]
mod tests;
