use fkst_control_plane::schedule::{
    decide, parse_cron_job, parse_marker, render_marker, CronExpr, JobDef, RunRecord, RunStatus,
    ScheduleAction, ScheduleState,
};
use k8s_openapi::chrono::{DateTime, TimeZone, Utc};

const TRIGGER_BODY: &str = r#"### Schedule
cron: 0 3 * * *
timezone: UTC

### Job Type
raise

### Raise Label
fkst-dev

### Raise Title
Daily maintenance

### Raise Body
Run the daily maintenance workflow.
"#;

#[test]
fn due_utc_daily_raise_job_round_trips_its_run_marker() {
    let trigger_created_at = Utc
        .with_ymd_and_hms(2026, 7, 27, 2, 0, 0)
        .single()
        .expect("valid creation timestamp");
    let before_slot = Utc
        .with_ymd_and_hms(2026, 7, 27, 2, 59, 59)
        .single()
        .expect("valid pre-slot timestamp");
    let slot = Utc
        .with_ymd_and_hms(2026, 7, 27, 3, 0, 0)
        .single()
        .expect("valid slot timestamp");

    let spec = parse_cron_job(TRIGGER_BODY).expect("accepted raise trigger");
    assert_eq!(spec.schedule.timezone, "UTC");
    assert_eq!(
        &spec.job,
        &JobDef::Raise {
            label: "fkst-dev".to_string(),
            title: "Daily maintenance".to_string(),
            body: "Run the daily maintenance workflow.".to_string(),
        }
    );
    let state = ScheduleState { trigger_created_at };

    assert_eq!(
        decide(&spec, &state, before_slot).expect("decision"),
        ScheduleAction::Nothing
    );
    assert_eq!(
        decide(&spec, &state, slot).expect("decision"),
        ScheduleAction::ExecuteRaise { slot }
    );

    let record = RunRecord {
        slot,
        manual: false,
        status: RunStatus::Ok,
        started: slot,
        ended: Some(slot),
        issue: Some(3901),
    };
    let marker = render_marker(&record);
    assert_eq!(parse_marker(&marker).expect("rendered marker"), record);
}

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
    let body = TRIGGER_BODY.replace("cron: 0 3 * * *", "cron: 0 3 * * 7");
    let message = parse_cron_job(&body)
        .expect_err("day-of-week 7 is rejected")
        .to_string();
    assert!(message.contains("day-of-week"), "{message}");
    assert!(message.contains("use 0"), "{message}");
}
