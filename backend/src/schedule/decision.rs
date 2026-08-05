//! The pure clock: given what GitHub currently says about one scheduled workflow,
//! decide the single thing to do about it now.
//!
//! Everything the decision needs is in [`ScheduleState`], which the impure pass
//! assembles from an issue's labels and its run-record comments. No I/O, no clock
//! read — `now` is injected — so the whole matrix, including the awkward
//! partial-write states, is exhaustively testable without a cluster.
//!
//! ## Why the state has two sources of truth for "running"
//!
//! A dispatch is two GitHub writes: a `fkst-cron-running` LABEL and a `Dispatched`
//! run RECORD. They cannot be made atomic, and every reconcile effect in this
//! codebase is best-effort, so an interrupted dispatch leaves one of them behind.
//! Rather than pretend one is authoritative, [`ScheduleState`] carries BOTH and
//! [`decide`] converges each disagreement explicitly:
//!
//! | label | open dispatch record | decision |
//! |---|---|---|
//! | set | present | the run is genuinely in flight |
//! | set | absent | [`ScheduleAction::ReleaseRunning`] — nothing is running |
//! | clear | present | [`ScheduleAction::AdoptRunning`] — re-latch, then let the watchdog govern it |
//! | clear | absent | idle |
//!
//! `AdoptRunning` deliberately assumes the run may be alive. Assuming the opposite
//! would let a second dispatch start alongside a live run; assuming this one costs
//! at worst one watchdog interval of a schedule looking busy.

use k8s_openapi::chrono::{DateTime, Duration, Utc};

use crate::goals::scheduled_workflow_parse::RunMode;

use super::marker::RunStatus;

/// How many skipped slots the misfire report will count before giving up and
/// reporting the cap. Purely cosmetic — the firing decision does not depend on it —
/// so it is bounded rather than exact.
const MAX_COUNTED_SKIPS: u32 = 64;

/// The in-flight run recovered from the newest `Dispatched` record that has no
/// terminal record for its slot.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct OpenDispatch {
    pub slot: DateTime<Utc>,
    pub started: DateTime<Utc>,
}

/// Everything the clock needs to know about one scheduled workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleState {
    /// When the definition issue was created. A definition never fires for a slot
    /// that predates its own existence, however long the control plane was away.
    pub anchor: DateTime<Utc>,
    /// The newest slot carrying ANY run record — the recurrence cursor. This is
    /// what makes the pass stateless: without it the clock would re-emit the first
    /// slot forever.
    pub cursor: Option<DateTime<Utc>>,
    /// The `fkst-cron-running` latch.
    pub running_label: bool,
    /// The newest dispatch with no terminal record for its slot.
    pub open_dispatch: Option<OpenDispatch>,
    /// The terminal status of the newest COMPLETED slot, used to decide whether a
    /// still-set latch should be cleared as a completion or released as a stray.
    pub latest_terminal: Option<(DateTime<Utc>, RunStatus)>,
    /// The user-applied `fkst-cron-paused` latch.
    pub paused: bool,
}

/// The one thing to do about a schedule right now.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ScheduleAction {
    /// Nothing is due and nothing is inconsistent.
    Nothing,
    /// Fire: create the run issue, latch the label, record the dispatch.
    ///
    /// `skipped` counts the slots between the cursor and this one that will never
    /// run. The misfire policy is to fire ONCE for the most recent due slot and
    /// record the rest — replaying a backlog after an outage is never what an
    /// operator wants from a cron, and would multiply the very load that caused it.
    Dispatch { slot: DateTime<Utc>, skipped: u32 },
    /// A slot came due while the previous run was still going. Recorded, not queued.
    SkipOverlap { slot: DateTime<Utc> },
    /// The in-flight run outlived its budget. The cost backstop: the only thing
    /// that stops a hung run pinning a schedule forever.
    Expire {
        slot: DateTime<Utc>,
        started: DateTime<Utc>,
    },
    /// A terminal record arrived for the in-flight slot; drop the running latch.
    Complete {
        slot: DateTime<Utc>,
        status: RunStatus,
    },
    /// The latch is set but no dispatch was ever recorded — release it.
    ReleaseRunning,
    /// A dispatch was recorded but the latch write did not land — re-latch it.
    AdoptRunning { slot: DateTime<Utc> },
}

/// Decide the action for one schedule.
///
/// `timeout` is the deployment's per-run budget. Repairs are considered BEFORE the
/// pause check: a paused schedule with a stuck latch must still be released, or
/// pausing would be a way to strand a run forever.
pub fn decide(
    mode: &RunMode,
    state: &ScheduleState,
    now: DateTime<Utc>,
    timeout: Duration,
) -> ScheduleAction {
    match (state.running_label, state.open_dispatch) {
        // Genuinely running: the watchdog outranks everything, then overlap.
        (true, Some(open)) => {
            if now - open.started >= timeout {
                return ScheduleAction::Expire {
                    slot: open.slot,
                    started: open.started,
                };
            }
            match due_slot(mode, state, now) {
                Some(slot) => ScheduleAction::SkipOverlap { slot },
                None => ScheduleAction::Nothing,
            }
        }
        // Latched with nothing open. Either a terminal record arrived for the run
        // the latch belongs to (the common case, and what actually clears it), or
        // the dispatch record never landed and the latch is a stray.
        (true, None) => match state.latest_terminal {
            Some((slot, status)) => ScheduleAction::Complete { slot, status },
            None => ScheduleAction::ReleaseRunning,
        },
        // A dispatch record with no latch: the label write was interrupted.
        (false, Some(open)) => ScheduleAction::AdoptRunning { slot: open.slot },
        // Idle.
        (false, None) => {
            if state.paused {
                return ScheduleAction::Nothing;
            }
            match due_slot(mode, state, now) {
                Some(slot) => ScheduleAction::Dispatch {
                    slot,
                    skipped: count_skipped(mode, state, slot),
                },
                None => ScheduleAction::Nothing,
            }
        }
    }
}

/// The most recent slot that has come due and has not been recorded yet.
///
/// For [`RunMode::Once`] the anchor IS the slot, so a definition fires as soon as
/// the pass observes it and never again once any record exists.
fn due_slot(mode: &RunMode, state: &ScheduleState, now: DateTime<Utc>) -> Option<DateTime<Utc>> {
    match mode {
        RunMode::Once => (state.cursor.is_none() && now >= state.anchor).then_some(state.anchor),
        RunMode::Cron(cron) => {
            let floor = state.cursor.unwrap_or(state.anchor);
            cron.previous_or_equal(now).filter(|slot| *slot > floor)
        }
    }
}

/// How many slots between the cursor and `slot` are being passed over.
fn count_skipped(mode: &RunMode, state: &ScheduleState, slot: DateTime<Utc>) -> u32 {
    let RunMode::Cron(cron) = mode else {
        return 0;
    };
    let mut cursor = state.cursor.unwrap_or(state.anchor);
    let mut skipped = 0;
    while skipped < MAX_COUNTED_SKIPS {
        match cron.next_after(cursor) {
            Ok(next) if next < slot => {
                skipped += 1;
                cursor = next;
            }
            _ => break,
        }
    }
    skipped
}

#[cfg(test)]
#[path = "decision_tests.rs"]
mod tests;
