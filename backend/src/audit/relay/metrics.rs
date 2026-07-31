//! Control-plane telemetry for the relay conversation.
//!
//! ```text
//! fkst_audit_relay_requests_total{phase,result}
//! fkst_audit_relay_request_duration_seconds{phase,result}   (sum + count)
//! fkst_audit_required_rejections_total{reason}
//! ```
//!
//! Every label is a closed Rust enum (epic `OPS-04`). The rejection counter is
//! the emergency series: it counts requests the deployment REFUSED because it
//! could not promise to record them, and requests whose handler ran but whose
//! outcome could not be confirmed durable. Those two are the only honest ways
//! `required` mode can fail, and neither can itself be durably recorded — which
//! is exactly why they need a metric and a log rather than an audit event.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Which leg of the relay conversation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayPhase {
    /// The pre-handler durable acknowledgement.
    Start,
    /// The terminal event commit.
    Completion,
    /// A sandbox lifecycle transition.
    Lifecycle,
    /// The scoped read behind the activity query.
    Read,
}

impl RelayPhase {
    pub const ALL: [RelayPhase; 4] = [
        RelayPhase::Start,
        RelayPhase::Completion,
        RelayPhase::Lifecycle,
        RelayPhase::Read,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RelayPhase::Start => "start",
            RelayPhase::Completion => "completion",
            RelayPhase::Lifecycle => "lifecycle",
            RelayPhase::Read => "read",
        }
    }

    fn index(self) -> usize {
        match self {
            RelayPhase::Start => 0,
            RelayPhase::Completion => 1,
            RelayPhase::Lifecycle => 2,
            RelayPhase::Read => 3,
        }
    }
}

/// How one relay call ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RelayCallResult {
    /// The relay committed and acknowledged.
    Ack,
    /// The event id is already durable with different content.
    Conflict,
    /// The relay refused the body.
    Rejected,
    /// The relay could not be reached, or could not commit.
    Unavailable,
}

impl RelayCallResult {
    pub const ALL: [RelayCallResult; 4] = [
        RelayCallResult::Ack,
        RelayCallResult::Conflict,
        RelayCallResult::Rejected,
        RelayCallResult::Unavailable,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RelayCallResult::Ack => "ack",
            RelayCallResult::Conflict => "conflict",
            RelayCallResult::Rejected => "rejected",
            RelayCallResult::Unavailable => "unavailable",
        }
    }

    fn index(self) -> usize {
        match self {
            RelayCallResult::Ack => 0,
            RelayCallResult::Conflict => 1,
            RelayCallResult::Rejected => 2,
            RelayCallResult::Unavailable => 3,
        }
    }
}

/// Why a `required`-mode request was refused or could not be confirmed.
///
/// The two `*_conflict` reasons are separated from the two outage reasons on
/// purpose: an outage says "the relay could not answer", while a conflict says
/// "the relay answered, and what it holds for this event id is NOT what this
/// process built". The second is an event-id collision or a request that
/// outlived its completion deadline and was already closed as `incomplete` — a
/// different alert with a different remedy, so it gets its own series rather
/// than hiding inside the outage counter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RequiredRejection {
    /// The start could not be made durable, so the handler never ran.
    IngressUnavailable,
    /// A DIFFERENT start is already durable under this event id, so the handler
    /// never ran.
    IngressConflict,
    /// The handler ran, but its terminal event could not be confirmed durable.
    CompletionUnconfirmed,
    /// The handler ran, and the relay proved a different terminal projection is
    /// already durable under this event id.
    CompletionConflict,
}

impl RequiredRejection {
    pub const ALL: [RequiredRejection; 4] = [
        RequiredRejection::IngressUnavailable,
        RequiredRejection::IngressConflict,
        RequiredRejection::CompletionUnconfirmed,
        RequiredRejection::CompletionConflict,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            RequiredRejection::IngressUnavailable => "audit_ingress_unavailable",
            RequiredRejection::IngressConflict => "audit_ingress_conflict",
            RequiredRejection::CompletionUnconfirmed => "audit_completion_unconfirmed",
            RequiredRejection::CompletionConflict => "audit_completion_conflict",
        }
    }

    fn index(self) -> usize {
        match self {
            RequiredRejection::IngressUnavailable => 0,
            RequiredRejection::IngressConflict => 1,
            RequiredRejection::CompletionUnconfirmed => 2,
            RequiredRejection::CompletionConflict => 3,
        }
    }
}

#[derive(Debug, Default)]
struct Counters {
    calls: [[AtomicU64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()],
    duration_millis: [[AtomicU64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()],
    rejections: [AtomicU64; RequiredRejection::ALL.len()],
}

/// The cloneable handle carried by the middleware and the activity source.
#[derive(Clone, Debug, Default)]
pub struct RelayClientMetrics {
    counters: Arc<Counters>,
}

impl RelayClientMetrics {
    pub fn new() -> Self {
        Self::default()
    }

    /// Count one relay call and its duration.
    pub fn record_call(&self, phase: RelayPhase, result: RelayCallResult, elapsed: Duration) {
        self.counters.calls[phase.index()][result.index()].fetch_add(1, Ordering::Relaxed);
        self.counters.duration_millis[phase.index()][result.index()].fetch_add(
            u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX),
            Ordering::Relaxed,
        );
    }

    /// Count one `required`-mode refusal.
    pub fn record_rejection(&self, reason: RequiredRejection) {
        self.counters.rejections[reason.index()].fetch_add(1, Ordering::Relaxed);
    }

    /// A consistent read for `/metrics`.
    pub fn snapshot(&self) -> RelayClientMetricsSnapshot {
        let mut calls = [[0u64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()];
        let mut duration_millis = [[0u64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()];
        for phase in RelayPhase::ALL {
            for result in RelayCallResult::ALL {
                calls[phase.index()][result.index()] =
                    self.counters.calls[phase.index()][result.index()].load(Ordering::Relaxed);
                duration_millis[phase.index()][result.index()] = self.counters.duration_millis
                    [phase.index()][result.index()]
                .load(Ordering::Relaxed);
            }
        }
        let mut rejections = [0u64; RequiredRejection::ALL.len()];
        for reason in RequiredRejection::ALL {
            rejections[reason.index()] =
                self.counters.rejections[reason.index()].load(Ordering::Relaxed);
        }
        RelayClientMetricsSnapshot {
            calls,
            duration_millis,
            rejections,
        }
    }
}

/// An immutable projection for the exposition.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RelayClientMetricsSnapshot {
    calls: [[u64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()],
    duration_millis: [[u64; RelayCallResult::ALL.len()]; RelayPhase::ALL.len()],
    rejections: [u64; RequiredRejection::ALL.len()],
}

impl RelayClientMetricsSnapshot {
    pub fn calls(&self, phase: RelayPhase, result: RelayCallResult) -> u64 {
        self.calls[phase.index()][result.index()]
    }

    /// Total observed duration in SECONDS, the unit the series name declares.
    pub fn duration_seconds(&self, phase: RelayPhase, result: RelayCallResult) -> f64 {
        self.duration_millis[phase.index()][result.index()] as f64 / 1_000.0
    }

    pub fn rejections(&self, reason: RequiredRejection) -> u64 {
        self.rejections[reason.index()]
    }
}

#[cfg(test)]
#[path = "metrics_tests.rs"]
mod tests;
