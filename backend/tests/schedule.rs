//! The scheduled-workflow contract, exercised through the crate's PUBLIC surface:
//! the cadences the milestone needs, the durable run-record wire format the pod
//! side also writes, and the clock's behaviour across a run's whole life.

use fkst_control_plane::goals::scheduled_workflow_parse::{parse_scheduled_workflow, RunMode};
use fkst_control_plane::schedule::{
    collect_records, decide, parse_marker, render_marker, CronExpr, OpenDispatch, RunRecord,
    RunStatus, RunStep, ScheduleAction, ScheduleState, StepStatus,
};
use k8s_openapi::chrono::{DateTime, Duration, TimeZone, Utc};

const DEFINITION: &str = r#"### Workflow
github-candidate-sourcing

### Run Mode
cron: 0 1 * * 1-5

### Arguments
role: AI Tools Application Engineer
min_score: 6
"#;

fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
    Utc.with_ymd_and_hms(year, month, day, hour, minute, 0)
        .single()
        .expect("valid UTC timestamp")
}

/// The first `count` firings strictly after `from`, driven through the public
/// `CronExpr` surface exactly as the schedule pass drives it.
fn firings(expression: &str, from: DateTime<Utc>, count: usize) -> Vec<DateTime<Utc>> {
    let cron = CronExpr::parse(expression).expect("expression parses");
    let mut cursor = from;
    (0..count)
        .map(|_| {
            cursor = cron.next_after(cursor).expect("a firing exists");
            cursor
        })
        .collect()
}

#[test]
fn the_milestone_cadences_are_expressible_through_the_public_surface() {
    // The acceptance workload's cadence: weekdays only, skipping the weekend.
    // 2026-07-31 is a Friday, 08-03 the following Monday.
    assert_eq!(
        firings("0 1 * * 1-5", at(2026, 7, 30, 12, 0), 3),
        vec![
            at(2026, 7, 31, 1, 0),
            at(2026, 8, 3, 1, 0),
            at(2026, 8, 4, 1, 0),
        ]
    );
    // Sub-hourly steps, which the daily-only skeleton could not express at all.
    assert_eq!(
        firings("*/15 * * * *", at(2026, 7, 27, 9, 0), 4),
        vec![
            at(2026, 7, 27, 9, 15),
            at(2026, 7, 27, 9, 30),
            at(2026, 7, 27, 9, 45),
            at(2026, 7, 27, 10, 0),
        ]
    );
    // The day-of-month / day-of-week OR rule, end to end.
    assert_eq!(
        firings("0 0 1 * 1", at(2026, 7, 27, 0, 0), 3),
        vec![
            at(2026, 8, 1, 0, 0),
            at(2026, 8, 3, 0, 0),
            at(2026, 8, 10, 0, 0),
        ]
    );
}

#[test]
fn an_invalid_cadence_names_the_field_the_author_must_fix() {
    let body = DEFINITION.replace("0 1 * * 1-5", "0 1 * * 7");
    let message = parse_scheduled_workflow(&body)
        .expect_err("day-of-week 7 is rejected")
        .to_string();
    assert!(message.contains("day-of-week"), "{message}");
    assert!(message.contains("use 0"), "{message}");
}

#[test]
fn a_weekday_definition_runs_its_whole_life_through_the_public_clock() {
    let spec = parse_scheduled_workflow(DEFINITION).expect("valid definition");
    assert_eq!(spec.workflow_id, "github-candidate-sourcing");
    assert_eq!(spec.arguments["min_score"], "6");
    let RunMode::Cron(_) = &spec.run_mode else {
        panic!("a cron definition");
    };

    let budget = Duration::seconds(3600);
    // Monday 2026-08-03. Anchored on the Friday before, with no history yet.
    let anchor = at(2026, 7, 31, 12, 0);
    let idle = ScheduleState {
        anchor,
        cursor: None,
        running_label: false,
        open_dispatch: None,
        latest_terminal: None,
        paused: false,
    };

    // 1. Nothing before the first weekday slot.
    assert_eq!(
        decide(&spec.run_mode, &idle, at(2026, 8, 3, 0, 30), budget),
        ScheduleAction::Nothing
    );

    // 2. Monday 01:00 comes due — and the weekend slots never existed.
    let slot = at(2026, 8, 3, 1, 0);
    assert_eq!(
        decide(&spec.run_mode, &idle, at(2026, 8, 3, 1, 5), budget),
        ScheduleAction::Dispatch { slot, skipped: 0 }
    );

    // 3. While it runs, Tuesday's slot is recorded as skipped, never queued.
    let running = ScheduleState {
        cursor: Some(slot),
        running_label: true,
        open_dispatch: Some(OpenDispatch {
            slot,
            started: slot,
        }),
        ..idle.clone()
    };
    // A budget wider than the cadence, or the watchdog would fire first and the
    // overlap rule would never be reachable on a daily schedule.
    assert_eq!(
        decide(
            &spec.run_mode,
            &running,
            at(2026, 8, 4, 1, 5),
            Duration::seconds(48 * 3600)
        ),
        ScheduleAction::SkipOverlap {
            slot: at(2026, 8, 4, 1, 0)
        }
    );

    // 4. Past its budget, the watchdog releases it.
    assert_eq!(
        decide(&spec.run_mode, &running, slot + budget, budget),
        ScheduleAction::Expire {
            slot,
            started: slot
        }
    );

    // 5. A terminal record clears the latch instead.
    let completed = ScheduleState {
        open_dispatch: None,
        latest_terminal: Some((slot, RunStatus::Ok)),
        ..running
    };
    assert_eq!(
        decide(&spec.run_mode, &completed, at(2026, 8, 3, 2, 0), budget),
        ScheduleAction::Complete {
            slot,
            status: RunStatus::Ok
        }
    );
}

#[test]
fn a_run_record_survives_the_round_trip_the_pod_side_also_performs() {
    // The pod's workflow runner renders this exact wire format from another
    // repository, so the round trip is the cross-repo contract, not a formality.
    let slot = at(2026, 8, 3, 1, 0);
    let record = RunRecord {
        slot,
        manual: false,
        status: RunStatus::Failed,
        started: slot,
        ended: Some(at(2026, 8, 3, 1, 14)),
        issue: Some(4242),
        detail: Some("step 2 returned no parseable payload".to_string()),
        steps: vec![
            RunStep {
                index: 1,
                id: "scrape".to_string(),
                status: StepStatus::Ok,
                duration_s: Some(41),
            },
            RunStep {
                index: 2,
                id: "score".to_string(),
                status: StepStatus::Failed,
                duration_s: Some(9),
            },
            RunStep {
                index: 3,
                id: "publish".to_string(),
                status: StepStatus::Skipped,
                duration_s: None,
            },
        ],
    };
    let comment = format!("❌ Run failed at step 2.\n\n{}", render_marker(&record));
    assert_eq!(
        parse_marker(&render_marker(&record)).expect("marker"),
        record
    );
    assert_eq!(collect_records(&[comment]), vec![record]);
}
