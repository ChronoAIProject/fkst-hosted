//! Bounded telemetry for the session-access projection and scope decisions.
//!
//! Every label is a closed enum (epic `OPS-04`). No actor id, login, session id,
//! repository, issue number, configured entry, or viewer value ever becomes a
//! Prometheus label — those are structured-log fields at most, and the denial log
//! deliberately carries only the reason, never the probed value.
//!
//! The counters are a fixed-size array indexed by [`ScopeOutcome`], so the series
//! set is decided at compile time: a future variant cannot accidentally introduce
//! unbounded cardinality.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use super::viewer::{RequestedScope, ScopeDenialReason, ScopeRequest, ViewerScope};

/// The exact `(scope, result, reason)` label triples that can occur.
///
/// Enumerated rather than composed, because most of the 2x2x4 cross product is
/// unreachable and exporting empty impossible series would only mislead.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScopeOutcome {
    /// Personal scope, resolved from an omitted parameter.
    MineDefault,
    /// Personal scope, explicitly requested.
    MineExplicit,
    /// Global scope, resolved from an omitted parameter (an administrator).
    AllDefault,
    /// Global scope, explicitly requested by an administrator.
    AllExplicit,
    /// A regular caller asked for the global scope.
    AllForbidden,
    /// A regular caller supplied a cross-user actor filter.
    CrossActorForbidden,
}

impl ScopeOutcome {
    /// How many distinct series this metric exports.
    pub const COUNT: usize = 6;

    /// Every variant, in exposition order.
    pub const ALL: [ScopeOutcome; Self::COUNT] = [
        ScopeOutcome::MineDefault,
        ScopeOutcome::MineExplicit,
        ScopeOutcome::AllDefault,
        ScopeOutcome::AllExplicit,
        ScopeOutcome::AllForbidden,
        ScopeOutcome::CrossActorForbidden,
    ];

    /// The `(scope, result, reason)` labels.
    pub fn labels(self) -> (&'static str, &'static str, &'static str) {
        match self {
            ScopeOutcome::MineDefault => ("mine", "allowed", "resolved_default"),
            ScopeOutcome::MineExplicit => ("mine", "allowed", "resolved_explicit"),
            ScopeOutcome::AllDefault => ("all", "allowed", "resolved_default"),
            ScopeOutcome::AllExplicit => ("all", "allowed", "resolved_explicit"),
            ScopeOutcome::AllForbidden => {
                ("all", "forbidden", ScopeDenialReason::GlobalScope.as_str())
            }
            ScopeOutcome::CrossActorForbidden => (
                "mine",
                "forbidden",
                ScopeDenialReason::CrossActorFilter.as_str(),
            ),
        }
    }

    /// Classify one resolution.
    pub fn of(request: ScopeRequest, resolved: &Result<ViewerScope, ScopeDenialReason>) -> Self {
        match resolved {
            Err(ScopeDenialReason::GlobalScope) => ScopeOutcome::AllForbidden,
            Err(ScopeDenialReason::CrossActorFilter) => ScopeOutcome::CrossActorForbidden,
            Ok(scope) => match (scope.is_global(), request.requested) {
                (true, Some(RequestedScope::Global)) => ScopeOutcome::AllExplicit,
                (true, _) => ScopeOutcome::AllDefault,
                (false, Some(RequestedScope::Personal)) => ScopeOutcome::MineExplicit,
                (false, _) => ScopeOutcome::MineDefault,
            },
        }
    }

    fn index(self) -> usize {
        match self {
            ScopeOutcome::MineDefault => 0,
            ScopeOutcome::MineExplicit => 1,
            ScopeOutcome::AllDefault => 2,
            ScopeOutcome::AllExplicit => 3,
            ScopeOutcome::AllForbidden => 4,
            ScopeOutcome::CrossActorForbidden => 5,
        }
    }
}

/// Process-local scope-decision counters. Cheap to clone; every clone shares one
/// backing store.
#[derive(Clone, Default)]
pub struct ScopeMetrics {
    counters: Arc<[AtomicU64; ScopeOutcome::COUNT]>,
}

impl ScopeMetrics {
    /// Fresh counters.
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one resolved (or refused) scope selection.
    pub fn record(&self, outcome: ScopeOutcome) {
        self.counters[outcome.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// A consistent read projection for `/metrics`.
    pub fn snapshot(&self) -> ScopeMetricsSnapshot {
        let mut counts = [0u64; ScopeOutcome::COUNT];
        for (slot, counter) in counts.iter_mut().zip(self.counters.iter()) {
            *slot = counter.load(Ordering::Relaxed);
        }
        ScopeMetricsSnapshot { counts }
    }
}

impl std::fmt::Debug for ScopeMetrics {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScopeMetrics")
            .field("outcomes", &ScopeOutcome::COUNT)
            .finish()
    }
}

/// An immutable copy of the scope counters.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct ScopeMetricsSnapshot {
    counts: [u64; ScopeOutcome::COUNT],
}

impl ScopeMetricsSnapshot {
    /// The count for one outcome.
    pub fn count(&self, outcome: ScopeOutcome) -> u64 {
        self.counts[outcome.index()]
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
