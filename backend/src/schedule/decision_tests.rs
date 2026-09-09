//! The full clock matrix, including the four label/record combinations an
//! interrupted dispatch can leave behind.

use k8s_openapi::chrono::TimeZone;

use crate::schedule::CronExpr;

use super::*;

/// Deliberately several cadence periods long, so an overlap and an expiry are
/// separable: with a budget shorter than the cadence every overlap would also be a
/// timeout and the precedence between them would be untestable.
const TIMEOUT_SECS: i64 = 4 * 3600;

fn at(day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, minute, 0)
        .single()
        .expect("valid timestamp")
}

fn hourly() -> RunMode {
    RunMode::Cron(CronExpr::parse("0 * * * *").expect("valid cron"))
}

/// An idle schedule anchored at 27 July 00:00 with no history.
fn idle() -> ScheduleState {
    ScheduleState {
        anchor: at(27, 0, 0),
        cursor: None,
        running_label: false,
        open_dispatch: None,
        latest_terminal: None,
        paused: false,
    }
}

fn act(mode: &RunMode, state: &ScheduleState, now: DateTime<Utc>) -> ScheduleAction {
    decide(mode, state, now, Duration::seconds(TIMEOUT_SECS))
}

// ---- idle ------------------------------------------------------------------

#[test]
fn a_slot_that_has_not_arrived_yet_does_nothing() {
    assert_eq!(
        act(&hourly(), &idle(), at(27, 0, 30)),
        ScheduleAction::Nothing
    );
}

#[test]
fn the_first_due_slot_dispatches() {
    assert_eq!(
        act(&hourly(), &idle(), at(27, 1, 5)),
        ScheduleAction::Dispatch {
            slot: at(27, 1, 0),
            skipped: 0
        }
    );
}

#[test]
fn a_recorded_slot_is_not_dispatched_again() {
    let state = ScheduleState {
        cursor: Some(at(27, 1, 0)),
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &state, at(27, 1, 30)),
        ScheduleAction::Nothing
    );
}

#[test]
fn a_missed_window_fires_once_for_the_most_recent_slot_and_counts_the_rest() {
    // The control plane was away from 01:00 to 06:30. The 02:00-05:00 slots are
    // gone: replaying them would multiply exactly the load that caused the outage.
    let state = ScheduleState {
        cursor: Some(at(27, 1, 0)),
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &state, at(27, 6, 30)),
        ScheduleAction::Dispatch {
            slot: at(27, 6, 0),
            skipped: 4
        }
    );
}

#[test]
fn the_anchor_bounds_the_first_run_of_a_long_dormant_definition() {
    // No history at all, and the clock is a week past the anchor: still exactly one
    // firing, for the most recent slot.
    assert_eq!(
        act(&hourly(), &idle(), at(31, 9, 30)),
        ScheduleAction::Dispatch {
            slot: at(31, 9, 0),
            skipped: MAX_COUNTED_SKIPS
        }
    );
}

#[test]
fn the_skip_count_is_bounded_rather_than_exact() {
    // Cosmetic only: the firing decision does not depend on it, so counting a
    // month of missed minutes would be pure waste.
    let state = ScheduleState {
        cursor: Some(at(27, 0, 0)),
        ..idle()
    };
    let minutely = RunMode::Cron(CronExpr::parse("* * * * *").expect("valid cron"));
    let ScheduleAction::Dispatch { skipped, .. } = act(&minutely, &state, at(31, 0, 0)) else {
        panic!("a slot is due");
    };
    assert_eq!(skipped, MAX_COUNTED_SKIPS);
}

#[test]
fn pausing_suppresses_dispatch_without_touching_the_cursor() {
    let state = ScheduleState {
        paused: true,
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &state, at(27, 6, 30)),
        ScheduleAction::Nothing
    );
    // Resuming fires for the current slot, not for the ones spent paused.
    let resumed = ScheduleState {
        paused: false,
        ..state
    };
    assert_eq!(
        act(&hourly(), &resumed, at(27, 6, 30)),
        ScheduleAction::Dispatch {
            slot: at(27, 6, 0),
            // 01:00 through 05:00 were spent paused and are gone, not queued.
            skipped: 5
        }
    );
}

// ---- once ------------------------------------------------------------------

#[test]
fn a_once_definition_fires_immediately_and_exactly_once() {
    assert_eq!(
        act(&RunMode::Once, &idle(), at(27, 0, 1)),
        ScheduleAction::Dispatch {
            slot: at(27, 0, 0),
            skipped: 0
        }
    );
    let after = ScheduleState {
        cursor: Some(at(27, 0, 0)),
        ..idle()
    };
    assert_eq!(
        act(&RunMode::Once, &after, at(27, 12, 0)),
        ScheduleAction::Nothing
    );
}

// ---- in flight -------------------------------------------------------------

fn running(slot: DateTime<Utc>, started: DateTime<Utc>) -> ScheduleState {
    ScheduleState {
        cursor: Some(slot),
        running_label: true,
        open_dispatch: Some(OpenDispatch { slot, started }),
        ..idle()
    }
}

#[test]
fn a_run_in_flight_with_nothing_due_does_nothing() {
    let state = running(at(27, 1, 0), at(27, 1, 0));
    assert_eq!(
        act(&hourly(), &state, at(27, 1, 30)),
        ScheduleAction::Nothing
    );
}

#[test]
fn a_slot_arriving_mid_run_is_skipped_not_queued() {
    let state = running(at(27, 1, 0), at(27, 1, 0));
    assert_eq!(
        act(&hourly(), &state, at(27, 2, 5)),
        ScheduleAction::SkipOverlap { slot: at(27, 2, 0) }
    );
}

#[test]
fn the_watchdog_outranks_a_due_slot() {
    // Both an overlap and an expiry apply; releasing the stuck run first is what
    // lets the schedule recover at all.
    let state = running(at(27, 1, 0), at(27, 1, 0));
    assert_eq!(
        act(
            &hourly(),
            &state,
            at(27, 2, 5) + Duration::seconds(TIMEOUT_SECS)
        ),
        ScheduleAction::Expire {
            slot: at(27, 1, 0),
            started: at(27, 1, 0)
        }
    );
}

#[test]
fn the_watchdog_fires_exactly_at_the_budget() {
    // A `once` run so no slot can come due and confuse the boundary with an overlap.
    let state = running(at(27, 1, 0), at(27, 1, 0));
    let boundary = at(27, 1, 0) + Duration::seconds(TIMEOUT_SECS);
    assert_eq!(
        act(&RunMode::Once, &state, boundary - Duration::seconds(1)),
        ScheduleAction::Nothing
    );
    assert!(matches!(
        act(&RunMode::Once, &state, boundary),
        ScheduleAction::Expire { .. }
    ));
}

#[test]
fn the_watchdog_still_releases_a_paused_schedule() {
    // Otherwise pausing would be a way to strand a run in flight forever.
    let state = ScheduleState {
        paused: true,
        ..running(at(27, 1, 0), at(27, 1, 0))
    };
    assert!(matches!(
        act(
            &hourly(),
            &state,
            at(27, 1, 0) + Duration::seconds(TIMEOUT_SECS)
        ),
        ScheduleAction::Expire { .. }
    ));
}

// ---- completion + partial-write repair -------------------------------------

#[test]
fn a_terminal_record_clears_the_latch_with_its_status() {
    for status in [RunStatus::Ok, RunStatus::Failed, RunStatus::Timeout] {
        let state = ScheduleState {
            cursor: Some(at(27, 1, 0)),
            running_label: true,
            open_dispatch: None,
            latest_terminal: Some((at(27, 1, 0), status)),
            ..idle()
        };
        assert_eq!(
            act(&hourly(), &state, at(27, 1, 30)),
            ScheduleAction::Complete {
                slot: at(27, 1, 0),
                status
            }
        );
    }
}

#[test]
fn a_latch_with_no_dispatch_record_at_all_is_released() {
    // The label write landed and the record write did not. Without this repair the
    // schedule would look busy until the watchdog — except there is no dispatch to
    // time out from, so it would look busy forever.
    let state = ScheduleState {
        running_label: true,
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &state, at(27, 6, 0)),
        ScheduleAction::ReleaseRunning
    );
}

#[test]
fn a_dispatch_record_with_no_latch_is_re_latched_rather_than_ignored() {
    // The opposite interruption. Adopting assumes the run MAY be alive: assuming
    // otherwise could start a second run alongside a live one, whereas adopting
    // costs at worst one watchdog interval of looking busy.
    let state = ScheduleState {
        cursor: Some(at(27, 1, 0)),
        running_label: false,
        open_dispatch: Some(OpenDispatch {
            slot: at(27, 1, 0),
            started: at(27, 1, 0),
        }),
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &state, at(27, 2, 30)),
        ScheduleAction::AdoptRunning { slot: at(27, 1, 0) }
    );
}

#[test]
fn adoption_takes_precedence_over_a_due_slot() {
    // A due slot must never start a second run while an unlatched dispatch record
    // says one may already be going.
    let state = ScheduleState {
        cursor: Some(at(27, 1, 0)),
        open_dispatch: Some(OpenDispatch {
            slot: at(27, 1, 0),
            started: at(27, 1, 0),
        }),
        ..idle()
    };
    assert!(matches!(
        act(&hourly(), &state, at(27, 9, 0)),
        ScheduleAction::AdoptRunning { .. }
    ));
}

#[test]
fn repairs_run_even_while_paused() {
    let released = ScheduleState {
        running_label: true,
        paused: true,
        ..idle()
    };
    assert_eq!(
        act(&hourly(), &released, at(27, 6, 0)),
        ScheduleAction::ReleaseRunning
    );
    let adopted = ScheduleState {
        paused: true,
        open_dispatch: Some(OpenDispatch {
            slot: at(27, 1, 0),
            started: at(27, 1, 0),
        }),
        ..idle()
    };
    assert!(matches!(
        act(&hourly(), &adopted, at(27, 6, 0)),
        ScheduleAction::AdoptRunning { .. }
    ));
}

#[test]
fn the_decision_is_a_pure_function_of_its_inputs() {
    // Same inputs, same answer — the property the whole stateless design rests on.
    let state = running(at(27, 1, 0), at(27, 1, 0));
    let now = at(27, 2, 5);
    assert_eq!(act(&hourly(), &state, now), act(&hourly(), &state, now));
}
