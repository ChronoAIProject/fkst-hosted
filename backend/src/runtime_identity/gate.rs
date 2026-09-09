//! Bounded suppression of repeated identity-backfill attempts.
//!
//! The reconciler is level-triggered: it re-observes every live runtime on every
//! sweep (default 30s). Without a gate, a runtime whose stamp can never be
//! completed — a genuine attribution conflict, or a value the OpenSandbox
//! metadata validator rejects — would be re-attempted every sweep forever,
//! producing one backend call, one warning, and one `identity_conflict`
//! lifecycle event per sweep. That is precisely the poll spam the epic forbids.
//!
//! So every terminal decision parks the session for a cooldown:
//!
//! - a **permanent** decision (conflict, invalid value, backend rejection) waits
//!   [`PERMANENT_COOLDOWN`], long enough that a human fixing the trigger is the
//!   thing that moves it, not the sweep;
//! - a **settled** decision (a successful backfill) waits [`SETTLE_COOLDOWN`],
//!   which only has to outlast the window in which a stale observation could
//!   still describe the pre-patch runtime.
//!
//! The gate is process-local and reconstructable: it is rebuilt for each leader
//! generation, and losing it costs at most one redundant, idempotent patch.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Cooldown after a decision that cannot change without human action.
pub const PERMANENT_COOLDOWN: Duration = Duration::from_secs(3600);

/// Cooldown after a successful backfill, covering only observation staleness.
pub const SETTLE_COOLDOWN: Duration = Duration::from_secs(60);

/// Hard cap on tracked sessions. A deployment has far fewer live sessions than
/// this; the cap exists so a pathological churn of session ids can never grow
/// the map without bound.
const MAX_ENTRIES: usize = 4_096;

/// A cheaply clonable, process-local suppression set. Cloning shares one map, so
/// the reconcile driver and its per-repo tasks all observe the same cooldowns.
#[derive(Clone, Default)]
pub struct IdentityGate {
    inner: Arc<Mutex<HashMap<String, Instant>>>,
}

impl IdentityGate {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether an identity operation may be attempted for `session_id` now.
    pub fn allow(&self, session_id: &str) -> bool {
        let now = Instant::now();
        let mut entries = self.lock();
        entries.retain(|_, until| *until > now);
        !entries.contains_key(session_id)
    }

    /// Park `session_id` for `cooldown`.
    ///
    /// A later, longer cooldown always wins over a shorter one still in flight:
    /// a conflict observed after a settle must not be released early.
    pub fn suppress(&self, session_id: &str, cooldown: Duration) {
        let now = Instant::now();
        let until = now
            .checked_add(cooldown)
            .unwrap_or_else(|| now + SETTLE_COOLDOWN);
        let mut entries = self.lock();
        entries.retain(|_, until| *until > now);
        if entries.len() >= MAX_ENTRIES && !entries.contains_key(session_id) {
            // Bounded by construction: drop the whole set rather than let it
            // grow. The cost is at most one redundant idempotent patch per
            // tracked session, which is exactly what the gate is allowed to
            // lose (see the module docs).
            tracing::warn!(
                entries = entries.len(),
                "runtime identity gate: suppression set full; clearing it"
            );
            entries.clear();
        }
        entries
            .entry(session_id.to_string())
            .and_modify(|existing| {
                if until > *existing {
                    *existing = until;
                }
            })
            .or_insert(until);
    }

    /// How many sessions are currently parked (diagnostics + tests).
    pub fn len(&self) -> usize {
        let now = Instant::now();
        let mut entries = self.lock();
        entries.retain(|_, until| *until > now);
        entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Poison-safe: a panic elsewhere while the lock was held must not wedge
    /// every subsequent reconcile pass.
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Instant>> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }
}

impl std::fmt::Debug for IdentityGate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // A bounded count only: a `{:?}` of the reconcile context must never
        // dump session ids into a log line that did not ask for them.
        f.debug_struct("IdentityGate")
            .field("suppressed", &self.len())
            .finish()
    }
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
