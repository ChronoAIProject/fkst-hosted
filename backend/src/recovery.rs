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
    pub leader_election_enabled: bool,
    pub leader_identity: Option<String>,
    pub observed_holder_identity: Option<String>,
    pub leader: bool,
    /// Acquisition resync completed; Service publication may still be pending.
    pub leader_ready: bool,
    pub leader_routing_ready: bool,
    pub leader_acquisitions: u64,
    pub leader_losses: u64,
    pub leader_renew_failures: u64,
    pub leader_acquire_failures: u64,
    pub leader_conflicts: u64,
    pub leader_routing_failures: u64,
    pub observed_lease_transitions: u64,
    pub last_successful_leader_renew_timestamp_seconds: u64,
    pub last_successful_leader_resync_timestamp_seconds: u64,
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
            leader_election_enabled: false,
            leader_identity: None,
            observed_holder_identity: None,
            leader: false,
            leader_ready: false,
            leader_routing_ready: false,
            leader_acquisitions: 0,
            leader_losses: 0,
            leader_renew_failures: 0,
            leader_acquire_failures: 0,
            leader_conflicts: 0,
            leader_routing_failures: 0,
            observed_lease_transitions: 0,
            last_successful_leader_renew_timestamp_seconds: 0,
            last_successful_leader_resync_timestamp_seconds: 0,
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

    /// Enable the HA readiness contract. Followers remain live but never ready;
    /// each acquisition must complete a fresh full resync before Service routing.
    pub fn enable_leader_election(&self, identity: String) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.leader_election_enabled = true;
        snapshot.leader_identity = Some(identity);
        snapshot.leader = false;
        snapshot.leader_ready = false;
        snapshot.leader_routing_ready = false;
        snapshot.ready = false;
        snapshot.startup_resync_complete = false;
        snapshot.degraded = false;
    }

    /// Record Lease acquisition before a new worker generation starts.
    pub fn record_leader_acquired(&self, lease_transitions: u64) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.leader = true;
        snapshot.leader_ready = false;
        snapshot.leader_routing_ready = false;
        snapshot.ready = false;
        snapshot.startup_resync_complete = false;
        snapshot.degraded = false;
        snapshot.leader_acquisitions = snapshot.leader_acquisitions.saturating_add(1);
        snapshot.observed_lease_transitions = lease_transitions;
        snapshot.observed_holder_identity = snapshot.leader_identity.clone();
        snapshot.last_successful_leader_renew_timestamp_seconds = unix_now();
    }

    /// Record a confirmed renewal while the current generation remains leader.
    pub fn record_leader_renewed(&self, lease_transitions: u64) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.observed_lease_transitions = lease_transitions;
        snapshot.observed_holder_identity = snapshot.leader_identity.clone();
        snapshot.last_successful_leader_renew_timestamp_seconds = unix_now();
    }

    /// Record another holder while this process follows it.
    pub fn record_leader_follower(&self, holder: Option<String>, lease_transitions: u64) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.observed_holder_identity = holder;
        snapshot.observed_lease_transitions = lease_transitions;
        snapshot.leader = false;
        snapshot.leader_ready = false;
        snapshot.leader_routing_ready = false;
        snapshot.ready = false;
        snapshot.startup_resync_complete = false;
        snapshot.degraded = false;
    }

    /// Invalidate Service readiness immediately after confirmed leadership loss.
    pub fn record_leader_lost(&self, holder: Option<String>, lease_transitions: u64) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if snapshot.leader {
            snapshot.leader_losses = snapshot.leader_losses.saturating_add(1);
        }
        snapshot.observed_holder_identity = holder;
        snapshot.observed_lease_transitions = lease_transitions;
        snapshot.leader = false;
        snapshot.leader_ready = false;
        snapshot.leader_routing_ready = false;
        snapshot.ready = false;
        snapshot.startup_resync_complete = false;
        snapshot.degraded = false;
    }

    pub fn record_leader_api_failure(&self, renewal: bool) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        if renewal {
            snapshot.leader_renew_failures = snapshot.leader_renew_failures.saturating_add(1);
        } else {
            snapshot.leader_acquire_failures = snapshot.leader_acquire_failures.saturating_add(1);
        }
    }

    pub fn record_leader_conflict(&self) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.leader_conflicts = snapshot.leader_conflicts.saturating_add(1);
    }

    /// Gate public readiness on successful publication of this leader's routing
    /// label. Failed or absent publication stays fail-closed after resync.
    pub fn record_leader_routing(&self, published: bool) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.leader_routing_ready = published && snapshot.leader;
        if snapshot.leader_election_enabled {
            snapshot.ready = snapshot.leader
                && snapshot.leader_ready
                && snapshot.leader_routing_ready
                && !snapshot.degraded;
        }
    }

    pub fn record_leader_routing_failure(&self) {
        let mut snapshot = self.inner.write().unwrap_or_else(|e| e.into_inner());
        snapshot.leader_routing_failures = snapshot.leader_routing_failures.saturating_add(1);
        snapshot.leader_routing_ready = false;
        snapshot.ready = false;
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
                snapshot.degraded = false;
                snapshot.last_success_timestamp_seconds = unix_now();
                if snapshot.leader_election_enabled && snapshot.leader {
                    snapshot.leader_ready = true;
                    snapshot.last_successful_leader_resync_timestamp_seconds =
                        snapshot.last_success_timestamp_seconds;
                }
                snapshot.ready = if snapshot.leader_election_enabled {
                    snapshot.leader && snapshot.leader_ready && snapshot.leader_routing_ready
                } else {
                    true
                };
            }
            ResyncResult::Partial => {
                snapshot.attempts.partial = snapshot.attempts.partial.saturating_add(1);
                snapshot.ready = false;
                snapshot.degraded = true;
                if snapshot.leader_election_enabled {
                    snapshot.leader_ready = false;
                }
            }
            ResyncResult::Failure => {
                snapshot.attempts.failure = snapshot.attempts.failure.saturating_add(1);
                snapshot.ready = false;
                snapshot.degraded = true;
                if snapshot.leader_election_enabled {
                    snapshot.leader_ready = false;
                }
            }
        }
    }

    pub fn snapshot(&self) -> RecoverySnapshot {
        self.inner.read().unwrap_or_else(|e| e.into_inner()).clone()
    }
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
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

    #[test]
    fn leader_readiness_requires_each_acquisitions_resync() {
        let monitor = RecoveryMonitor::new(true);
        monitor.enable_leader_election("pod-a".to_string());
        assert!(!monitor.snapshot().ready);

        monitor.record_leader_acquired(3);
        monitor.record_attempt(ResyncResult::Success, Duration::from_millis(10), 2);
        assert!(!monitor.snapshot().ready, "routing is not published yet");
        monitor.record_leader_routing(true);
        let ready = monitor.snapshot();
        assert!(ready.leader);
        assert!(ready.leader_ready);
        assert!(ready.ready);
        assert_eq!(ready.leader_acquisitions, 1);
        assert_eq!(ready.observed_lease_transitions, 3);
        assert!(ready.last_successful_leader_resync_timestamp_seconds > 0);

        monitor.record_attempt(ResyncResult::Success, Duration::from_millis(5), 1);
        let periodic_success = monitor.snapshot();
        assert!(
            periodic_success.ready,
            "a successful periodic resync must preserve published leader readiness"
        );
        assert!(periodic_success.leader_routing_ready);

        monitor.record_leader_lost(Some("pod-b".to_string()), 4);
        let follower = monitor.snapshot();
        assert!(!follower.leader);
        assert!(!follower.leader_ready);
        assert!(!follower.leader_routing_ready);
        assert!(!follower.ready);
        assert!(!follower.startup_resync_complete);
        assert_eq!(follower.leader_losses, 1);
        assert_eq!(follower.observed_holder_identity.as_deref(), Some("pod-b"));

        monitor.record_leader_acquired(5);
        let reacquired = monitor.snapshot();
        assert!(!reacquired.ready, "reacquisition requires another resync");
        assert_eq!(reacquired.leader_acquisitions, 2);
    }
}
