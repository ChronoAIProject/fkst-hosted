//! State recovery from GitHub facts, and the effects each decision produces.

use k8s_openapi::chrono::TimeZone;

use crate::goals::scheduled_workflow_parse::parse_scheduled_workflow;
use crate::schedule::{RunRecord, RunStatus};

use super::*;

fn at(day: u32, hour: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 7, day, hour, 0, 0)
        .single()
        .expect("valid timestamp")
}

fn hourly_spec() -> ScheduledWorkflowSpec {
    parse_scheduled_workflow(
        "### Workflow\nsourcing\n\n### Run Mode\ncron: 0 * * * *\n\n### Arguments\nrole: engineer\n",
    )
    .expect("valid definition")
}

fn cfg() -> ReconcileConfig {
    ReconcileConfig::default()
}

fn observation<'a>(
    labels: &'a [String],
    spec: &'a ScheduledWorkflowSpec,
    records: &'a [RunRecord],
) -> ScheduleObservation<'a> {
    ScheduleObservation {
        schedule_issue: 50,
        labels,
        created_at: at(27, 0),
        spec,
        records,
        work_label: "fkst-dev-acme",
        creator_login: "alice",
    }
}

fn labels(names: &[&str]) -> Vec<String> {
    names.iter().map(|name| (*name).to_string()).collect()
}

// ---- state recovery --------------------------------------------------------

#[test]
fn the_cursor_is_the_newest_recorded_slot() {
    let spec = hourly_spec();
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
        RunRecord::new(at(27, 3), RunStatus::Failed, at(27, 3)),
        RunRecord::new(at(27, 2), RunStatus::Ok, at(27, 2)),
    ];
    let state = build_state(&observation(&[], &spec, &records));
    assert_eq!(
        state.cursor,
        Some(at(27, 3)),
        "order in the issue is not order in time"
    );
    assert_eq!(state.latest_terminal, Some((at(27, 3), RunStatus::Failed)));
    assert_eq!(state.open_dispatch, None);
}

#[test]
fn a_dispatch_with_a_terminal_for_its_slot_is_not_open() {
    let spec = hourly_spec();
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)),
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
    ];
    assert_eq!(
        build_state(&observation(&[], &spec, &records)).open_dispatch,
        None
    );
}

#[test]
fn a_dispatch_without_a_terminal_is_the_open_run() {
    let spec = hourly_spec();
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)),
        RunRecord::new(at(27, 1), RunStatus::Ok, at(27, 1)),
        RunRecord::new(at(27, 2), RunStatus::Dispatched, at(27, 2)),
    ];
    assert_eq!(
        build_state(&observation(&[], &spec, &records)).open_dispatch,
        Some(crate::schedule::OpenDispatch {
            slot: at(27, 2),
            started: at(27, 2)
        })
    );
}

#[test]
fn the_latches_are_read_from_the_definition_issues_own_labels() {
    let spec = hourly_spec();
    let names = labels(&[
        SCHEDULED_WORKFLOW_LABEL_FOR_TEST,
        CRON_RUNNING_LABEL,
        CRON_PAUSED_LABEL,
    ]);
    let state = build_state(&observation(&names, &spec, &[]));
    assert!(state.running_label && state.paused);
}

/// Local alias so the fixture reads like a real issue's label set without pulling
/// the whole reserved-label module into scope.
const SCHEDULED_WORKFLOW_LABEL_FOR_TEST: &str = "fkst-scheduled-workflow";

// ---- effects ---------------------------------------------------------------

#[test]
fn a_due_slot_produces_a_dispatch_carrying_the_definitions_arguments() {
    let spec = hourly_spec();
    let effects = plan_schedule(&observation(&[], &spec, &[]), at(27, 1), &cfg());
    let [ScheduleEffect::Dispatch {
        request, skipped, ..
    }] = effects.as_slice()
    else {
        panic!("expected exactly one dispatch, got {effects:?}");
    };
    assert_eq!(*skipped, 0);
    assert_eq!(request.slot, at(27, 1));
    assert_eq!(request.workflow_id, "sourcing");
    assert_eq!(request.work_label, "fkst-dev-acme");
    assert_eq!(request.creator_login, "alice");
    assert_eq!(request.arguments["role"], "engineer");
    assert!(!request.manual, "the clock never fires a manual run");
}

#[test]
fn an_accepted_definition_clears_a_stale_invalid_latch() {
    // The clearable-latch convention: an author fixes a typo by editing the issue,
    // not by recreating it.
    let spec = hourly_spec();
    let names = labels(&[SCHEDULE_INVALID_LABEL]);
    let effects = plan_schedule(&observation(&names, &spec, &[]), at(27, 0), &cfg());
    assert_eq!(
        effects,
        vec![ScheduleEffect::ClearInvalid { schedule_issue: 50 }],
        "nothing is due yet, but the stale latch still goes"
    );
}

#[test]
fn the_clear_precedes_the_dispatch_so_a_fixed_definition_runs_the_same_pass() {
    let spec = hourly_spec();
    let names = labels(&[SCHEDULE_INVALID_LABEL]);
    let effects = plan_schedule(&observation(&names, &spec, &[]), at(27, 1), &cfg());
    assert!(
        matches!(effects[0], ScheduleEffect::ClearInvalid { .. }),
        "{effects:?}"
    );
    assert!(
        matches!(effects[1], ScheduleEffect::Dispatch { .. }),
        "{effects:?}"
    );
}

#[test]
fn an_overlapping_slot_records_a_skip_and_creates_no_run() {
    let spec = hourly_spec();
    let names = labels(&[CRON_RUNNING_LABEL]);
    let records = vec![RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1))];
    // A budget wider than the cadence, so the overlap is observable at all: with
    // the 1h default the 02:00 slot would arrive exactly as the watchdog fires.
    let cfg = ReconcileConfig {
        cron_max_runtime_secs: 4 * 3600,
        ..cfg()
    };
    let effects = plan_schedule(&observation(&names, &spec, &records), at(27, 2), &cfg);
    assert_eq!(
        effects,
        vec![ScheduleEffect::RecordSkip {
            schedule_issue: 50,
            slot: at(27, 2)
        }]
    );
}

#[test]
fn a_terminal_record_completes_the_run() {
    let spec = hourly_spec();
    let names = labels(&[CRON_RUNNING_LABEL]);
    let records = vec![
        RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1)),
        RunRecord::new(at(27, 1), RunStatus::Failed, at(27, 1)),
    ];
    let effects = plan_schedule(&observation(&names, &spec, &records), at(27, 1), &cfg());
    assert_eq!(
        effects,
        vec![ScheduleEffect::Complete {
            schedule_issue: 50,
            slot: at(27, 1),
            status: RunStatus::Failed
        }]
    );
}

#[test]
fn a_run_past_its_budget_expires() {
    let spec = hourly_spec();
    let names = labels(&[CRON_RUNNING_LABEL]);
    let records = vec![RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1))];
    let mut cfg = cfg();
    cfg.cron_max_runtime_secs = 3600;
    let effects = plan_schedule(&observation(&names, &spec, &records), at(27, 3), &cfg);
    assert_eq!(
        effects,
        vec![ScheduleEffect::Expire {
            schedule_issue: 50,
            slot: at(27, 1),
            started: at(27, 1)
        }]
    );
}

#[test]
fn both_interrupted_dispatch_states_are_repaired() {
    let spec = hourly_spec();
    // Label written, record lost.
    let stray = labels(&[CRON_RUNNING_LABEL]);
    assert_eq!(
        plan_schedule(&observation(&stray, &spec, &[]), at(27, 5), &cfg()),
        vec![ScheduleEffect::ReleaseRunning { schedule_issue: 50 }]
    );
    // Record written, label lost.
    let records = vec![RunRecord::new(at(27, 1), RunStatus::Dispatched, at(27, 1))];
    assert_eq!(
        plan_schedule(&observation(&[], &spec, &records), at(27, 2), &cfg()),
        vec![ScheduleEffect::AdoptRunning {
            schedule_issue: 50,
            slot: at(27, 1)
        }]
    );
}

#[test]
fn a_paused_definition_produces_no_effects() {
    let spec = hourly_spec();
    let names = labels(&[CRON_PAUSED_LABEL]);
    assert!(plan_schedule(&observation(&names, &spec, &[]), at(27, 9), &cfg()).is_empty());
}

// ---- validation ------------------------------------------------------------

#[test]
fn a_cadence_tighter_than_the_deployment_minimum_is_rejected_naming_the_limit() {
    let spec = parse_scheduled_workflow("### Workflow\nx\n\n### Run Mode\ncron: */5 * * * *\n")
        .expect("valid definition");
    let detail = check_min_interval(&spec.run_mode, &cfg()).expect_err("too tight");
    assert!(detail.contains("300s"), "names the real cadence: {detail}");
    assert!(detail.contains("900s"), "names the limit: {detail}");
    assert!(
        detail.contains("boots a session pod"),
        "explains WHY there is a limit: {detail}"
    );
}

#[test]
fn a_cadence_at_or_above_the_minimum_is_accepted() {
    for expression in ["*/15 * * * *", "0 * * * *", "0 1 * * 1-5"] {
        let spec = parse_scheduled_workflow(&format!(
            "### Workflow\nx\n\n### Run Mode\ncron: {expression}\n"
        ))
        .expect("valid definition");
        assert!(
            check_min_interval(&spec.run_mode, &cfg()).is_ok(),
            "{expression} is at or above the 15-minute default"
        );
    }
}

#[test]
fn a_one_shot_definition_has_no_cadence_to_check() {
    let spec = parse_scheduled_workflow("### Workflow\nx\n\n### Run Mode\nonce\n")
        .expect("valid definition");
    assert!(check_min_interval(&spec.run_mode, &cfg()).is_ok());
}

#[test]
fn the_invalid_latch_comments_once_per_transition_not_once_per_sweep() {
    assert_eq!(
        plan_invalid(50, &[], "broken".to_string()),
        vec![ScheduleEffect::FlagInvalid {
            schedule_issue: 50,
            detail: "broken".to_string()
        }]
    );
    assert!(
        plan_invalid(50, &labels(&[SCHEDULE_INVALID_LABEL]), "broken".to_string()).is_empty(),
        "an already-latched definition must not re-comment every 30 seconds"
    );
}
