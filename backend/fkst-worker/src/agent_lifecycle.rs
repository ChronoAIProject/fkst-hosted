//! The worker agent's own drain-lifecycle gate (issue #140a).
//!
//! Split out of `agent.rs` (kept under the 500-line budget) as a child module
//! included via `#[path]`, so it can reach `WorkerAgent`'s private `lifecycle`
//! field. The gate is a lock-free [`AtomicU8`](std::sync::atomic::AtomicU8) the
//! heartbeat / pull / drain tasks share: the pull loop reads it each tick and
//! stops requesting work once it reads `Draining`, and the SIGTERM drain flips it
//! via [`WorkerAgent::begin_drain`]. It is unrelated to the `LifecycleState` the
//! heartbeat carries (that argument is passed explicitly by the run loop); this
//! gate is the worker's own intent, not the wire-reported state. No secrets.

use std::sync::atomic::Ordering;

use fkst_shared::protocol::LifecycleState;

use super::WorkerAgent;

/// `AtomicU8` encoding of [`LifecycleState::Active`] for the worker's own drain
/// gate. The two reportable states map 1:1; any other byte decodes to `Active`
/// (fail-safe: an impossible value never reads as "draining").
pub(super) const ACTIVE: u8 = 0;
/// `AtomicU8` encoding of [`LifecycleState::Draining`].
const DRAINING: u8 = 1;

/// Decode the stored drain byte into the wire [`LifecycleState`]. Any value
/// other than [`DRAINING`] is `Active` (fail-safe default).
fn decode(byte: u8) -> LifecycleState {
    match byte {
        DRAINING => LifecycleState::Draining,
        _ => LifecycleState::Active,
    }
}

impl WorkerAgent {
    /// Flip the worker into the `Draining` lifecycle (#140a). Idempotent: a
    /// second call is a no-op (drain is terminal — the worker never returns to
    /// `Active`). Stores with `Release` ordering so the pull loop's `Acquire`
    /// read in [`WorkerAgent::lifecycle`] observes the flip on its next tick and
    /// stops requesting new work.
    pub(crate) fn begin_drain(&self) {
        self.lifecycle.store(DRAINING, Ordering::Release);
    }

    /// The worker's own drain state (#140a). The pull loop reads this each tick
    /// and skips pulling once it is `Draining`; the drain routine and tests read
    /// it to confirm the flip. Decoded fail-safe (any non-draining byte is
    /// `Active`).
    pub(crate) fn lifecycle(&self) -> LifecycleState {
        decode(self.lifecycle.load(Ordering::Acquire))
    }
}
