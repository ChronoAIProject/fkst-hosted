//! Shared control-plane recovery state.
//!
//! The full-resync coordinator is the sole writer. HTTP readiness and Prometheus
//! exposition read immutable snapshots, so neither route can influence recovery.

use std::sync::{Arc, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// The bounded result classes exported by the recovery metrics.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ResyncResult {
    Success,
    Partial,
    Failure,
}

/// Monotonic process-local attempt counters, one field per bounded result class.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ResyncAttemptCounts {
    pub success: u64,
    pub partial: u64,
    pub failure: u64,
}

/// A consistent read projection for readiness and metrics.
#[derive(Clone, Debug, PartialEq)]
pub struct RecoverySnapshot {
    pub dispatch_enabled: bool,
    pub startup_resync_complete: bool,
    pub ready: bool,
    pub degraded: bool,
    pub attempts: ResyncAttemptCounts,
    pub last_result: Option<ResyncResult>,
    pub last_duration_seconds: f64,
    pub last_repositories_enqueued: u64,
    pub last_success_timestamp_seconds: u64,
}

impl RecoverySnapshot {
    fn new(dispatch_enabled: bool) -> Self {
        Self {
            dispatch_enabled,
            // No repository discovery is required when dispatch is deliberately
            // disabled, so that operating mode is ready immediately.
            startup_resync_complete: !dispatch_enabled,
            ready: !dispatch_enabled,
            degraded: false,
            attempts: ResyncAttemptCounts::default(),
            last_result: None,
            last_duration_seconds: 0.0,
            last_repositories_enqueued: 0,
            last_success_timestamp_seconds: 0,
        }
    }
}

/// Cheaply clonable writer/read handle shared by the coordinator and HTTP state.
#[derive(Clone, Debug)]
pub struct RecoveryMonitor {
    inner: Arc<RwLock<RecoverySnapshot>>,
}

impl RecoveryMonitor {
    /// Build the initial projection. Enabled dispatch starts recovering; disabled
    /// dispatch has no recovery prerequisite and starts ready.
    pub fn new(dispatch_enabled: bool) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RecoverySnapshot::new(dispatch_enabled))),
        }
    }

    /// Mark an enabled deployment degraded when the reconciler cannot be built.
    /// This is not a resync attempt, so it does not change attempt counters.
    pub fn mark_unavailable(&self) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if snapshot.dispatch_enabled {
            snapshot.ready = false;
            snapshot.degraded = true;
        }
    }

    /// Record one completed full-resync attempt.
    pub fn record_attempt(
        &self,
        result: ResyncResult,
        duration: Duration,
        repositories_enqueued: usize,
    ) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.last_result = Some(result);
        snapshot.last_duration_seconds = duration.as_secs_f64();
        snapshot.last_repositories_enqueued =
            u64::try_from(repositories_enqueued).unwrap_or(u64::MAX);

        match result {
            ResyncResult::Success => {
                snapshot.attempts.success = snapshot.attempts.success.saturating_add(1);
                snapshot.startup_resync_complete = true;
                snapshot.ready = true;
                snapshot.degraded = false;
                snapshot.last_success_timestamp_seconds = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
            }
            ResyncResult::Partial => {
                snapshot.attempts.partial = snapshot.attempts.partial.saturating_add(1);
                snapshot.ready = false;
                snapshot.degraded = true;
            }
            ResyncResult::Failure => {
                snapshot.attempts.failure = snapshot.attempts.failure.saturating_add(1);
                snapshot.ready = false;
                snapshot.degraded = true;
            }
        }
    }

    pub fn snapshot(&self) -> RecoverySnapshot {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

impl Default for RecoveryMonitor {
    fn default() -> Self {
        Self::new(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_dispatch_is_ready_without_attempts() {
        let snapshot = RecoveryMonitor::new(false).snapshot();
        assert!(snapshot.ready);
        assert!(snapshot.startup_resync_complete);
        assert_eq!(snapshot.attempts, ResyncAttemptCounts::default());
    }

    #[test]
    fn attempts_drive_degraded_and_ready_transitions() {
        let monitor = RecoveryMonitor::new(true);
        assert!(!monitor.snapshot().ready);

        monitor.record_attempt(ResyncResult::Partial, Duration::from_millis(250), 3);
        let partial = monitor.snapshot();
        assert!(partial.degraded);
        assert!(!partial.ready);
        assert_eq!(partial.attempts.partial, 1);
        assert_eq!(partial.last_repositories_enqueued, 3);

        monitor.record_attempt(ResyncResult::Success, Duration::from_millis(125), 5);
        let success = monitor.snapshot();
        assert!(success.ready);
        assert!(!success.degraded);
        assert!(success.startup_resync_complete);
        assert_eq!(success.attempts.success, 1);
        assert!(success.last_success_timestamp_seconds > 0);

        monitor.record_attempt(ResyncResult::Failure, Duration::from_secs(1), 0);
        let failure = monitor.snapshot();
        assert!(!failure.ready);
        assert!(failure.degraded);
        assert!(failure.startup_resync_complete);
        assert_eq!(failure.attempts.failure, 1);
    }
}
