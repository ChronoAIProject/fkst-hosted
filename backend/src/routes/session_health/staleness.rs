//! The **heartbeat watchdog**: the pure decision behind a session's staleness verdict.
//!
//! A package running on the engine cannot be the watchdog for the engine that runs it
//! — if `supervise` wedges or dies, the cron raiser never fires and no report is ever
//! written. So the control plane treats *silence* as the signal: reports were declared
//! to arrive every `expected_interval_secs` and the newest is far older than that.
//!
//! # The liveness gate is the whole point
//!
//! Silence has two completely different causes:
//!
//! * the engine is wedged — a fault, and exactly what this watchdog exists to surface;
//! * the pod was reaped because there is no pending work — entirely normal, and the
//!   designed end of every session's life.
//!
//! Treating both as "stale" would put a false alarm on **every idle session in the
//! fleet**, which is worse than no signal at all. A verdict is therefore only computed
//! when a live runtime is observed; otherwise the answer is
//! [`StalenessState::NotRunning`], which *explains* the silence rather than alarming
//! about it.
//!
//! # Fail open
//!
//! Every uncertainty resolves toward "no alarm". A backend that cannot be reached, a
//! clock that disagrees, a report from the future — none of them may render to a user
//! as "your session is stuck", because a control-plane problem is not a session
//! problem.

use k8s_openapi::chrono::{DateTime, Utc};
use serde::Serialize;
use utoipa::ToSchema;

use crate::session_health::HealthIndexEntry;

/// How many declared intervals may elapse before silence is a fault.
///
/// One tolerated missed tick, plus flush and upload latency, so a single slow cycle
/// does not read as a wedge.
pub const STALE_INTERVAL_FACTOR: u64 = 2;

/// The heartbeat verdict.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum StalenessState {
    /// No live runtime was observed, so reports are not expected and no verdict is
    /// computed. Wins over every other state, because it explains the absence.
    NotRunning,
    /// The runtime is live but no report has arrived yet.
    NeverReported,
    /// The runtime is live and the newest report is recent enough.
    Fresh,
    /// The runtime is live and reports have stopped arriving — the session's own
    /// reporting has died while its runtime has not.
    Stale,
}

/// The verdict plus the numbers behind it, so a client can render "last report 35m
/// ago" without recomputing anything.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, ToSchema)]
pub struct Staleness {
    /// The verdict.
    pub state: StalenessState,
    /// The producer's declared cadence, when a report has ever been seen. Read from
    /// the report, never a control-plane constant.
    pub expected_interval_secs: Option<u64>,
    /// Seconds since the newest report, when there is one.
    pub age_secs: Option<u64>,
}

/// Decide the heartbeat verdict.
///
/// `is_live` must already be fail-open: an unreachable backend is `false` (no alarm),
/// never `true`.
pub fn evaluate(newest: Option<&HealthIndexEntry>, is_live: bool, now: DateTime<Utc>) -> Staleness {
    let observed = newest.map(|entry| {
        let age = age_secs(&entry.generated_at, now);
        (entry.expected_interval_secs, age)
    });

    // `not_running` wins over everything: it explains the silence.
    if !is_live {
        return Staleness {
            state: StalenessState::NotRunning,
            expected_interval_secs: observed.map(|(interval, _)| interval),
            age_secs: observed.and_then(|(_, age)| age),
        };
    }

    let Some((interval, age)) = observed else {
        return Staleness {
            state: StalenessState::NeverReported,
            expected_interval_secs: None,
            age_secs: None,
        };
    };

    // An unreadable timestamp is a control-plane problem, not a session problem —
    // report the numbers we have and do not alarm.
    let state = match age {
        Some(age) if age > interval.saturating_mul(STALE_INTERVAL_FACTOR) => StalenessState::Stale,
        _ => StalenessState::Fresh,
    };

    Staleness {
        state,
        expected_interval_secs: Some(interval),
        age_secs: age,
    }
}

/// Seconds elapsed since `generated_at`, or `None` when it cannot be read.
///
/// A report stamped in the future (clock skew between the pod and the control plane)
/// yields `0`, never a negative age that would underflow into "ancient".
fn age_secs(generated_at: &str, now: DateTime<Utc>) -> Option<u64> {
    let generated_at = DateTime::parse_from_rfc3339(generated_at)
        .ok()?
        .with_timezone(&Utc);
    Some(now.signed_duration_since(generated_at).num_seconds().max(0) as u64)
}

#[cfg(test)]
#[path = "staleness_tests.rs"]
mod tests;
