//! Bounded telemetry for runtime attribution and sandbox lifecycle emission.
//!
//! Two series, both with CLOSED-ENUM labels only (epic `OPS-04`):
//!
//! ```text
//! fkst_runtime_identity_operations_total{backend,result}
//! fkst_sandbox_lifecycle_events_total{backend,action,result}
//! ```
//!
//! Session ids, runtime ids, repositories, creators, and trigger issues are
//! structured-log fields — never labels. Every label value comes from a Rust
//! enum with a fixed variant list, so the label sets are finite by construction
//! and the counters can live in fixed-size arrays indexed by variant. That is
//! also why there is no `HashMap` here: a map keyed by a runtime value is
//! exactly how an unbounded label sneaks in.
//!
//! The handle is a cheap `Arc` of atomics, cloned into the reconcile context and
//! read by `/metrics`; scraping can never perturb the reconciler.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use crate::audit::lifecycle::LifecycleAction;

use super::RuntimeBackendKind;

/// Outcome of one identity operation, as the `result` label.
///
/// The four [`RuntimeIdentityOutcome`](super::RuntimeIdentityOutcome) values
/// plus the two the outcome type cannot express: an attempt the gate declined to
/// make, and an attempt the backend failed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdentityOperationResult {
    Unchanged,
    Backfilled,
    Conflict,
    NotFound,
    /// The bounded suppression gate declined the attempt.
    Suppressed,
    /// The backend call failed (transport, permission, validation).
    Failed,
}

impl IdentityOperationResult {
    pub const ALL: [IdentityOperationResult; 6] = [
        IdentityOperationResult::Unchanged,
        IdentityOperationResult::Backfilled,
        IdentityOperationResult::Conflict,
        IdentityOperationResult::NotFound,
        IdentityOperationResult::Suppressed,
        IdentityOperationResult::Failed,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            IdentityOperationResult::Unchanged => "unchanged",
            IdentityOperationResult::Backfilled => "backfilled",
            IdentityOperationResult::Conflict => "conflict",
            IdentityOperationResult::NotFound => "not_found",
            IdentityOperationResult::Suppressed => "suppressed",
            IdentityOperationResult::Failed => "failed",
        }
    }

    fn index(self) -> usize {
        match self {
            IdentityOperationResult::Unchanged => 0,
            IdentityOperationResult::Backfilled => 1,
            IdentityOperationResult::Conflict => 2,
            IdentityOperationResult::NotFound => 3,
            IdentityOperationResult::Suppressed => 4,
            IdentityOperationResult::Failed => 5,
        }
    }
}

impl From<super::RuntimeIdentityOutcome> for IdentityOperationResult {
    fn from(outcome: super::RuntimeIdentityOutcome) -> Self {
        match outcome {
            super::RuntimeIdentityOutcome::Unchanged => IdentityOperationResult::Unchanged,
            super::RuntimeIdentityOutcome::Backfilled => IdentityOperationResult::Backfilled,
            super::RuntimeIdentityOutcome::Conflict => IdentityOperationResult::Conflict,
            super::RuntimeIdentityOutcome::NotFound => IdentityOperationResult::NotFound,
        }
    }
}

/// Whether a lifecycle event reached the audit sink.
///
/// `dropped` is not cosmetic: a lifecycle event lost to a full queue is a hole
/// in the transition history, and a hole nobody counts is a hole nobody fixes
/// (epic `AUD-06`).
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LifecycleEmitResult {
    Emitted,
    Dropped,
}

impl LifecycleEmitResult {
    pub const ALL: [LifecycleEmitResult; 2] =
        [LifecycleEmitResult::Emitted, LifecycleEmitResult::Dropped];

    pub fn as_str(self) -> &'static str {
        match self {
            LifecycleEmitResult::Emitted => "emitted",
            LifecycleEmitResult::Dropped => "dropped",
        }
    }

    fn index(self) -> usize {
        match self {
            LifecycleEmitResult::Emitted => 0,
            LifecycleEmitResult::Dropped => 1,
        }
    }
}

const BACKENDS: usize = RuntimeBackendKind::ALL.len();
const IDENTITY_RESULTS: usize = IdentityOperationResult::ALL.len();
const LIFECYCLE_ACTIONS: usize = LifecycleAction::ALL.len();
const LIFECYCLE_RESULTS: usize = LifecycleEmitResult::ALL.len();

#[derive(Debug, Default)]
struct Counters {
    identity: [[AtomicU64; IDENTITY_RESULTS]; BACKENDS],
    lifecycle: [[[AtomicU64; LIFECYCLE_RESULTS]; LIFECYCLE_ACTIONS]; BACKENDS],
}

/// Cheaply clonable writer/reader handle for both series.
#[derive(Clone, Debug, Default)]
pub struct RuntimeTelemetry {
    counters: Arc<Counters>,
}

impl RuntimeTelemetry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one identity operation.
    pub fn record_identity(&self, backend: RuntimeBackendKind, result: IdentityOperationResult) {
        self.counters.identity[backend.index()][result.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// Count one lifecycle-event emission attempt.
    pub fn record_lifecycle(
        &self,
        backend: RuntimeBackendKind,
        action: LifecycleAction,
        result: LifecycleEmitResult,
    ) {
        self.counters.lifecycle[backend.index()][action.index()][result.index()]
            .fetch_add(1, Ordering::Relaxed);
    }

    /// An immutable read projection for `/metrics`.
    pub fn snapshot(&self) -> RuntimeTelemetrySnapshot {
        let mut identity = [[0_u64; IDENTITY_RESULTS]; BACKENDS];
        let mut lifecycle = [[[0_u64; LIFECYCLE_RESULTS]; LIFECYCLE_ACTIONS]; BACKENDS];
        for backend in RuntimeBackendKind::ALL {
            for result in IdentityOperationResult::ALL {
                identity[backend.index()][result.index()] =
                    self.counters.identity[backend.index()][result.index()].load(Ordering::Relaxed);
            }
            for action in LifecycleAction::ALL {
                for result in LifecycleEmitResult::ALL {
                    lifecycle[backend.index()][action.index()][result.index()] =
                        self.counters.lifecycle[backend.index()][action.index()][result.index()]
                            .load(Ordering::Relaxed);
                }
            }
        }
        RuntimeTelemetrySnapshot {
            identity,
            lifecycle,
        }
    }
}

/// A consistent read projection of both counter families.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeTelemetrySnapshot {
    identity: [[u64; IDENTITY_RESULTS]; BACKENDS],
    lifecycle: [[[u64; LIFECYCLE_RESULTS]; LIFECYCLE_ACTIONS]; BACKENDS],
}

impl RuntimeTelemetrySnapshot {
    pub fn identity(&self, backend: RuntimeBackendKind, result: IdentityOperationResult) -> u64 {
        self.identity[backend.index()][result.index()]
    }

    pub fn lifecycle(
        &self,
        backend: RuntimeBackendKind,
        action: LifecycleAction,
        result: LifecycleEmitResult,
    ) -> u64 {
        self.lifecycle[backend.index()][action.index()][result.index()]
    }
}

impl Default for RuntimeTelemetrySnapshot {
    fn default() -> Self {
        RuntimeTelemetry::new().snapshot()
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
